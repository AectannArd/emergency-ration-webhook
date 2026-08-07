//! Allocation Controller (T019) — the *demand* side.
//!
//! Sums pod resource requests across non-terminal pods, reads the budget from the
//! `cluster-allocation` `Allocation` CRD `spec`, computes the ceiling from the
//! `ClusterCapacity` supply, and writes the result back to the `Allocation`
//! `.status`. See `contracts/allocation-crd.md` §Controller Behaviour.

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Patch, PatchParams, PostParams};
use kube::runtime::reflector::Store;
use kube::runtime::{reflector, watcher};
use kube::{Api, Client, ResourceExt};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, EnforcementMode, resolve_effective_budgets,
};
use crate::resources::quantity::extract_pod_requests;
use crate::time_util::now_rfc3339;
use crate::webhook::admission::ceiling_per_resource;

/// A pod counts toward current allocation unless its phase is terminal.
///
/// Per `contracts/allocation-crd.md` §Pod Phase Filtering: `Pending`,
/// `Running`, and `Unknown` are counted; `Succeeded` and `Failed` are not. A pod
/// with no phase yet (just created, not scheduled) is counted — its requests are
/// reserved.
pub fn is_non_terminal(phase: Option<&str>) -> bool {
    !matches!(phase, Some("Failed") | Some("Succeeded"))
}

/// Sum effective CPU (milli) and memory (bytes) requests across non-terminal
/// pods, applying the Kubernetes defaulting convention via
/// [`extract_pod_requests`]. Pure and unit-tested.
pub fn sum_pod_allocation<'a, I>(pods: I) -> (i64, i64)
where
    I: IntoIterator<Item = &'a Pod>,
{
    let mut cpu = 0i64;
    let mut memory = 0i64;
    for pod in pods {
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        if !is_non_terminal(phase) {
            continue;
        }
        let Some(spec) = pod.spec.as_ref() else {
            continue;
        };
        // A pod with an unparseable quantity is skipped here; such a pod could not
        // have passed admission (the webhook rejects unparseable quantities), so it
        // never reaches the running set.
        if let Ok((c, m)) = extract_pod_requests(spec) {
            cpu += c;
            memory += m;
        }
    }
    (cpu, memory)
}

/// Build the full `AllocationStatus` from the raw figures. Each resource gets its
/// own ceiling — `floor(supply * budget / 100)` via [`ceiling_per_resource`] — and
/// the resolved per-resource budgets are echoed into the status (FR-009).
/// Utilisation is `allocated / ceiling` (0 when there is no ceiling).
///
/// spec-012 (research R3, Option B): the caller resolves the per-resource budgets
/// once via [`resolve_effective_budgets`] and passes the tuple here, so this
/// function stays pure arithmetic (figures + budgets → status).
pub fn build_allocation_status(
    allocated: (i64, i64),
    total_supply: (i64, i64),
    budgets: (i32, i32),
) -> AllocationStatus {
    let ceilings = ceiling_per_resource(total_supply, budgets);
    AllocationStatus {
        allocated_cpu_milli: allocated.0,
        allocated_memory_bytes: allocated.1,
        ceiling_cpu_milli: ceilings.0,
        ceiling_memory_bytes: ceilings.1,
        utilization_percent_cpu: ratio(allocated.0, ceilings.0),
        utilization_percent_memory: ratio(allocated.1, ceilings.1),
        effective_cpu_budget_percent: budgets.0,
        effective_memory_budget_percent: budgets.1,
        last_updated: now_rfc3339(),
    }
}

fn ratio(allocated: i64, ceiling: i64) -> f64 {
    if ceiling == 0 {
        0.0
    } else {
        allocated as f64 / ceiling as f64
    }
}

/// The `budgetPercent` seeded into an auto-created `cluster-allocation` instance.
/// A safe default below 100% so a fresh cluster admits workloads but still guards
/// against full overcommit (Constitution Principle II).
const DEFAULT_BUDGET_PERCENT: i32 = 80;

/// The decision [`ensure_singleton`] reaches from a singleton existence check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingletonCheck {
    /// The instance exists — leave it untouched (never overwrite the budget).
    Exists,
    /// The instance is missing (404) — create the default singleton.
    Missing,
    /// The check errored unexpectedly — log and retry next cycle.
    Error,
}

/// Whether a kube error is a 404 NotFound — i.e. the singleton is absent. The
/// HTTP code is matched directly so the behaviour is robust to reason-string
/// variations across Kubernetes versions.
fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 404)
}

/// Classify the result of `Api::get` on the singleton: `Ok` → exists, 404 →
/// create, anything else → retry.
fn classify_check<T>(result: &Result<T, kube::Error>) -> SingletonCheck {
    match result {
        Ok(_) => SingletonCheck::Exists,
        Err(err) if is_not_found(err) => SingletonCheck::Missing,
        Err(_) => SingletonCheck::Error,
    }
}

/// Whether a `create` attempt left the singleton in place. `AlreadyExists` (409)
/// is a success — another replica won the race, but the singleton now exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateOutcome {
    Created,
    AlreadyExists,
    Error,
}

fn classify_create<T>(result: &Result<T, kube::Error>) -> CreateOutcome {
    match result {
        Ok(_) => CreateOutcome::Created,
        Err(kube::Error::Api(status)) if status.code == 409 => CreateOutcome::AlreadyExists,
        Err(_) => CreateOutcome::Error,
    }
}

/// The default `cluster-allocation` instance created when the singleton is
/// absent: the only user-configurable fields, `budgetPercent` and
/// `enforcementMode`, are seeded with [`DEFAULT_BUDGET_PERCENT`] and
/// `enforce` (FR-010) so a fresh cluster starts in the fail-closed enforcement
/// mode. An operator can patch both afterwards.
fn default_allocation_singleton() -> Allocation {
    Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: DEFAULT_BUDGET_PERCENT,
            enforcement_mode: Some(EnforcementMode::Enforce),
            excluded_namespaces: None,
            excluded_priority_classes: None,
            // spec-012 FR-008: a fresh cluster boots in legacy mode — no per-resource
            // overrides, so both resources fall back to `budget_percent` (byte-identical
            // to the pre-spec-012 controller).
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    )
}

/// Idempotent get-or-create of the `cluster-allocation` singleton.
///
/// Called once at controller start. A 409 `AlreadyExists` (e.g. another replica
/// won the race) is treated as success, and an existing instance is never
/// overwritten — so an operator-set `budgetPercent` is always preserved. Without
/// this, `recompute` finds no budget and skips writing status, leaving the
/// webhook with no capacity data so it fails closed on every admission
/// (Constitution Principle I).
pub async fn ensure_singleton(api: &Api<Allocation>) {
    let lookup = api.get(CLUSTER_ALLOCATION_NAME).await;
    match classify_check(&lookup) {
        SingletonCheck::Exists => debug!(
            name = CLUSTER_ALLOCATION_NAME,
            "Allocation singleton already exists, preserving operator budget"
        ),
        SingletonCheck::Missing => {
            let created = api
                .create(&PostParams::default(), &default_allocation_singleton())
                .await;
            match classify_create(&created) {
                CreateOutcome::Created => info!(
                    name = CLUSTER_ALLOCATION_NAME,
                    budget_percent = DEFAULT_BUDGET_PERCENT,
                    "created Allocation singleton with default budget"
                ),
                CreateOutcome::AlreadyExists => debug!(
                    name = CLUSTER_ALLOCATION_NAME,
                    "Allocation singleton already exists (race with another replica)"
                ),
                CreateOutcome::Error => {
                    if let Err(err) = &created {
                        warn!(%err, "failed to create Allocation singleton; retrying next cycle");
                    }
                }
            }
        }
        SingletonCheck::Error => {
            if let Err(err) = &lookup {
                warn!(%err, "failed to check Allocation singleton; retrying next cycle");
            }
        }
    }
}

/// Recompute allocation from the caches and merge-patch the `Allocation` status.
async fn recompute(
    pod_store: &Store<Pod>,
    capacity_store: &Store<ClusterCapacity>,
    allocation_api: &Api<Allocation>,
) {
    // The budget lives in the Allocation CRD spec. It changes rarely; a periodic
    // GET is cheap relative to the recompute interval and avoids a third cache.
    // spec-012: resolve per-resource budgets (override-or-fallback) from the spec;
    // each resource then gets its own ceiling (FR-002).
    let budgets = match allocation_api.get(CLUSTER_ALLOCATION_NAME).await {
        Ok(allocation) => resolve_effective_budgets(&allocation.spec),
        Err(err) if is_not_found(&err) => {
            // The singleton vanished since startup (e.g. an operator deleted it):
            // recreate it and let the next tick read the fresh budget rather than
            // writing status against a stale/unknown value.
            warn!(%err, "Allocation singleton missing during recompute; recreating");
            ensure_singleton(allocation_api).await;
            return;
        }
        Err(err) => {
            debug!(%err, "Allocation get failed; skipping recompute");
            return;
        }
    };

    let pods = pod_store.state();
    let allocated = sum_pod_allocation(pods.iter().map(|pod| pod.as_ref()));

    let supply = capacity_store
        .find(|c| c.name_any() == CLUSTER_CAPACITY_NAME)
        .and_then(|c| c.status.clone())
        .map(|s| {
            (
                s.total_allocatable_cpu_milli,
                s.total_allocatable_memory_bytes,
            )
        })
        .unwrap_or((0, 0));

    let status = build_allocation_status(allocated, supply, budgets);
    let params = PatchParams::default();
    if let Err(err) = allocation_api
        .patch_status(
            CLUSTER_ALLOCATION_NAME,
            &params,
            &Patch::Merge(super::status_merge_patch(&status)),
        )
        .await
    {
        warn!(%err, "failed to patch Allocation status");
    }
}

/// Run the controller until the runtime is shut down.
///
/// Keeps pod and `ClusterCapacity` reflector caches warm in background tasks,
/// then recomputes the `Allocation` status on a short interval. Every recompute
/// reads only from the in-process caches (plus a single budget GET); the
/// admission hot path never touches this.
pub async fn run(client: Client) {
    let pods_api = Api::<Pod>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client.clone());
    let allocation_api = Api::<Allocation>::all(client);

    // Create the singleton before the first recompute — otherwise the budget GET
    // 404s and status is never written, leaving the webhook with no capacity data.
    ensure_singleton(&allocation_api).await;

    let (pod_store, pod_writer) = reflector::store::<Pod>();
    let (capacity_store, capacity_writer) = reflector::store::<ClusterCapacity>();

    tokio::spawn(
        reflector::reflector(
            pod_writer,
            watcher::watcher(pods_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "pod watch error");
            }
        }),
    );
    tokio::spawn(
        reflector::reflector(
            capacity_writer,
            watcher::watcher(capacity_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "ClusterCapacity watch error");
            }
        }),
    );

    // Initial recompute: write status immediately rather than waiting for the
    // first ticker tick, so the Allocation singleton is never left status-less
    // after ensure_singleton creates it. The caches may still be cold here, so
    // this first write may carry zeros — the reflectors sync and the ticker then
    // refreshes with real figures within the tick window.
    recompute(&pod_store, &capacity_store, &allocation_api).await;

    // Bounded-latency recompute: any change is reflected within the tick window.
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        recompute(&pod_store, &capacity_store, &allocation_api).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::mock_api::{
        already_exists, created_object, mock_client, not_found, ok_object,
    };
    use k8s_openapi::api::core::v1::{Container, PodSpec, ResourceRequirements};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use kube::core::Status;
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn pod_with(phase: Option<&str>, cpu: &str, memory: &str) -> Pod {
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
        requests.insert("memory".to_string(), Quantity(memory.to_string()));
        Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    resources: Some(ResourceRequirements {
                        requests: Some(requests),
                        limits: None,
                        claims: None,
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(k8s_openapi::api::core::v1::PodStatus {
                phase: phase.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // ---- is_non_terminal ----

    #[test]
    fn pending_running_unknown_are_counted() {
        assert!(is_non_terminal(Some("Pending")));
        assert!(is_non_terminal(Some("Running")));
        assert!(is_non_terminal(Some("Unknown")));
        assert!(is_non_terminal(None), "unscheduled pod is counted");
    }

    #[test]
    fn succeeded_and_failed_are_terminal() {
        assert!(!is_non_terminal(Some("Succeeded")));
        assert!(!is_non_terminal(Some("Failed")));
    }

    // ---- sum_pod_allocation ----

    #[test]
    fn sums_running_pod_requests() {
        let pods = vec![
            pod_with(Some("Running"), "1", "1Gi"),
            pod_with(Some("Pending"), "2", "2Gi"),
        ];
        let (cpu, memory) = sum_pod_allocation(&pods);
        assert_eq!(cpu, 3_000);
        assert_eq!(memory, 3 * 1024 * 1024 * 1024);
    }

    #[test]
    fn terminal_pods_excluded() {
        let pods = vec![
            pod_with(Some("Running"), "5", "5Gi"),
            pod_with(Some("Succeeded"), "100", "100Gi"),
            pod_with(Some("Failed"), "100", "100Gi"),
        ];
        let (cpu, memory) = sum_pod_allocation(&pods);
        assert_eq!(cpu, 5_000, "only the Running pod counts");
        assert_eq!(memory, 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn no_pods_is_zero() {
        assert_eq!(sum_pod_allocation(Vec::<Pod>::new().iter()), (0, 0));
    }

    // ---- build_allocation_status ----

    #[test]
    fn status_computes_ceiling_and_utilisation() {
        // supply 100 CPU / 200 GiB, budget 80% → ceiling 80 CPU / 160 GiB.
        // allocated 70 CPU / 110 GiB.
        let status = build_allocation_status(
            (70_000, 110 * 1024 * 1024 * 1024),
            (100_000, 200 * 1024 * 1024 * 1024),
            (80, 80),
        );
        assert_eq!(status.ceiling_cpu_milli, 80_000);
        assert_eq!(status.ceiling_memory_bytes, 160 * 1024 * 1024 * 1024);
        assert!((status.utilization_percent_cpu - 0.875).abs() < 1e-9);
        assert!((status.utilization_percent_memory - (110.0 / 160.0)).abs() < 1e-9);
        assert!(status.last_updated.ends_with('Z'));
    }

    #[test]
    fn zero_budget_yields_zero_ceiling() {
        let status = build_allocation_status((10_000, 10_000), (100_000, 100_000), (0, 0));
        assert_eq!(status.ceiling_cpu_milli, 0);
        assert_eq!(status.ceiling_memory_bytes, 0);
        assert_eq!(status.utilization_percent_cpu, 0.0);
    }

    // ---- spec-012: per-resource budgets (US1 / FR-003 / FR-009) ----

    #[test]
    fn build_allocation_status_uses_per_resource_budgets() {
        // T013: per-resource budgets (90, 60) on supply (100_000, 200 GiB) produce
        // ceiling_cpu_milli = 90_000, ceiling_memory_bytes = floor(200GiB*60/100),
        // and the effective-budget status fields mirror the budgets (FR-003/FR-009).
        const GIB: i64 = 1024 * 1024 * 1024;
        let status = build_allocation_status((70_000, 110 * GIB), (100_000, 200 * GIB), (90, 60));
        assert_eq!(status.ceiling_cpu_milli, 90_000);
        assert_eq!(status.ceiling_memory_bytes, (200 * GIB) * 60 / 100);
        assert_eq!(status.effective_cpu_budget_percent, 90);
        assert_eq!(status.effective_memory_budget_percent, 60);
        // Asymmetric budgets → asymmetric ceilings (sanity: neither matches the
        // symmetric 80% legacy value).
        assert_eq!(status.ceiling_cpu_milli, 90_000);
        assert_eq!(status.ceiling_memory_bytes, 120 * GIB);
    }

    // ---- spec-012 US3: effective budgets exposed in status (FR-009) ----

    #[test]
    fn build_allocation_status_exposes_effective_budgets() {
        // T022: the status echoes the resolved per-resource budgets that governed
        // the ceilings, for observability (FR-009). Covers the asymmetric case
        // (90, 60) -> (90, 60) and the legacy no-override case (80, 80) -> (80, 80).
        const GIB: i64 = 1024 * 1024 * 1024;
        let supply = (100_000, 200 * GIB);

        let asymmetric = build_allocation_status((70_000, 110 * GIB), supply, (90, 60));
        assert_eq!(asymmetric.effective_cpu_budget_percent, 90);
        assert_eq!(asymmetric.effective_memory_budget_percent, 60);

        let legacy = build_allocation_status((70_000, 110 * GIB), supply, (80, 80));
        assert_eq!(legacy.effective_cpu_budget_percent, 80);
        assert_eq!(legacy.effective_memory_budget_percent, 80);
    }

    // ---- spec-012 US2: backward compatibility (FR-005 / FR-008) ----

    #[test]
    fn no_override_ceilings_byte_identical_to_legacy() {
        // T019 / FR-005 / research R10: a no-override singleton produces ceilings
        // byte-identical to the pre-spec-012 controller for every budget_percent.
        // (budget, None, None) resolves to (budget, budget), so the per-resource
        // path must equal the legacy single-budget path — the US2 AC1 gate.
        const GIB: i64 = 1024 * 1024 * 1024;
        let supply = (100_000, 200 * GIB);
        for budget in [0, 50, 80, 100] {
            let status = build_allocation_status((0, 0), supply, (budget, budget));
            assert_eq!(
                status.ceiling_cpu_milli,
                supply.0 * budget as i64 / 100,
                "CPU ceiling matches legacy floor(supply*{budget}/100)"
            );
            assert_eq!(
                status.ceiling_memory_bytes,
                supply.1 * budget as i64 / 100,
                "memory ceiling matches legacy floor(supply*{budget}/100)"
            );
            // The effective budgets equal the legacy single budget.
            assert_eq!(status.effective_cpu_budget_percent, budget);
            assert_eq!(status.effective_memory_budget_percent, budget);
        }
    }

    // ---- singleton autocreation (ensure_singleton decision logic) ----

    /// Build a `kube::Error::Api` carrying a status with the given HTTP code and
    /// reason, mirroring what the apiserver returns for 404/409/etc.
    fn api_error(code: u16, reason: &str, message: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    #[test]
    fn existing_allocation_is_not_recreated() {
        // The operator set budgetPercent=50; the controller must NOT overwrite it.
        let lookup: Result<Allocation, kube::Error> = Ok(Allocation::new(
            CLUSTER_ALLOCATION_NAME,
            AllocationSpec {
                budget_percent: 50,
                enforcement_mode: None,
                excluded_namespaces: None,
                excluded_priority_classes: None,
                cpu_budget_percent: None,
                memory_budget_percent: None,
            },
        ));
        assert_eq!(classify_check(&lookup), SingletonCheck::Exists);
    }

    #[test]
    fn missing_allocation_triggers_create() {
        let lookup: Result<Allocation, kube::Error> =
            Err(api_error(404, "NotFound", "allocations not found"));
        assert_eq!(classify_check(&lookup), SingletonCheck::Missing);
    }

    #[test]
    fn unexpected_allocation_get_error_is_retried_not_created() {
        // A 403 (RBAC) is not a missing instance — never create blindly.
        let lookup: Result<Allocation, kube::Error> = Err(api_error(403, "Forbidden", "forbidden"));
        assert_eq!(classify_check(&lookup), SingletonCheck::Error);
    }

    #[test]
    fn allocation_create_conflict_is_success() {
        // 409 = another replica won the race; the singleton now exists.
        let created: Result<Allocation, kube::Error> =
            Err(api_error(409, "AlreadyExists", "already exists"));
        assert_eq!(classify_create(&created), CreateOutcome::AlreadyExists);
    }

    #[test]
    fn allocation_create_unexpected_error_is_failure() {
        let created: Result<Allocation, kube::Error> =
            Err(api_error(403, "Forbidden", "forbidden"));
        assert_eq!(classify_create(&created), CreateOutcome::Error);
    }

    #[test]
    fn default_allocation_uses_default_budget_and_name() {
        let alloc = default_allocation_singleton();
        assert_eq!(
            alloc.metadata.name.as_deref(),
            Some(CLUSTER_ALLOCATION_NAME)
        );
        assert_eq!(
            alloc.spec.budget_percent, DEFAULT_BUDGET_PERCENT,
            "auto-created singleton seeds a safe default budget"
        );
        assert_eq!(
            alloc.spec.enforcement_mode,
            Some(EnforcementMode::Enforce),
            "auto-created singleton seeds enforce mode (FR-010)"
        );
    }

    #[test]
    fn default_allocation_seeds_no_per_resource_overrides() {
        // T020 / FR-008: the auto-created singleton has no per-resource overrides,
        // so a fresh cluster boots in legacy mode (both resources at budget_percent).
        let alloc = default_allocation_singleton();
        assert!(
            alloc.spec.cpu_budget_percent.is_none(),
            "FR-008: auto-created singleton seeds no CPU override"
        );
        assert!(
            alloc.spec.memory_budget_percent.is_none(),
            "FR-008: auto-created singleton seeds no memory override"
        );
    }

    // ---- ensure_singleton against a mocked apiserver (Principle VI) ----
    //
    // These drive a real `kube::Api` through `mock_api`, scripting the apiserver
    // responses to prove the get-or-create wiring end-to-end — not just the
    // decision logic covered by the pure tests above.

    #[tokio::test]
    async fn ensure_singleton_creates_allocation_when_absent() {
        let (client, mut handle) = mock_client();
        let api = Api::<Allocation>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // 1. Existence GET → 404 NotFound (singleton absent).
        let (req, respond) = handle.next_request().await.expect("existence GET");
        assert_eq!(req.method().as_str(), "GET");
        assert!(
            req.uri().path().ends_with("/cluster-allocation"),
            "GET targets the singleton: {}",
            req.uri()
        );
        respond.send_response(not_found());

        // 2. Create POST → 201 Created.
        let (req, respond) = handle.next_request().await.expect("create POST");
        assert_eq!(req.method().as_str(), "POST");
        respond.send_response(created_object(&default_allocation_singleton()));

        task.await.expect("ensure_singleton did not panic");
    }

    #[tokio::test]
    async fn ensure_singleton_preserves_operator_budget_when_present() {
        let (client, mut handle) = mock_client();
        let api = Api::<Allocation>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // The operator set budgetPercent=50. Existence GET → 200 OK with that
        // instance. ensure_singleton must NOT overwrite it (no create).
        let existing = Allocation::new(
            CLUSTER_ALLOCATION_NAME,
            AllocationSpec {
                budget_percent: 50,
                enforcement_mode: None,
                excluded_namespaces: None,
                excluded_priority_classes: None,
                cpu_budget_percent: None,
                memory_budget_percent: None,
            },
        );
        let (req, respond) = handle.next_request().await.expect("existence GET");
        assert_eq!(req.method().as_str(), "GET");
        respond.send_response(ok_object(&existing));

        task.await.expect("ensure_singleton did not panic");

        // No create may follow: the operator's budget must be left intact. A
        // short timeout proves no second request is issued (it would resolve
        // immediately if a create were attempted).
        match tokio::time::timeout(Duration::from_millis(100), handle.next_request()).await {
            Err(_elapsed) => {}
            Ok(Some(req)) => panic!(
                "ensure_singleton issued an unexpected create {}",
                req.0.method()
            ),
            Ok(None) => {}
        }
    }

    #[tokio::test]
    async fn ensure_singleton_tolerates_create_conflict() {
        let (client, mut handle) = mock_client();
        let api = Api::<Allocation>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // Existence GET → 404.
        let (_req, respond) = handle.next_request().await.expect("existence GET");
        respond.send_response(not_found());

        // Create POST → 409 AlreadyExists (another replica won the race).
        let (req, respond) = handle.next_request().await.expect("create POST");
        assert_eq!(req.method().as_str(), "POST");
        respond.send_response(already_exists());

        // 409 is success: ensure_singleton completes without error.
        task.await.expect("ensure_singleton did not panic on 409");
    }
}
