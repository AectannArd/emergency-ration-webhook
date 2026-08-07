//! BDD step definitions for dry-run enforcement mode (spec-004, T035–T036).
//!
//! Drives the real `handle` admission path through Cucumber expressions so the
//! spec's dry-run acceptance scenarios read as Gherkin. The `World` materialises
//! the cached Allocation/ClusterCapacity state — including the `enforcementMode`
//! on the Allocation spec — submits a pod, and asserts on the admission verdict,
//! the rejection message, and the warning a dry-run admit carries.
//!
//! Run with: `cargo test --test dry_run_bdd`.

use std::sync::Arc;

use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus, EnforcementMode,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::{parse_rfc3339, rfc3339_from_unix};
use capacity_admission_webhook::webhook::handler::{AppState, Clock, handle};
use cucumber::{World as _, given, then, when};
use kube::core::admission::AdmissionResponse;
use kube::runtime::reflector::Store;
use kube::runtime::reflector::store::Writer;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;
const FIXTURE_TIME: &str = "2026-07-26T00:00:00Z";

/// Pinned clock so a fresh (stale_secs == 0) Allocation sees age 0.
fn fixed_now() -> i64 {
    parse_rfc3339(FIXTURE_TIME).unwrap()
}

#[derive(cucumber::World)]
struct DryRunWorld {
    enforcement_mode: EnforcementMode,
    total_cpu_milli: i64,
    total_mem_bytes: i64,
    budget_percent: i32,
    allocated_cpu_milli: i64,
    allocated_mem_bytes: i64,
    /// Seconds since the Allocation status was last refreshed (0 = fresh).
    stale_secs: i64,
    allocation_store: Store<Allocation>,
    allocation_writer: Writer<Allocation>,
    capacity_store: Store<ClusterCapacity>,
    capacity_writer: Writer<ClusterCapacity>,
    metrics: Arc<Metrics>,
    last: Option<AdmissionResponse>,
}

impl std::fmt::Debug for DryRunWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The reflector stores/writers/metrics/response are not Debug; report
        // only the scenario figures (enough to diagnose a failing scenario).
        f.debug_struct("DryRunWorld")
            .field("enforcement_mode", &self.enforcement_mode.as_log_str())
            .field("total_cpu_milli", &self.total_cpu_milli)
            .field("budget_percent", &self.budget_percent)
            .field("allocated_cpu_milli", &self.allocated_cpu_milli)
            .field("stale_secs", &self.stale_secs)
            .field("admitted", &self.last.as_ref().is_some_and(|r| r.allowed))
            .finish()
    }
}

impl Default for DryRunWorld {
    fn default() -> Self {
        let (allocation_store, allocation_writer) = kube::runtime::reflector::store::<Allocation>();
        let (capacity_store, capacity_writer) =
            kube::runtime::reflector::store::<ClusterCapacity>();
        Self {
            enforcement_mode: EnforcementMode::Enforce,
            total_cpu_milli: 0,
            total_mem_bytes: 0,
            budget_percent: 80,
            allocated_cpu_milli: 0,
            allocated_mem_bytes: 0,
            stale_secs: 0,
            allocation_store,
            allocation_writer,
            capacity_store,
            capacity_writer,
            metrics: Arc::new(Metrics::new()),
            last: None,
        }
    }
}

impl DryRunWorld {
    /// Materialise the scenario's capacity/allocation figures (with the active
    /// enforcement mode and optional staleness) into the reflector caches, then
    /// run a pod through the admission handler.
    async fn submit(&mut self, request_cpu_milli: i64, request_mem_bytes: i64) {
        let capacity = {
            let mut c = ClusterCapacity::new(
                CLUSTER_CAPACITY_NAME,
                ClusterCapacitySpec {
                    node_selectors: None,
                },
            );
            c.status = Some(ClusterCapacityStatus {
                total_allocatable_cpu_milli: self.total_cpu_milli,
                total_allocatable_memory_bytes: self.total_mem_bytes,
                node_count: 2,
                last_updated: FIXTURE_TIME.to_string(),
                excluded_node_count: 0,
                excluded_by_unschedulable: 0,
                excluded_by_selector: 0,
            });
            c
        };
        self.capacity_writer
            .apply_watcher_event(&watcher::Event::Apply(capacity));

        let ceiling_cpu = self.total_cpu_milli * self.budget_percent as i64 / 100;
        let ceiling_mem = self.total_mem_bytes * self.budget_percent as i64 / 100;
        // A non-zero stale_secs ages the Allocation status past the 30s threshold.
        let last_updated = if self.stale_secs > 0 {
            rfc3339_from_unix(fixed_now() - self.stale_secs)
        } else {
            FIXTURE_TIME.to_string()
        };
        let allocation = {
            let mut a = Allocation::new(
                CLUSTER_ALLOCATION_NAME,
                AllocationSpec {
                    budget_percent: self.budget_percent,
                    enforcement_mode: Some(self.enforcement_mode),
                    excluded_namespaces: None,
                    excluded_priority_classes: None,
                    cpu_budget_percent: None,
                    memory_budget_percent: None,
                },
            );
            a.status = Some(AllocationStatus {
                allocated_cpu_milli: self.allocated_cpu_milli,
                allocated_memory_bytes: self.allocated_mem_bytes,
                ceiling_cpu_milli: ceiling_cpu,
                ceiling_memory_bytes: ceiling_mem,
                utilization_percent_cpu: ratio(self.allocated_cpu_milli, ceiling_cpu),
                utilization_percent_memory: ratio(self.allocated_mem_bytes, ceiling_mem),
                last_updated,
                effective_cpu_budget_percent: self.budget_percent,
                effective_memory_budget_percent: self.budget_percent,
            });
            a
        };
        self.allocation_writer
            .apply_watcher_event(&watcher::Event::Apply(allocation));

        let body = review_body(request_cpu_milli, request_mem_bytes);
        let state = AppState::with_clock(
            Arc::new(self.allocation_store.clone()),
            Arc::new(self.capacity_store.clone()),
            Arc::new(fixed_now) as Clock,
            Arc::clone(&self.metrics),
            "capacity-admission".to_string(),
        );
        self.last = Some(handle(body, &state).await);
    }
}

fn ratio(allocated: i64, ceiling: i64) -> f64 {
    if ceiling == 0 {
        0.0
    } else {
        allocated as f64 / ceiling as f64
    }
}

/// Serialise a pod requesting `cpu_milli` / `mem_bytes` into an AdmissionReview.
fn review_body(cpu_milli: i64, mem_bytes: i64) -> axum::body::Bytes {
    // Express requests in whole cores / GiB for readability; milli/byte inputs
    // are converted back to Kubernetes quantity strings.
    let cpu = if cpu_milli % 1000 == 0 {
        format!("{}", cpu_milli / 1000)
    } else {
        format!("{}m", cpu_milli)
    };
    let memory = format!("{}Gi", mem_bytes / GIB);
    let object = serde_json::json!({
        "kind": "Pod",
        "apiVersion": "v1",
        "metadata": {"name": "bdd-pod", "namespace": "default"},
        "spec": {
            "containers": [{
                "name": "c",
                "image": "nginx",
                "resources": {
                    "requests": {"cpu": cpu, "memory": memory}
                }
            }]
        }
    });
    let review = serde_json::json!({
        "kind": "AdmissionReview",
        "apiVersion": "admission.k8s.io/v1",
        "request": {
            "uid": "bdd",
            "name": "bdd-pod",
            "namespace": "default",
            "kind": {"group": "", "version": "v1", "kind": "Pod"},
            "resource": {"group": "", "version": "v1", "resource": "pods"},
            "operation": "CREATE",
            "userInfo": {"username": "operator@example.com"},
            "object": object,
            "oldObject": null,
            "dryRun": false,
        }
    });
    axum::body::Bytes::from(serde_json::to_vec(&review).unwrap())
}

// ---- Given ----

#[given(expr = "the cluster has {int} CPU and {int} GiB allocatable")]
async fn cluster_capacity(world: &mut DryRunWorld, cpu: i64, mem_gib: i64) {
    world.total_cpu_milli = cpu * 1000;
    world.total_mem_bytes = mem_gib * GIB;
}

#[given(expr = "the budget is {int} percent")]
async fn budget(world: &mut DryRunWorld, percent: i32) {
    world.budget_percent = percent;
}

#[given(expr = "the current allocation is {int} CPU and {int} GiB")]
async fn current_allocation(world: &mut DryRunWorld, cpu: i64, mem_gib: i64) {
    world.allocated_cpu_milli = cpu * 1000;
    world.allocated_mem_bytes = mem_gib * GIB;
}

#[given(expr = "the enforcement mode is {string}")]
async fn enforcement_mode(world: &mut DryRunWorld, mode: String) {
    world.enforcement_mode = match mode.as_str() {
        "enforce" => EnforcementMode::Enforce,
        "dry-run" => EnforcementMode::DryRun,
        other => panic!("unknown enforcement mode {other:?}; expected enforce/dry-run"),
    };
}

#[given(expr = "the allocation was last refreshed {int} seconds ago")]
async fn allocation_age(world: &mut DryRunWorld, secs: i64) {
    world.stale_secs = secs;
}

// ---- When ----

#[when(expr = "a pod requesting {int} CPU and {int} GiB is submitted")]
async fn submit_pod(world: &mut DryRunWorld, cpu: i64, mem_gib: i64) {
    world.submit(cpu * 1000, mem_gib * GIB).await;
}

// ---- Then ----

#[then("the pod is admitted")]
async fn admitted(world: &mut DryRunWorld) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(resp.allowed, "expected admission, but was rejected");
}

#[then("the pod is rejected")]
async fn rejected(world: &mut DryRunWorld) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(!resp.allowed, "expected rejection, but was admitted");
}

#[then(expr = "the admission warning contains {string}")]
async fn warning_contains(world: &mut DryRunWorld, fragment: String) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    let warnings = resp
        .warnings
        .as_ref()
        .expect("an admission warning was expected");
    assert_eq!(warnings.len(), 1, "expected exactly one warning");
    assert!(
        warnings[0].contains(&fragment),
        "warning {:?} does not contain {fragment:?}",
        warnings[0]
    );
}

#[then("the admission carries no warning")]
async fn no_warning(world: &mut DryRunWorld) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(
        resp.warnings.is_none(),
        "expected no warning, but got {:?}",
        resp.warnings
    );
}

#[then(expr = "the rejection message contains {string}")]
async fn message_contains(world: &mut DryRunWorld, fragment: String) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(
        resp.result.message.contains(&fragment),
        "rejection message {:?} does not contain {fragment:?}",
        resp.result.message
    );
}

#[tokio::main]
async fn main() {
    DryRunWorld::run("tests/bdd/features/dry_run.feature").await;
}
