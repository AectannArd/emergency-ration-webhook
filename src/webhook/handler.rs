//! Admission webhook HTTP handler and the network-free decision core (T020).
//!
//! Layering (innermost out):
//! - [`decide`] — the pure decision: extract pod requests (FR-005), resolve the
//!   UPDATE delta (FR-004), run [`check_budget`], and return an admit response or
//!   an [`AdmissionError`]. No I/O, exhaustively unit-testable.
//! - [`evaluate`] — reads the cached `Allocation` (+ `ClusterCapacity`) singleton
//!   from the reflector stores, computes the freshness and capacity figures, and
//!   returns a [`DecisionOutcome`] (admit / deny / fail-closed reject). Missing
//!   cache → fail-closed `CapacityDataMissing` (Principle I).
//! - [`handle`] / [`validate`] — the axum `POST /validate` path: deserialise the
//!   `AdmissionReview` body, run [`evaluate`] under a [`with_catch_unwind`] +
//!   [`with_timeout`] guard, classify any foreign error, then emit the structured
//!   log + metrics for the decision. [`healthz`] is the readiness probe and
//!   [`metrics_handler`] exposes Prometheus metrics.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use k8s_openapi::api::core::v1::Pod;
use kube::ResourceExt;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview, Operation};
use kube::runtime::reflector::Store;

use crate::crd::{
    Allocation, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity,
    EnforcementMode, ExemptionReason, check_exemption, resolve_enforcement_mode,
};
use crate::metrics::{CapacityFigures, Metrics, ResourceLabel as MetricResource, VerdictLabel};
use crate::resources::quantity::{self, QuantityParseError};
use crate::time_util;
use crate::webhook::admission::{AdmissionVerdict, Figures, check_budget};
use crate::webhook::error::{AdmissionError, BudgetViolation, MissingCapacityData, ResourceType};

/// A process clock returning Unix seconds. Injected so tests can pin time for
/// the freshness computation; production uses [`time_util::now_unix`].
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Shared state injected into every request handler. The webhook's hot path reads
/// allocation/capacity figures purely from the in-process reflector caches — no
/// network — and records the decision on the shared [`Metrics`] registry.
#[derive(Clone)]
pub struct AppState {
    pub allocation_store: Arc<Store<Allocation>>,
    pub capacity_store: Arc<Store<ClusterCapacity>>,
    pub decision_timeout_ms: u64,
    pub capacity_freshness_timeout_secs: u64,
    pub clock: Clock,
    pub metrics: Arc<Metrics>,
    /// spec-008: the webhook's own namespace (from `--namespace`/`NAMESPACE`),
    /// used for the FR-007 bootstrap self-exemption inside `evaluate()`. Retained
    /// as config (FR-010) — it is NOT deprecated by the CRD-based exclusion.
    pub webhook_namespace: String,
}

impl AppState {
    /// Production state: real clock, default timeouts, empty capacity cache
    /// (filled by the background reflector in `main`).
    pub fn new(
        allocation_store: Arc<Store<Allocation>>,
        capacity_store: Arc<Store<ClusterCapacity>>,
        decision_timeout_ms: u64,
        capacity_freshness_timeout_secs: u64,
        metrics: Arc<Metrics>,
        webhook_namespace: String,
    ) -> Self {
        Self {
            allocation_store,
            capacity_store,
            decision_timeout_ms,
            capacity_freshness_timeout_secs,
            clock: Arc::new(time_util::now_unix),
            metrics,
            webhook_namespace,
        }
    }

    /// State with an injected clock (pinned time) and default timeouts — used by
    /// integration tests to make the freshness computation deterministic.
    pub fn with_clock(
        allocation_store: Arc<Store<Allocation>>,
        capacity_store: Arc<Store<ClusterCapacity>>,
        clock: Clock,
        metrics: Arc<Metrics>,
        webhook_namespace: String,
    ) -> Self {
        Self {
            allocation_store,
            capacity_store,
            decision_timeout_ms: 100,
            capacity_freshness_timeout_secs: 30,
            clock,
            metrics,
            webhook_namespace,
        }
    }
}

/// Build the axum router for the HTTPS admission endpoints (`/validate`,
/// `/metrics`, `/healthz`).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/validate", post(validate))
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz))
        .with_state(state)
}

/// Build the axum router for the plaintext HTTP scrape/probe endpoints
/// (`/metrics`, `/healthz`). Mounted on a separate port so Prometheus can scrape
/// and kubelet can probe without TLS.
pub fn metrics_router(state: AppState) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz))
        .with_state(state)
}

/// Readiness/liveness probe. Always 200 once the process is up.
pub async fn healthz() -> impl IntoResponse {
    "ok"
}

/// `GET /metrics` — Prometheus text exposition (T027).
pub async fn metrics_handler(State(state): State<AppState>) -> String {
    state.metrics.render()
}

/// `POST /validate` — entry point hit by the kube-apiserver.
pub async fn validate(State(state): State<AppState>, body: Bytes) -> Json<serde_json::Value> {
    let response = handle(body, &state).await;
    // Wrap the AdmissionResponse in an AdmissionReview envelope for the apiserver.
    let review = response.into_review();
    Json(serde_json::to_value(&review).expect("admission response is always serialisable"))
}

/// Deserialise the AdmissionReview body, evaluate it against the cached
/// allocation under the fail-safe guards (panic / timeout / unknown), emit the
/// structured log + metrics, and return the verdict response. Pure (no
/// `self`/`State`), so it is exercised directly by integration tests.
pub async fn handle(body: Bytes, state: &AppState) -> AdmissionResponse {
    let now = (state.clock)();
    let start = Instant::now();
    let meta = request_meta(&body);

    // The decision runs inside a panic guard and a per-request timeout. Both
    // map to fail-closed rejections (Principle I): a panic → InternalError, an
    // elapsed timeout → Timeout. run_decision itself only errors on a malformed
    // AdmissionReview (DeserialisationFailure); every other path produces a
    // DecisionOutcome (admit / deny / fail-closed reject) directly.
    let guarded = async {
        // catch_unwind over run_decision; the `?` turns a panic into
        // InternalError. run_decision's own error (DeserialisationFailure) is
        // the inner Result, handled by the caller below.
        with_catch_unwind(AssertUnwindSafe(|| run_decision(&body, state, now)))?
    };
    let result = with_timeout(guarded, state.decision_timeout_ms).await;

    let mut outcome = match result {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => reject_outcome(error, &meta, DecisionVerdict::Error, "enforce"),
        // The outer Err is the timeout or panic (InternalError); classify any
        // other unexpected error through the unknown catch-all (T037).
        Err(error) => reject_outcome(error, &meta, DecisionVerdict::Error, "enforce"),
    };
    outcome.summary.latency_ms = start.elapsed().as_millis() as i64;

    emit_log(&outcome.summary);
    record_metrics(&state.metrics, &outcome.summary);
    outcome.response
}

/// Deserialise the body and evaluate it. Returns `Err(DeserialisationFailure)`
/// when the AdmissionReview cannot be parsed; otherwise a [`DecisionOutcome`]
/// (admit, deny, or a fail-closed reject) carrying the response and the summary
/// used for logging/metrics.
fn run_decision(
    body: &[u8],
    state: &AppState,
    now: i64,
) -> Result<DecisionOutcome, AdmissionError> {
    let review: AdmissionReview<Pod> =
        serde_json::from_slice(body).map_err(|err| AdmissionError::DeserialisationFailure {
            detail: err.to_string(),
        })?;
    let request: AdmissionRequest<Pod> =
        review
            .try_into()
            .map_err(|_| AdmissionError::DeserialisationFailure {
                detail: "request field missing".to_string(),
            })?;
    Ok(evaluate(
        &request,
        &state.allocation_store,
        &state.capacity_store,
        now,
        state.capacity_freshness_timeout_secs,
        &state.webhook_namespace,
    ))
}

/// Read the cached allocation/capacity, compute the capacity figures and
/// freshness, and decide. Fail-closed when the allocation state is not yet
/// populated (Principle I). Infallible: every failure becomes a reject outcome.
///
/// `webhook_namespace` is the webhook's own namespace (spec-008, FR-007) used by
/// the exemption check — checked first inside [`check_exemption`] so the webhook
/// never self-gates once the Allocation is cached.
pub fn evaluate(
    request: &AdmissionRequest<Pod>,
    allocation_store: &Store<Allocation>,
    capacity_store: &Store<ClusterCapacity>,
    now: i64,
    freshness_threshold_secs: u64,
    webhook_namespace: &str,
) -> DecisionOutcome {
    let Some(allocation) =
        allocation_store.find(|allocation| allocation.name_any() == CLUSTER_ALLOCATION_NAME)
    else {
        // No Allocation singleton → no budget and no mode context. Default to
        // "enforce" for the summary (Principle I rejects regardless of mode).
        return reject_outcome(
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
            "enforce",
        );
    };
    let budget_percent = allocation.spec.budget_percent;
    // spec-004: resolve the effective enforcement mode from the cached Allocation
    // spec (None → Enforce, FR-003). The mode is read here, once, and threaded
    // through every subsequent decision so the summary always carries it. The
    // fail-closed paths below return BEFORE check_budget, so they reject in both
    // modes regardless of this value (FR-006 / Principle I) — the mode only
    // governs the budget Deny branch at the end.
    let enforcement = resolve_enforcement_mode(allocation.spec.enforcement_mode);
    let mode_log = enforcement.as_log_str();
    let Some(status) = allocation.status.clone() else {
        return reject_outcome(
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
            mode_log,
        );
    };

    // spec-008: exemption check (data-model §3.1 step 4). Runs AFTER the
    // Allocation singleton + its status are found and BEFORE the freshness check
    // — so the fail-closed paths above (missing allocation, missing status) still
    // reject even for a pod that would otherwise be exempt. An exempt pod is
    // admitted without a freshness check, capacity lookup, or budget arithmetic.
    if let Some(reason) = check_exemption(
        request.namespace.as_deref(),
        pod_priority_class(request),
        &allocation.spec,
        webhook_namespace,
    ) {
        return DecisionOutcome {
            response: AdmissionResponse::from(request),
            summary: DecisionSummary::exempt(request, reason, enforcement),
        };
    }

    let freshness = assess_freshness(&status.last_updated, now, freshness_threshold_secs);

    // T032: enforce the freshness threshold. Data older than the threshold (or
    // with an unparseable timestamp) cannot be authoritatively verified → deny
    // (Principle I). This path rejects in both modes (FR-006).
    if freshness.stale {
        return reject_outcome(
            AdmissionError::CapacityDataStale {
                age_secs: freshness.age_secs,
                threshold_secs: freshness_threshold_secs,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
            mode_log,
        );
    }

    // The supply-side cache must also be present; a missing ClusterCapacity
    // means the supply state is not initialised → deny (Error Path Matrix).
    // Rejects in both modes (FR-006).
    let Some(capacity_status) = capacity_store
        .find(|c| c.name_any() == CLUSTER_CAPACITY_NAME)
        .and_then(|c| c.status.clone())
    else {
        return reject_outcome(
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::ClusterCapacity,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
            mode_log,
        );
    };
    let total_cpu = capacity_status.total_allocatable_cpu_milli;
    let total_mem = capacity_status.total_allocatable_memory_bytes;

    // Resolve the effective request (figures). A quantity parse failure is
    // fail-closed (T034) — rejects in both modes (FR-006).
    let effective = match effective_request(request) {
        Ok(effective) => effective,
        Err(error) => {
            return reject_outcome(
                error,
                &request_meta_of(request),
                DecisionVerdict::Error,
                mode_log,
            )
            .with_freshness(freshness.seconds)
            .with_budget(budget_percent)
            .with_totals(total_cpu, total_mem);
        }
    };

    let allocated: Figures = (status.allocated_cpu_milli, status.allocated_memory_bytes);
    let ceilings: Figures = (status.ceiling_cpu_milli, status.ceiling_memory_bytes);

    match check_budget(allocated, effective, ceilings) {
        AdmissionVerdict::Admit => DecisionOutcome {
            response: AdmissionResponse::from(request),
            summary: DecisionSummary::decision(
                request,
                budget_percent,
                freshness.seconds,
                enforcement,
                ResourceFigures::within(
                    ResourceType::Cpu,
                    allocated,
                    effective,
                    ceilings,
                    total_cpu,
                ),
                ResourceFigures::within(
                    ResourceType::Memory,
                    allocated,
                    effective,
                    ceilings,
                    total_mem,
                ),
            )
            .verdict(DecisionVerdict::Allow),
        },
        AdmissionVerdict::Deny(violations) => {
            // The summary carries the same capacity figures a real deny would,
            // regardless of mode; only the response and verdict differ.
            let summary = DecisionSummary::decision(
                request,
                budget_percent,
                freshness.seconds,
                enforcement,
                ResourceFigures::within(
                    ResourceType::Cpu,
                    allocated,
                    effective,
                    ceilings,
                    total_cpu,
                )
                .mark_over(&violations),
                ResourceFigures::within(
                    ResourceType::Memory,
                    allocated,
                    effective,
                    ceilings,
                    total_mem,
                )
                .mark_over(&violations),
            );
            // spec-004: at the budget Deny branch (the ONLY insertion point for
            // the mode toggle), dry-run converts the rejection into an admit
            // carrying the would-be rejection as a warning. Enforce keeps the
            // existing fail-closed rejection. Because every fail-closed path
            // returns above before reaching check_budget, no error rejection can
            // be converted (FR-006 / Principle I).
            match enforcement {
                EnforcementMode::DryRun => {
                    let mut response = AdmissionResponse::from(request);
                    response.warnings = Some(vec![dry_run_warning(&violations)]);
                    DecisionOutcome {
                        response,
                        summary: summary.verdict(DecisionVerdict::DryRunDeny),
                    }
                }
                EnforcementMode::Enforce => {
                    let response = AdmissionError::OverBudget {
                        violations: violations.clone(),
                    }
                    .into_response(&request.uid);
                    DecisionOutcome {
                        response,
                        summary: summary.verdict(DecisionVerdict::Deny),
                    }
                }
            }
        }
    }
}

/// Build the dry-run admission warning from the budget violations: the same
/// per-resource message a real rejection carries (data-model.md §7), prefixed
/// with `"Budget violations (dry-run): "`. When both resources are over budget
/// both lines are joined with `\n` inside a single warning string.
fn dry_run_warning(violations: &[BudgetViolation]) -> String {
    let body = violations
        .iter()
        .map(BudgetViolation::message_line)
        .collect::<Vec<_>>()
        .join("\n");
    format!("Budget violations (dry-run): {body}")
}

/// Apply the per-request timeout. Elapsed → [`AdmissionError::Timeout`].
/// Exposed so the timeout path is unit-testable with a deliberately slow future
/// (the real decision is sub-millisecond, so it cannot naturally exceed the
/// deadline — the guard still wraps the decision in production).
pub async fn with_timeout<F>(future: F, timeout_ms: u64) -> Result<F::Output, AdmissionError>
where
    F: std::future::Future,
{
    match tokio::time::timeout(Duration::from_millis(timeout_ms), future).await {
        Ok(value) => Ok(value),
        Err(_) => Err(AdmissionError::Timeout { timeout_ms }),
    }
}

/// Guard a fallible computation against panics. A caught panic →
/// [`AdmissionError::InternalError`] (T036).
pub fn with_catch_unwind<F, R>(f: F) -> Result<R, AdmissionError>
where
    F: FnOnce() -> R + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(value) => Ok(value),
        Err(_) => Err(AdmissionError::InternalError),
    }
}

/// The unknown-error catch-all (T037): an error already typed as
/// [`AdmissionError`] is preserved; any foreign error becomes
/// [`AdmissionError::Unknown`]. Guarantees Principle III's "no third category".
pub fn classify_error(error: Box<dyn std::error::Error>) -> AdmissionError {
    match error.downcast::<AdmissionError>() {
        Ok(known) => *known,
        Err(other) => AdmissionError::Unknown {
            detail: other.to_string(),
        },
    }
}

/// Capacity-data freshness assessment (T032). Returns the seconds since the
/// Allocation status was last refreshed (for logging + the freshness gauge) and
/// whether the data is stale (age strictly greater than the threshold). An
/// unparseable `lastUpdated` is treated as stale (Principle I: fail-closed — we
/// cannot authoritatively verify freshness).
pub fn assess_freshness(last_updated: &str, now: i64, threshold_secs: u64) -> Freshness {
    match time_util::parse_rfc3339(last_updated) {
        None => Freshness {
            seconds: -1,
            age_secs: 0,
            stale: true,
        },
        Some(refreshed) => {
            // A future timestamp clamps to age 0 (fresh); only strictly older
            // than the threshold is stale.
            let age = now.saturating_sub(refreshed).max(0) as u64;
            Freshness {
                seconds: age as i64,
                age_secs: age,
                stale: age > threshold_secs,
            }
        }
    }
}

/// Result of [`assess_freshness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Freshness {
    /// Seconds since the last refresh (≥ 0), or -1 when unparseable.
    pub seconds: i64,
    /// Seconds since the last refresh, for the stale error message.
    pub age_secs: u64,
    /// Whether the data is older than the freshness threshold.
    pub stale: bool,
}

// ---------------------------------------------------------------------------
// Decision outcome + summary (logging/metrics payload)
// ---------------------------------------------------------------------------

/// A decision-level verdict (coarser than the per-resource metric verdict).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionVerdict {
    Allow,
    Deny,
    /// spec-004: a dry-run mode decision that would have denied, but instead
    /// admitted the pod with a warning. Fail-closed paths never produce this —
    /// the conversion happens only at the `check_budget` Deny branch.
    DryRunDeny,
    /// spec-008: admitted by exclusion policy with no budget check. Produced
    /// only by the exemption branch in `evaluate()`, after the Allocation is
    /// found. Carries the triggering [`ExemptionReason`] on the summary.
    Exempt,
    Error,
}

/// Per-resource capacity figures used by the structured log and the gauges.
#[derive(Debug, Clone, Copy)]
pub struct ResourceFigures {
    pub resource: ResourceType,
    pub allocated: i64,
    pub requested: i64,
    pub projected: i64,
    pub ceiling: i64,
    pub total_allocatable: i64,
    /// Whether this resource's projected allocation exceeded its ceiling.
    pub over: bool,
}

impl Default for ResourceFigures {
    fn default() -> Self {
        Self {
            resource: ResourceType::Cpu,
            allocated: 0,
            requested: 0,
            projected: 0,
            ceiling: 0,
            total_allocatable: 0,
            over: false,
        }
    }
}

impl ResourceFigures {
    /// Build the figures for a resource from the decision inputs (CPU → tuple
    /// index 0, memory → 1).
    fn within(
        resource: ResourceType,
        allocated: Figures,
        requested: Figures,
        ceiling: Figures,
        total_allocatable: i64,
    ) -> Self {
        let (allocated, requested, ceiling) = match resource {
            ResourceType::Cpu => (allocated.0, requested.0, ceiling.0),
            ResourceType::Memory => (allocated.1, requested.1, ceiling.1),
        };
        Self {
            resource,
            allocated,
            requested,
            projected: allocated.saturating_add(requested),
            ceiling,
            total_allocatable,
            over: false,
        }
    }

    /// Mark `over` when a budget violation names this resource.
    fn mark_over(mut self, violations: &[BudgetViolation]) -> Self {
        self.over = violations.iter().any(|v| v.resource == self.resource);
        self
    }
}

/// Everything the log + metrics need for one admission decision.
#[derive(Debug, Clone)]
pub struct DecisionSummary {
    pub workload: String,
    pub operation: String,
    pub verdict: DecisionVerdict,
    pub reason: String,
    pub budget_percent: i64,
    pub freshness_seconds: i64,
    pub latency_ms: i64,
    /// spec-004: the active enforcement mode for this decision
    /// (`"enforce"` / `"dry_run"`). Present on every decision (FR-009).
    pub enforcement_mode: String,
    /// spec-008: the criterion that triggered an exemption (`Some` iff the
    /// verdict is [`DecisionVerdict::Exempt`]). Drives the exemption counter's
    /// `reason` label and the log's `exemption_reason` field.
    pub exemption_reason: Option<ExemptionReason>,
    pub cpu: ResourceFigures,
    pub memory: ResourceFigures,
}

impl DecisionSummary {
    fn decision(
        request: &AdmissionRequest<Pod>,
        budget_percent: i32,
        freshness_seconds: i64,
        enforcement_mode: EnforcementMode,
        cpu: ResourceFigures,
        memory: ResourceFigures,
    ) -> Self {
        Self {
            workload: workload_of(request),
            operation: operation_of(&request.operation).to_string(),
            verdict: DecisionVerdict::Allow,
            reason: String::new(),
            budget_percent: budget_percent as i64,
            freshness_seconds,
            latency_ms: 0,
            enforcement_mode: enforcement_mode.as_log_str().to_string(),
            exemption_reason: None,
            cpu,
            memory,
        }
    }

    /// spec-008: build the summary for an exempt decision. The pod is admitted
    /// by exclusion policy with no budget check, so there are no resource
    /// figures, no freshness assessment, and no budget-percent context
    /// (carried as -1, matching [`reject_outcome`]).
    fn exempt(
        request: &AdmissionRequest<Pod>,
        reason: ExemptionReason,
        enforcement_mode: EnforcementMode,
    ) -> Self {
        Self {
            workload: workload_of(request),
            operation: operation_of(&request.operation).to_string(),
            verdict: DecisionVerdict::Exempt,
            reason: String::new(),
            budget_percent: -1,
            freshness_seconds: -1,
            latency_ms: 0,
            enforcement_mode: enforcement_mode.as_log_str().to_string(),
            exemption_reason: Some(reason),
            cpu: ResourceFigures::default(),
            memory: ResourceFigures {
                resource: ResourceType::Memory,
                ..ResourceFigures::default()
            },
        }
    }

    fn verdict(mut self, verdict: DecisionVerdict) -> Self {
        self.verdict = verdict;
        self
    }
}

/// A decision result with its fail-safe response and the observability summary.
pub struct DecisionOutcome {
    pub response: AdmissionResponse,
    pub summary: DecisionSummary,
}

impl DecisionOutcome {
    /// Mutate the summary (builder helpers used on the early-return paths).
    fn with_freshness(mut self, seconds: i64) -> Self {
        self.summary.freshness_seconds = seconds;
        self
    }
    fn with_budget(mut self, budget_percent: i32) -> Self {
        self.summary.budget_percent = budget_percent as i64;
        self
    }
    fn with_totals(mut self, cpu: i64, memory: i64) -> Self {
        self.summary.cpu.total_allocatable = cpu;
        self.summary.memory.total_allocatable = memory;
        self
    }
}

/// Build a fail-closed reject outcome from an [`AdmissionError`] (admit path
/// never reaches here). `enforcement_mode` is the mode label for the summary
/// (FR-009); callers that have an Allocation pass the resolved mode, the
/// request-layer guards (`handle`) pass `"enforce"` — there is no Allocation
/// context on a deserialisation/timeout/panic path.
fn reject_outcome(
    error: AdmissionError,
    meta: &RequestMeta,
    verdict: DecisionVerdict,
    enforcement_mode: &str,
) -> DecisionOutcome {
    let reason = error.slug().to_string();
    let response = error.into_response(&meta.uid);
    DecisionOutcome {
        response,
        summary: DecisionSummary {
            workload: meta.workload(),
            operation: meta.operation.clone(),
            verdict,
            reason,
            budget_percent: -1,
            freshness_seconds: -1,
            latency_ms: 0,
            enforcement_mode: enforcement_mode.to_string(),
            exemption_reason: None,
            cpu: ResourceFigures::default(),
            memory: ResourceFigures::default(),
        },
    }
}

/// Emit the structured tracing event(s) for a decision (T026).
///
/// - Admit → one INFO event per resource, with every Logging Contract field.
/// - Deny / DryRunDeny → one WARN event per resource (reason names the violated
///   resource). A dry-run deny carries `decision = "dry_run_deny"` so it is
///   distinguishable from an enforced deny in log aggregators (spec-004, FR-008).
/// - Error → one ERROR event with the reason/error.
///
/// spec-004 (FR-009): every variant carries the active `enforcement_mode`.
fn emit_log(summary: &DecisionSummary) {
    match summary.verdict {
        DecisionVerdict::Error => {
            tracing::error!(
                target: "capacity_admission",
                workload = %summary.workload,
                operation = %summary.operation,
                decision = "error",
                enforcement_mode = %summary.enforcement_mode,
                reason = %summary.reason,
                error = %summary.reason,
                budget_percent = summary.budget_percent,
                freshness_seconds = summary.freshness_seconds,
                latency_ms = summary.latency_ms,
                "admission rejected"
            );
        }
        DecisionVerdict::Exempt => {
            // spec-008: admitted by exclusion policy, no budget check. Single
            // INFO event carrying the triggering reason (FR-008 / Principle IV).
            let reason = summary
                .exemption_reason
                .map(ExemptionReason::as_str)
                .unwrap_or("");
            tracing::info!(
                target: "capacity_admission",
                workload = %summary.workload,
                operation = %summary.operation,
                decision = "exempt",
                enforcement_mode = %summary.enforcement_mode,
                exemption_reason = reason,
                latency_ms = summary.latency_ms,
                "admission allowed by exclusion policy"
            );
        }
        DecisionVerdict::Allow => {
            for (rtype, figures) in [("cpu", &summary.cpu), ("memory", &summary.memory)] {
                tracing::info!(
                    target: "capacity_admission",
                    workload = %summary.workload,
                    operation = %summary.operation,
                    decision = "allow",
                    enforcement_mode = %summary.enforcement_mode,
                    resource_type = rtype,
                    allocated = figures.allocated,
                    requested = figures.requested,
                    projected = figures.projected,
                    ceiling = figures.ceiling,
                    budget_percent = summary.budget_percent,
                    freshness_seconds = summary.freshness_seconds,
                    latency_ms = summary.latency_ms,
                    "admission allowed"
                );
            }
        }
        DecisionVerdict::Deny | DecisionVerdict::DryRunDeny => {
            // Dry-run deny logs at WARN (same as an enforced deny) but with the
            // distinct `dry_run_deny` decision label (FR-008).
            let decision_label: &'static str = match summary.verdict {
                DecisionVerdict::DryRunDeny => "dry_run_deny",
                _ => "deny",
            };
            for (rtype, figures) in [("cpu", &summary.cpu), ("memory", &summary.memory)] {
                let reason = if figures.over {
                    format!("{rtype}_over_budget")
                } else {
                    String::new()
                };
                tracing::warn!(
                    target: "capacity_admission",
                    workload = %summary.workload,
                    operation = %summary.operation,
                    decision = decision_label,
                    enforcement_mode = %summary.enforcement_mode,
                    resource_type = rtype,
                    reason = %reason,
                    allocated = figures.allocated,
                    requested = figures.requested,
                    projected = figures.projected,
                    ceiling = figures.ceiling,
                    budget_percent = summary.budget_percent,
                    freshness_seconds = summary.freshness_seconds,
                    latency_ms = summary.latency_ms,
                    "admission denied"
                );
            }
        }
    }
}

/// Record the verdict counters, decision latency, freshness, and capacity
/// gauges from the decision summary (T027/T029). The capacity gauges are only
/// refreshed when the decision had real figures (admit/deny), so they match the
/// state used by the most recent decision (SC-003).
fn record_metrics(metrics: &Metrics, summary: &DecisionSummary) {
    if summary.verdict == DecisionVerdict::Exempt {
        // spec-008: an exempt decision bypasses the budget. Bump the exemption
        // counter (NOT the verdict counter) and skip the capacity gauges — no
        // figures were computed (data-model §4.2). Latency is still observed
        // (every decision, regardless of outcome).
        if let Some(reason) = summary.exemption_reason {
            metrics.record_exemption(reason.as_str());
        }
        metrics.observe_duration(summary.latency_ms as f64 / 1000.0);
        return;
    }
    for (resource, figures) in [
        (MetricResource::Cpu, &summary.cpu),
        (MetricResource::Memory, &summary.memory),
    ] {
        let verdict = match summary.verdict {
            DecisionVerdict::Error => VerdictLabel::Error,
            // spec-004: a dry-run would-be-deny records its own label so the
            // verdict counter distinguishes shadow admits from enforced denies.
            DecisionVerdict::DryRunDeny => VerdictLabel::DryRunDeny,
            _ if figures.over => VerdictLabel::Deny,
            _ => VerdictLabel::Allow,
        };
        metrics.record_verdict(resource, verdict);
    }
    metrics.observe_duration(summary.latency_ms as f64 / 1000.0);
    if summary.freshness_seconds >= 0 {
        metrics.set_freshness(summary.freshness_seconds);
    }
    if summary.verdict != DecisionVerdict::Error {
        metrics.refresh_capacity(
            to_capacity_figures(&summary.cpu),
            to_capacity_figures(&summary.memory),
        );
    }
}

fn to_capacity_figures(figures: &ResourceFigures) -> CapacityFigures {
    CapacityFigures {
        allocated: figures.allocated,
        ceiling: figures.ceiling,
        total_allocatable: figures.total_allocatable,
        ratio: ratio(figures.allocated, figures.ceiling),
    }
}

fn ratio(allocated: i64, ceiling: i64) -> f64 {
    if ceiling == 0 {
        0.0
    } else {
        allocated as f64 / ceiling as f64
    }
}

/// Refresh the capacity gauges + freshness gauge straight from the cached CRD
/// status (T029). Called periodically by a background task so the metrics stay
/// current even without admission traffic, complementing the per-decision
/// refresh in [`record_metrics`] (SC-003).
pub fn refresh_gauges(
    metrics: &Metrics,
    allocation_store: &Store<Allocation>,
    capacity_store: &Store<ClusterCapacity>,
    now: i64,
) {
    let Some(status) = allocation_store
        .find(|a| a.name_any() == CLUSTER_ALLOCATION_NAME)
        .and_then(|a| a.status.clone())
    else {
        return;
    };
    let (total_cpu, total_mem) = capacity_store
        .find(|c| c.name_any() == CLUSTER_CAPACITY_NAME)
        .and_then(|c| c.status.clone())
        .map(|s| {
            (
                s.total_allocatable_cpu_milli,
                s.total_allocatable_memory_bytes,
            )
        })
        .unwrap_or((0, 0));

    let freshness = assess_freshness(&status.last_updated, now, 0);
    metrics.set_freshness(freshness.seconds.max(0));
    metrics.refresh_capacity(
        CapacityFigures {
            allocated: status.allocated_cpu_milli,
            ceiling: status.ceiling_cpu_milli,
            total_allocatable: total_cpu,
            ratio: ratio(status.allocated_cpu_milli, status.ceiling_cpu_milli),
        },
        CapacityFigures {
            allocated: status.allocated_memory_bytes,
            ceiling: status.ceiling_memory_bytes,
            total_allocatable: total_mem,
            ratio: ratio(status.allocated_memory_bytes, status.ceiling_memory_bytes),
        },
    );
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Pure decision core (data-model.md §3–4). Extracts the pod's effective request,
/// resolves an UPDATE as a delta against the old object, and checks the budget.
pub fn decide(
    request: &AdmissionRequest<Pod>,
    status: &AllocationStatus,
) -> Result<AdmissionResponse, AdmissionError> {
    let new_request = pod_request(request.object.as_ref())?;
    let effective = match request.operation {
        Operation::Update => {
            let old_request = pod_request(request.old_object.as_ref())?;
            (new_request.0 - old_request.0, new_request.1 - old_request.1)
        }
        _ => new_request,
    };

    let allocated = (status.allocated_cpu_milli, status.allocated_memory_bytes);
    let ceilings = (status.ceiling_cpu_milli, status.ceiling_memory_bytes);

    match check_budget(allocated, effective, ceilings) {
        AdmissionVerdict::Admit => Ok(AdmissionResponse::from(request)),
        AdmissionVerdict::Deny(violations) => Err(AdmissionError::OverBudget { violations }),
    }
}

/// Resolve the effective (cpu_milli, memory_bytes) request for a pod, applying
/// the UPDATE delta. Exposed for the figure computation in [`evaluate`].
fn effective_request(request: &AdmissionRequest<Pod>) -> Result<Figures, AdmissionError> {
    let new_request = pod_request(request.object.as_ref())?;
    match request.operation {
        Operation::Update => {
            let old_request = pod_request(request.old_object.as_ref())?;
            Ok((new_request.0 - old_request.0, new_request.1 - old_request.1))
        }
        _ => Ok(new_request),
    }
}

/// Extract the (cpu_milli, memory_bytes) request of a pod, applying the
/// Kubernetes defaulting convention (T009). A missing object contributes 0.
fn pod_request(pod: Option<&Pod>) -> Result<Figures, AdmissionError> {
    let Some(spec) = pod.and_then(|p| p.spec.as_ref()) else {
        return Ok((0, 0));
    };
    quantity::extract_pod_requests(spec).map_err(|QuantityParseError::Invalid { input, .. }| {
        AdmissionError::QuantityParseFailure {
            field: "resources.requests".to_string(),
            value: input,
        }
    })
}

/// spec-008: extract the pod's `priorityClassName` for the exemption check
/// (string match only — no PriorityClass resource resolution, R3). Absent on a
/// missing object/spec.
fn pod_priority_class(request: &AdmissionRequest<Pod>) -> Option<&str> {
    request
        .object
        .as_ref()
        .and_then(|pod| pod.spec.as_ref())
        .and_then(|spec| spec.priority_class_name.as_deref())
}

/// `namespace/name` for the triggering workload.
fn workload_of(request: &AdmissionRequest<Pod>) -> String {
    format!(
        "{}/{}",
        request.namespace.as_deref().unwrap_or(""),
        request.name
    )
}

/// Uppercase operation label for logging.
fn operation_of(operation: &Operation) -> &'static str {
    match operation {
        Operation::Create => "CREATE",
        Operation::Update => "UPDATE",
        Operation::Delete => "DELETE",
        Operation::Connect => "CONNECT",
    }
}

/// Best-effort request metadata parsed straight from the raw body, used to echo
/// `uid`/`workload`/`operation` on the fail-safe error paths (deserialisation,
/// timeout, panic) where no typed request is available.
#[derive(Debug, Clone, Default)]
struct RequestMeta {
    uid: String,
    namespace: String,
    name: String,
    operation: String,
}

impl RequestMeta {
    fn workload(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

fn request_meta(body: &[u8]) -> RequestMeta {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    request_meta_from_value(&value)
}

fn request_meta_of(request: &AdmissionRequest<Pod>) -> RequestMeta {
    RequestMeta {
        uid: request.uid.clone(),
        namespace: request.namespace.clone().unwrap_or_default(),
        name: request.name.clone(),
        operation: operation_of(&request.operation).to_string(),
    }
}

fn request_meta_from_value(value: &serde_json::Value) -> RequestMeta {
    let request = value.get("request").unwrap_or(&serde_json::Value::Null);
    let str_field = |key: &str| {
        request
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    RequestMeta {
        uid: str_field("uid"),
        namespace: str_field("namespace"),
        name: str_field("name"),
        operation: str_field("operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        Allocation, AllocationSpec, AllocationStatus, EnforcementMode, ExemptionReason,
    };
    use crate::time_util::parse_rfc3339;
    use k8s_openapi::api::core::v1::{Container, PodSpec, ResourceRequirements};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use kube::runtime::reflector::Store;
    use kube::runtime::watcher;
    use std::collections::BTreeMap;

    /// Build a pod with a single container requesting `cpu` / `memory`.
    fn pod(cpu: &str, memory: &str) -> Pod {
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
        requests.insert("memory".to_string(), Quantity(memory.to_string()));
        let spec = PodSpec {
            containers: vec![Container {
                resources: Some(ResourceRequirements {
                    requests: Some(requests),
                    limits: None,
                    claims: None,
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        Pod {
            spec: Some(spec),
            ..Default::default()
        }
    }

    /// Build an `AdmissionRequest<Pod>` by round-tripping a real AdmissionReview
    /// JSON document — the same path the apiserver body takes.
    fn request(obj: &Pod, op: Operation, old: Option<&Pod>) -> AdmissionRequest<Pod> {
        let object = serde_json::to_value(obj).unwrap();
        let old_object = match old {
            Some(o) => serde_json::to_value(o).unwrap(),
            None => serde_json::Value::Null,
        };
        let op_str = operation_of(&op);
        let review = serde_json::json!({
            "kind": "AdmissionReview",
            "apiVersion": "admission.k8s.io/v1",
            "request": {
                "uid": "uid-1",
                "name": "p",
                "namespace": "default",
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "resource": {"group": "", "version": "v1", "resource": "pods"},
                "operation": op_str,
                "userInfo": {"username": "test"},
                "object": object,
                "oldObject": old_object,
                "dryRun": false,
            }
        });
        let parsed: AdmissionReview<Pod> = serde_json::from_value(review).unwrap();
        parsed.try_into().unwrap()
    }

    /// Allocation status: allocated 70 CPU / 110Gi, ceiling 80 CPU / 160Gi.
    fn status() -> AllocationStatus {
        AllocationStatus {
            allocated_cpu_milli: 70_000,
            allocated_memory_bytes: 110 * 1024,
            ceiling_cpu_milli: 80_000,
            ceiling_memory_bytes: 160 * 1024,
            utilization_percent_cpu: 0.0,
            utilization_percent_memory: 0.0,
            last_updated: "2026-07-26T00:00:00Z".to_string(),
        }
    }

    fn allocation_with(status: AllocationStatus) -> Allocation {
        let mut a = Allocation::new(
            CLUSTER_ALLOCATION_NAME,
            AllocationSpec {
                budget_percent: 80,
                enforcement_mode: None,
                excluded_namespaces: None,
                excluded_priority_classes: None,
            },
        );
        a.status = Some(status);
        a
    }

    /// A pinned "now" equal to the fixture `last_updated` → freshness 0 (fresh).
    fn now() -> i64 {
        parse_rfc3339("2026-07-26T00:00:00Z").unwrap()
    }

    fn empty_capacity_store() -> Store<ClusterCapacity> {
        kube::runtime::reflector::store::<ClusterCapacity>().0
    }

    /// A capacity store with 100 CPU / 200 GiB present (freshness matches the
    /// fixture clock), so the admit/deny paths reach the budget check.
    fn populated_capacity_store() -> Store<ClusterCapacity> {
        use crate::crd::{ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus};
        let (store, mut writer) = kube::runtime::reflector::store::<ClusterCapacity>();
        let mut c = ClusterCapacity::new(
            CLUSTER_CAPACITY_NAME,
            ClusterCapacitySpec {
                node_selectors: None,
            },
        );
        c.status = Some(ClusterCapacityStatus {
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 200 * 1024 * 1024 * 1024,
            node_count: 2,
            last_updated: "2026-07-26T00:00:00Z".to_string(),
            excluded_node_count: 0,
            excluded_by_unschedulable: 0,
            excluded_by_selector: 0,
        });
        writer.apply_watcher_event(&kube::runtime::watcher::Event::Apply(c));
        store
    }

    fn populated_store() -> Store<Allocation> {
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        writer.apply_watcher_event(&watcher::Event::Apply(allocation_with(status())));
        store
    }

    /// A populated Allocation store with the singleton in `mode` (spec-004).
    fn populated_store_with_mode(mode: EnforcementMode) -> Store<Allocation> {
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        let mut allocation = allocation_with(status());
        allocation.spec.enforcement_mode = Some(mode);
        writer.apply_watcher_event(&watcher::Event::Apply(allocation));
        store
    }

    // ---- spec-008 exemption test fixtures ----

    /// The webhook's own namespace used in the exemption tests (FR-007). Pods in
    /// `"default"` (the existing fixtures) are NOT exempt, so those tests are
    /// unaffected by the new check.
    const WEBHOOK_NS: &str = "capacity-admission";

    /// Build an `AdmissionRequest<Pod>` in `namespace` (mirrors [`request`]).
    fn request_in(
        namespace: &str,
        obj: &Pod,
        op: Operation,
        old: Option<&Pod>,
    ) -> AdmissionRequest<Pod> {
        let object = serde_json::to_value(obj).unwrap();
        let old_object = match old {
            Some(o) => serde_json::to_value(o).unwrap(),
            None => serde_json::Value::Null,
        };
        let op_str = operation_of(&op);
        let review = serde_json::json!({
            "kind": "AdmissionReview",
            "apiVersion": "admission.k8s.io/v1",
            "request": {
                "uid": "uid-1",
                "name": "p",
                "namespace": namespace,
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "resource": {"group": "", "version": "v1", "resource": "pods"},
                "operation": op_str,
                "userInfo": {"username": "test"},
                "object": object,
                "oldObject": old_object,
                "dryRun": false,
            }
        });
        let parsed: AdmissionReview<Pod> = serde_json::from_value(review).unwrap();
        parsed.try_into().unwrap()
    }

    /// Build a pod with a single container and an optional `priorityClassName`.
    fn pod_with_priority(cpu: &str, memory: &str, priority_class: Option<&str>) -> Pod {
        let mut pod = pod(cpu, memory);
        if let Some(pc) = priority_class
            && let Some(spec) = pod.spec.as_mut()
        {
            spec.priority_class_name = Some(pc.to_string());
        }
        pod
    }

    /// A populated Allocation store whose singleton carries exclusion lists.
    fn populated_store_excluded(
        excluded_namespaces: Option<Vec<&str>>,
        excluded_priority_classes: Option<Vec<&str>>,
    ) -> Store<Allocation> {
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        let mut allocation = allocation_with(status());
        allocation.spec.excluded_namespaces =
            excluded_namespaces.map(|v| v.into_iter().map(String::from).collect());
        allocation.spec.excluded_priority_classes =
            excluded_priority_classes.map(|v| v.into_iter().map(String::from).collect());
        writer.apply_watcher_event(&watcher::Event::Apply(allocation));
        store
    }

    // ---- decide: CREATE admit/deny (spec scenarios 1, 2, 4) ----

    #[test]
    fn create_under_ceiling_is_admitted() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let resp = decide(&req, &status()).unwrap();
        assert!(resp.allowed);
        assert_eq!(resp.uid, "uid-1");
    }

    #[test]
    fn create_over_ceiling_is_denied_with_figures() {
        let req = request(&pod("15", "1"), Operation::Create, None);
        let err = decide(&req, &status()).unwrap_err();
        let AdmissionError::OverBudget { violations } = err else {
            panic!("expected OverBudget, got {err:?}");
        };
        let cpu = &violations[0];
        assert_eq!(cpu.allocated, 70_000);
        assert_eq!(cpu.requested, 15_000);
        assert_eq!(cpu.projected, 85_000);
        assert_eq!(cpu.ceiling, 80_000);
    }

    #[test]
    fn create_zero_request_is_admitted() {
        let bare = Pod {
            spec: Some(PodSpec {
                containers: vec![Container::default()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let req = request(&bare, Operation::Create, None);
        assert!(decide(&req, &status()).unwrap().allowed);
    }

    // ---- decide: UPDATE evaluated as delta (spec scenario 5) ----

    #[test]
    fn update_evaluates_delta_not_full_request() {
        let old = pod("10", "1");
        let new = pod("20", "1");
        let req = request(&new, Operation::Update, Some(&old));
        assert!(
            decide(&req, &status()).unwrap().allowed,
            "delta of +10 lands exactly at the 80 ceiling (inclusive)"
        );
    }

    #[test]
    fn update_delta_over_ceiling_is_denied() {
        let old = pod("10", "1");
        let new = pod("30", "1");
        let req = request(&new, Operation::Update, Some(&old));
        assert!(decide(&req, &status()).is_err());
    }

    // ---- evaluate: missing allocation is fail-closed ----

    #[test]
    fn evaluate_rejects_when_allocation_missing() {
        let req = request(&pod("1", "1"), Operation::Create, None);
        let (store, _writer) = kube::runtime::reflector::store::<Allocation>();
        let capacity = empty_capacity_store();
        let outcome = evaluate(&req, &store, &capacity, now(), 30, WEBHOOK_NS);
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
    }

    #[test]
    fn evaluate_admits_when_allocation_present() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store(),
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Allow);
        assert_eq!(outcome.summary.budget_percent, 80);
        // Freshness field is computed for logging even before enforcement.
        assert_eq!(outcome.summary.freshness_seconds, 0);
    }

    #[test]
    fn evaluate_deny_carries_figures_in_summary() {
        let req = request(&pod("15", "1"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store(),
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Deny);
        assert_eq!(outcome.summary.cpu.allocated, 70_000);
        assert_eq!(outcome.summary.cpu.requested, 15_000);
        assert_eq!(outcome.summary.cpu.projected, 85_000);
        assert_eq!(outcome.summary.cpu.ceiling, 80_000);
        assert!(outcome.summary.cpu.over);
        assert!(!outcome.summary.memory.over);
    }

    // ---- spec-004: dry-run shadow evaluation (US1) ----

    #[test]
    fn evaluate_dry_run_admits_over_budget_with_warning() {
        // T010: dry-run converts an over-budget deny to an admit carrying the
        // would-be rejection as a warning (allowed == true).
        let req = request(&pod("15", "1"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store_with_mode(EnforcementMode::DryRun),
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            outcome.response.allowed,
            "dry-run mode admits over-budget pods"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::DryRunDeny);
        assert_eq!(
            outcome.summary.enforcement_mode, "dry_run",
            "summary carries the active enforcement mode"
        );
        // The same figures a real deny would carry.
        assert!(outcome.summary.cpu.over);
        assert_eq!(outcome.summary.cpu.projected, 85_000);
        let warnings = outcome
            .response
            .warnings
            .as_ref()
            .expect("dry-run admit carries a warning");
        assert_eq!(warnings.len(), 1, "one warning string");
        assert!(
            warnings[0].starts_with("Budget violations (dry-run): "),
            "{warnings:?}"
        );
        assert!(
            warnings[0].contains("CPU budget exceeded"),
            "warning reuses the rejection message: {warnings:?}"
        );
        assert!(warnings[0].contains("projected 85000m"), "{warnings:?}");
    }

    #[test]
    fn evaluate_enforce_rejects_over_budget_unchanged() {
        // T011: enforce mode rejects an over-budget pod (existing behaviour).
        let req = request(&pod("15", "1"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store_with_mode(EnforcementMode::Enforce),
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "enforce mode rejects over-budget"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Deny);
        assert_eq!(outcome.summary.enforcement_mode, "enforce");
        assert!(
            outcome.response.warnings.is_none(),
            "an enforce deny carries no warning"
        );
    }

    #[test]
    fn evaluate_dry_run_admits_within_budget_without_warning() {
        // T012: dry-run admits a within-budget pod normally (no warning, Allow).
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store_with_mode(EnforcementMode::DryRun),
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Allow);
        assert_eq!(outcome.summary.enforcement_mode, "dry_run");
        assert!(
            outcome.response.warnings.is_none(),
            "a within-budget admit carries no warning"
        );
    }

    // ---- spec-004: fail-closed paths reject in dry-run mode too (US2) ----
    //
    // The dry-run conversion happens ONLY at the check_budget Deny branch. Every
    // error path returns before check_budget is reached, so it is structurally
    // impossible for dry-run to convert an error rejection (FR-006 / Principle I).
    // These tests verify that architectural guarantee holds under dry-run mode.

    #[test]
    fn dry_run_rejects_stale_capacity_data() {
        // T018: stale data rejects even in dry-run mode.
        let req = request(&pod("15", "1"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store_with_mode(EnforcementMode::DryRun),
            &populated_capacity_store(),
            now() + 60, // 60s older than the 30s threshold → stale
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "stale capacity data must reject in dry-run mode (FR-006)"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_stale");
        assert!(
            outcome.response.warnings.is_none(),
            "a fail-closed reject carries no warning even in dry-run"
        );
    }

    #[test]
    fn dry_run_rejects_missing_allocation_singleton() {
        // T019: a missing Allocation singleton rejects in dry-run mode. The mode
        // lives on the Allocation spec, so a missing instance cannot be dry-run —
        // the fail-closed path is reached before any mode resolution.
        let req = request(&pod("15", "1"), Operation::Create, None);
        let (empty, _writer) = kube::runtime::reflector::store::<Allocation>();
        let outcome = evaluate(
            &req,
            &empty,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "missing Allocation must reject in dry-run mode (FR-006)"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
    }

    #[test]
    fn dry_run_rejects_missing_cluster_capacity() {
        // T020: a missing ClusterCapacity rejects in dry-run mode.
        let req = request(&pod("15", "1"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store_with_mode(EnforcementMode::DryRun),
            &empty_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "missing ClusterCapacity must reject in dry-run mode (FR-006)"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
    }

    // ---- spec-004: record_metrics maps DryRunDeny (US3) ----

    #[test]
    fn record_metrics_maps_dry_run_deny_to_dry_run_deny_label() {
        // T024: a DryRunDeny decision records the dry_run_deny verdict label,
        // distinct from a plain deny.
        let metrics = Metrics::new();
        let summary = DecisionSummary {
            workload: "default/p".to_string(),
            operation: "CREATE".to_string(),
            verdict: DecisionVerdict::DryRunDeny,
            reason: "cpu_over_budget".to_string(),
            budget_percent: 80,
            freshness_seconds: 0,
            latency_ms: 1,
            enforcement_mode: "dry_run".to_string(),
            exemption_reason: None,
            cpu: ResourceFigures {
                resource: ResourceType::Cpu,
                allocated: 70_000,
                requested: 15_000,
                projected: 85_000,
                ceiling: 80_000,
                total_allocatable: 100_000,
                over: true,
            },
            memory: ResourceFigures {
                resource: ResourceType::Memory,
                ..ResourceFigures::default()
            },
        };
        record_metrics(&metrics, &summary);
        let text = metrics.render();
        assert!(
            text.contains(
                r#"capacity_admission_verdicts_total{resource="cpu",verdict="dry_run_deny"} 1"#
            ),
            "DryRunDeny must bump the dry_run_deny series: {text}"
        );
        // The dry-run admit must NOT also bump the plain deny series.
        assert!(
            text.contains(r#"capacity_admission_verdicts_total{resource="cpu",verdict="deny"} 0"#),
            "dry-run deny must not bump the enforced deny series: {text}"
        );
    }

    // ---- spec-008: Exempt verdict + exemptions counter (data-model §3.3/§4.2) ----

    #[test]
    fn decision_summary_exempt_constructor_sets_verdict_and_reason() {
        // T005: the exempt() summary builder sets verdict=Exempt + the reason.
        let req = request(&pod("1", "1"), Operation::Create, None);
        let summary = DecisionSummary::exempt(
            &req,
            ExemptionReason::PriorityClass,
            EnforcementMode::Enforce,
        );
        assert_eq!(summary.verdict, DecisionVerdict::Exempt);
        assert_eq!(
            summary.exemption_reason,
            Some(ExemptionReason::PriorityClass)
        );
        assert_eq!(summary.enforcement_mode, "enforce");
        assert_eq!(summary.workload, "default/p");
        assert_eq!(summary.operation, "CREATE");
    }

    #[test]
    fn record_metrics_exempt_bumps_exemptions_not_verdicts() {
        // T005: an Exempt decision bumps capacity_admission_exemptions_total
        // (with the reason label) and does NOT touch capacity_admission_verdicts_total.
        // Latency is still observed (data-model §4.2); capacity gauges are not
        // refreshed (no figures computed).
        let metrics = Metrics::new();
        let summary = DecisionSummary {
            workload: "monitoring/p".to_string(),
            operation: "CREATE".to_string(),
            verdict: DecisionVerdict::Exempt,
            exemption_reason: Some(ExemptionReason::Namespace),
            reason: String::new(),
            budget_percent: -1,
            freshness_seconds: -1,
            latency_ms: 2,
            enforcement_mode: "enforce".to_string(),
            cpu: ResourceFigures::default(),
            memory: ResourceFigures {
                resource: ResourceType::Memory,
                ..ResourceFigures::default()
            },
        };
        record_metrics(&metrics, &summary);
        let text = metrics.render();
        assert!(
            text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 1"#),
            "Exempt must bump the exemptions counter with the reason: {text}"
        );
        assert!(
            text.contains(r#"capacity_admission_verdicts_total{resource="cpu",verdict="allow"} 0"#),
            "Exempt must NOT bump the verdicts counter: {text}"
        );
        assert!(
            text.contains("capacity_admission_decision_duration_seconds_count 1"),
            "Exempt must still observe latency: {text}"
        );
    }

    #[test]
    fn evaluate_rejects_stale_data() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        // 60s older than the fixture clock → exceeds the 30s threshold.
        let outcome = evaluate(
            &req,
            &populated_store(),
            &populated_capacity_store(),
            now() + 60,
            30,
            WEBHOOK_NS,
        );
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_stale");
        assert_eq!(outcome.response.result.code, 500);
    }

    #[test]
    fn evaluate_rejects_missing_cluster_capacity() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let outcome = evaluate(
            &req,
            &populated_store(),
            &empty_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
    }

    // ---- spec-008: exemption check in evaluate() (insertion point) ----
    //
    // The exemption check is a single check_exemption() call inserted AFTER the
    // Allocation singleton + its status are found and BEFORE the freshness check
    // (data-model 3.1). Fail-closed paths (missing allocation/status) reject
    // before reaching it.

    #[test]
    fn evaluate_exempts_over_budget_pod_in_excluded_namespace() {
        // T006/US1: an over-budget pod in an excluded namespace is admitted
        // (Exempt) with no budget check.
        let req = request_in("monitoring", &pod("15", "1"), Operation::Create, None);
        let store = populated_store_excluded(Some(vec!["monitoring"]), None);
        let outcome = evaluate(
            &req,
            &store,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(outcome.response.allowed, "excluded namespace -> admitted");
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Exempt);
        assert_eq!(
            outcome.summary.exemption_reason,
            Some(ExemptionReason::Namespace)
        );
        assert!(
            outcome.response.warnings.is_none(),
            "an exempt admit carries no warning"
        );
    }

    #[test]
    fn evaluate_nonexempt_over_budget_pod_is_still_denied() {
        // T006/US1 AC2: a non-excluded namespace is still budget-checked.
        let req = request_in("app-team-a", &pod("15", "1"), Operation::Create, None);
        let store = populated_store_excluded(Some(vec!["monitoring"]), None);
        let outcome = evaluate(
            &req,
            &store,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Deny);
    }

    #[test]
    fn evaluate_exempts_pod_in_webhook_namespace_with_empty_config() {
        // T006/FR-007: the webhook's own namespace is exempt even with both
        // exclusion lists empty ("empty CRD cache" = empty exclusion config).
        let req = request_in(WEBHOOK_NS, &pod("15", "1"), Operation::Create, None);
        let store = populated_store_excluded(None, None);
        let outcome = evaluate(
            &req,
            &store,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(outcome.response.allowed, "webhook ns -> exempt (FR-007)");
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Exempt);
        assert_eq!(
            outcome.summary.exemption_reason,
            Some(ExemptionReason::WebhookNamespace)
        );
    }

    #[test]
    fn evaluate_exempts_pod_by_priority_class() {
        // T006/US2: an over-budget pod with an excluded priority class is
        // admitted regardless of namespace.
        let req = request_in(
            "app-team-a",
            &pod_with_priority("15", "1", Some("system-node-critical")),
            Operation::Create,
            None,
        );
        let store = populated_store_excluded(None, Some(vec!["system-node-critical"]));
        let outcome = evaluate(
            &req,
            &store,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Exempt);
        assert_eq!(
            outcome.summary.exemption_reason,
            Some(ExemptionReason::PriorityClass)
        );
    }

    #[test]
    fn evaluate_excluded_pod_still_rejected_when_status_missing() {
        // T006 fail-closed integrity: a pod in an excluded namespace whose
        // Allocation status is missing is rejected BEFORE the exemption check.
        let req = request_in("monitoring", &pod("15", "1"), Operation::Create, None);
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        let mut allocation = allocation_with(status());
        allocation.status = None; // status missing -> reject before exemption.
        allocation.spec.excluded_namespaces = Some(vec!["monitoring".to_string()]);
        writer.apply_watcher_event(&watcher::Event::Apply(allocation));
        let outcome = evaluate(
            &req,
            &store,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "missing status rejects before exemption (fail-closed)"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
    }

    #[test]
    fn evaluate_excluded_pod_still_rejected_when_allocation_missing() {
        // T006 fail-closed integrity: a missing Allocation singleton rejects even
        // a webhook-namespace pod. Cold-start self-exemption at this layer is the
        // apiserver namespaceSelector's job (FR-009 / research R4).
        let req = request_in(WEBHOOK_NS, &pod("15", "1"), Operation::Create, None);
        let (empty, _writer) = kube::runtime::reflector::store::<Allocation>();
        let outcome = evaluate(
            &req,
            &empty,
            &populated_capacity_store(),
            now(),
            30,
            WEBHOOK_NS,
        );
        assert!(
            !outcome.response.allowed,
            "missing Allocation rejects before exemption (fail-closed)"
        );
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
    }

    // ---- assess_freshness ----

    #[test]
    fn freshness_current_data_is_not_stale() {
        let f = assess_freshness("2026-07-26T00:00:00Z", now(), 30);
        assert!(!f.stale);
        assert_eq!(f.seconds, 0);
    }

    #[test]
    fn freshness_older_than_threshold_is_stale() {
        let f = assess_freshness("2026-07-26T00:00:00Z", now() + 45, 30);
        assert!(f.stale);
        assert_eq!(f.seconds, 45);
        assert_eq!(f.age_secs, 45);
    }

    #[test]
    fn freshness_exactly_at_threshold_is_not_stale() {
        // "older than" is strict: age == threshold is still fresh.
        let f = assess_freshness("2026-07-26T00:00:00Z", now() + 30, 30);
        assert!(!f.stale);
    }

    #[test]
    fn freshness_unparseable_timestamp_is_stale() {
        let f = assess_freshness("garbage", now(), 30);
        assert!(f.stale);
        assert_eq!(f.seconds, -1);
    }

    // ---- with_timeout / with_catch_unwind / classify_error ----

    #[tokio::test]
    async fn with_timeout_returns_value_when_fast() {
        let result = with_timeout(async { 42u8 }, 100).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn with_timeout_fails_closed_when_slow() {
        let slow = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            42u8
        };
        let result = with_timeout(slow, 10).await;
        assert!(matches!(
            result,
            Err(AdmissionError::Timeout { timeout_ms: 10 })
        ));
    }

    #[test]
    fn catch_unwind_maps_panic_to_internal_error() {
        let result = with_catch_unwind(|| panic!("boom"));
        assert!(matches!(result, Err(AdmissionError::InternalError)));
    }

    #[test]
    fn classify_error_preserves_known_variants() {
        let boxed: Box<dyn std::error::Error> = Box::new(AdmissionError::Timeout { timeout_ms: 5 });
        assert!(matches!(
            classify_error(boxed),
            AdmissionError::Timeout { timeout_ms: 5 }
        ));
    }

    #[test]
    fn classify_error_maps_foreign_errors_to_unknown() {
        let boxed: Box<dyn std::error::Error> = Box::new(std::io::Error::other("surprise"));
        match classify_error(boxed) {
            AdmissionError::Unknown { detail } => assert!(detail.contains("surprise")),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
