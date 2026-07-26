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
    ) -> Self {
        Self {
            allocation_store,
            capacity_store,
            decision_timeout_ms,
            capacity_freshness_timeout_secs,
            clock: Arc::new(time_util::now_unix),
            metrics,
        }
    }

    /// State with an injected clock (pinned time) and default timeouts — used by
    /// integration tests to make the freshness computation deterministic.
    pub fn with_clock(
        allocation_store: Arc<Store<Allocation>>,
        capacity_store: Arc<Store<ClusterCapacity>>,
        clock: Clock,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            allocation_store,
            capacity_store,
            decision_timeout_ms: 100,
            capacity_freshness_timeout_secs: 30,
            clock,
            metrics,
        }
    }
}

/// Build the axum router for the webhook endpoints.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/validate", post(validate))
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
        Ok(Err(error)) => reject_outcome(error, &meta, DecisionVerdict::Error),
        // The outer Err is the timeout or panic (InternalError); classify any
        // other unexpected error through the unknown catch-all (T037).
        Err(error) => reject_outcome(error, &meta, DecisionVerdict::Error),
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
    ))
}

/// Read the cached allocation/capacity, compute the capacity figures and
/// freshness, and decide. Fail-closed when the allocation state is not yet
/// populated (Principle I). Infallible: every failure becomes a reject outcome.
pub fn evaluate(
    request: &AdmissionRequest<Pod>,
    allocation_store: &Store<Allocation>,
    capacity_store: &Store<ClusterCapacity>,
    now: i64,
    freshness_threshold_secs: u64,
) -> DecisionOutcome {
    let Some(allocation) =
        allocation_store.find(|allocation| allocation.name_any() == CLUSTER_ALLOCATION_NAME)
    else {
        return reject_outcome(
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
        );
    };
    let budget_percent = allocation.spec.budget_percent;
    let Some(status) = allocation.status.clone() else {
        return reject_outcome(
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
        );
    };

    let freshness = assess_freshness(&status.last_updated, now, freshness_threshold_secs);

    // T032: enforce the freshness threshold. Data older than the threshold (or
    // with an unparseable timestamp) cannot be authoritatively verified → deny
    // (Principle I).
    if freshness.stale {
        return reject_outcome(
            AdmissionError::CapacityDataStale {
                age_secs: freshness.age_secs,
                threshold_secs: freshness_threshold_secs,
            },
            &request_meta_of(request),
            DecisionVerdict::Error,
        );
    }

    // The supply-side cache must also be present; a missing ClusterCapacity
    // means the supply state is not initialised → deny (Error Path Matrix).
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
        );
    };
    let total_cpu = capacity_status.total_allocatable_cpu_milli;
    let total_mem = capacity_status.total_allocatable_memory_bytes;

    // Resolve the effective request (figures). A quantity parse failure is
    // fail-closed (T034).
    let effective = match effective_request(request) {
        Ok(effective) => effective,
        Err(error) => {
            return reject_outcome(error, &request_meta_of(request), DecisionVerdict::Error)
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
            let response = AdmissionError::OverBudget {
                violations: violations.clone(),
            }
            .into_response(&request.uid);
            DecisionOutcome {
                response,
                summary: DecisionSummary::decision(
                    request,
                    budget_percent,
                    freshness.seconds,
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
                )
                .verdict(DecisionVerdict::Deny),
            }
        }
    }
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
    pub cpu: ResourceFigures,
    pub memory: ResourceFigures,
}

impl DecisionSummary {
    fn decision(
        request: &AdmissionRequest<Pod>,
        budget_percent: i32,
        freshness_seconds: i64,
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
            cpu,
            memory,
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
/// never reaches here).
fn reject_outcome(
    error: AdmissionError,
    meta: &RequestMeta,
    verdict: DecisionVerdict,
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
            cpu: ResourceFigures::default(),
            memory: ResourceFigures::default(),
        },
    }
}

/// Emit the structured tracing event(s) for a decision (T026).
///
/// - Admit → one INFO event per resource, with every Logging Contract field.
/// - Deny → one WARN event per resource (reason names the violated resource).
/// - Error → one ERROR event with the reason/error.
fn emit_log(summary: &DecisionSummary) {
    match summary.verdict {
        DecisionVerdict::Error => {
            tracing::error!(
                target: "capacity_admission",
                workload = %summary.workload,
                operation = %summary.operation,
                decision = "error",
                reason = %summary.reason,
                error = %summary.reason,
                budget_percent = summary.budget_percent,
                freshness_seconds = summary.freshness_seconds,
                latency_ms = summary.latency_ms,
                "admission rejected"
            );
        }
        DecisionVerdict::Allow => {
            for (rtype, figures) in [("cpu", &summary.cpu), ("memory", &summary.memory)] {
                tracing::info!(
                    target: "capacity_admission",
                    workload = %summary.workload,
                    operation = %summary.operation,
                    decision = "allow",
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
        DecisionVerdict::Deny => {
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
                    decision = "deny",
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
    for (resource, figures) in [
        (MetricResource::Cpu, &summary.cpu),
        (MetricResource::Memory, &summary.memory),
    ] {
        let verdict = match summary.verdict {
            DecisionVerdict::Error => VerdictLabel::Error,
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
    use crate::crd::{Allocation, AllocationSpec, AllocationStatus};
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
            AllocationSpec { budget_percent: 80 },
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
        let mut c = ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {});
        c.status = Some(ClusterCapacityStatus {
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 200 * 1024 * 1024 * 1024,
            node_count: 2,
            last_updated: "2026-07-26T00:00:00Z".to_string(),
        });
        writer.apply_watcher_event(&kube::runtime::watcher::Event::Apply(c));
        store
    }

    fn populated_store() -> Store<Allocation> {
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        writer.apply_watcher_event(&watcher::Event::Apply(allocation_with(status())));
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
        let outcome = evaluate(&req, &store, &capacity, now(), 30);
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
        );
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.verdict, DecisionVerdict::Error);
        assert_eq!(outcome.summary.reason, "capacity_data_stale");
        assert_eq!(outcome.response.result.code, 500);
    }

    #[test]
    fn evaluate_rejects_missing_cluster_capacity() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let outcome = evaluate(&req, &populated_store(), &empty_capacity_store(), now(), 30);
        assert!(!outcome.response.allowed);
        assert_eq!(outcome.summary.reason, "capacity_data_missing");
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
