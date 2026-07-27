//! BDD step definitions for fail-safe operation (User Story 3, T031).
//!
//! Drives the real `handle` admission path through Cucumber expressions,
//! setting up each failure condition (stale data, missing allocation/capacity,
//! malformed body, unparseable quantity) and asserting the fail-closed
//! rejection (`allowed: false`) with the contract reason in the message.
//!
//! Run with: `cargo test --test fail_safe_bdd`.

use std::sync::Arc;

use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::{parse_rfc3339, rfc3339_from_unix};
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use cucumber::{World as _, given, then, when};
use kube::core::admission::AdmissionResponse;
use kube::runtime::reflector::Store;
use kube::runtime::reflector::store::Writer;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;
const CLOCK_NOW: &str = "2026-07-26T12:00:00Z";

/// Which failure condition the scenario sets up.
#[derive(Default)]
enum Failure {
    #[default]
    None,
    /// No Allocation singleton in the cache.
    NoAllocation,
    /// No ClusterCapacity singleton in the cache.
    NoCapacity,
    /// The request body is not valid JSON.
    Malformed,
    /// The pod requests an unparseable CPU quantity.
    BadQuantity,
}

#[derive(cucumber::World)]
struct FailSafeWorld {
    total_cpu_milli: i64,
    total_mem_bytes: i64,
    budget_percent: i32,
    allocated_cpu_milli: i64,
    allocated_mem_bytes: i64,
    allocation_age_secs: i64,
    failure: Failure,
    allocation_store: Store<Allocation>,
    allocation_writer: Writer<Allocation>,
    capacity_store: Store<ClusterCapacity>,
    capacity_writer: Writer<ClusterCapacity>,
    last: Option<AdmissionResponse>,
}

impl std::fmt::Debug for FailSafeWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailSafeWorld")
            .field("failure", &std::mem::discriminant(&self.failure))
            .field("admitted", &self.last.as_ref().is_some_and(|r| r.allowed))
            .finish()
    }
}

impl Default for FailSafeWorld {
    fn default() -> Self {
        let (allocation_store, allocation_writer) = kube::runtime::reflector::store::<Allocation>();
        let (capacity_store, capacity_writer) =
            kube::runtime::reflector::store::<ClusterCapacity>();
        Self {
            total_cpu_milli: 0,
            total_mem_bytes: 0,
            budget_percent: 80,
            allocated_cpu_milli: 0,
            allocated_mem_bytes: 0,
            allocation_age_secs: 0,
            failure: Failure::default(),
            allocation_store,
            allocation_writer,
            capacity_store,
            capacity_writer,
            last: None,
        }
    }
}

impl FailSafeWorld {
    /// Materialise the scenario state and run the admission.
    async fn submit(&mut self, request_cpu_cores: Option<i64>, request_mem_gib: Option<i64>) {
        let now = parse_rfc3339(CLOCK_NOW).unwrap();

        // ClusterCapacity cache (unless the scenario removes it).
        if !matches!(self.failure, Failure::NoCapacity) {
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
                last_updated: rfc3339_from_unix(now),
                excluded_node_count: 0,
                excluded_by_unschedulable: 0,
                excluded_by_selector: 0,
            });
            self.capacity_writer
                .apply_watcher_event(&watcher::Event::Apply(c));
        }

        // Allocation cache (unless the scenario removes it).
        if !matches!(self.failure, Failure::NoAllocation) {
            let ceiling_cpu = self.total_cpu_milli * self.budget_percent as i64 / 100;
            let ceiling_mem = self.total_mem_bytes * self.budget_percent as i64 / 100;
            let mut a = Allocation::new(
                CLUSTER_ALLOCATION_NAME,
                AllocationSpec {
                    budget_percent: self.budget_percent,
                    enforcement_mode: None,
                },
            );
            a.status = Some(AllocationStatus {
                allocated_cpu_milli: self.allocated_cpu_milli,
                allocated_memory_bytes: self.allocated_mem_bytes,
                ceiling_cpu_milli: ceiling_cpu,
                ceiling_memory_bytes: ceiling_mem,
                utilization_percent_cpu: 0.0,
                utilization_percent_memory: 0.0,
                last_updated: rfc3339_from_unix(now - self.allocation_age_secs),
            });
            self.allocation_writer
                .apply_watcher_event(&watcher::Event::Apply(a));
        }

        let body = match self.failure {
            Failure::Malformed => axum::body::Bytes::from_static(b"{ not valid json }"),
            Failure::BadQuantity => review_body(Some("not-a-quantity"), Some(40)),
            _ => review_body(
                request_cpu_cores.map(|c| c.to_string()).as_deref(),
                request_mem_gib,
            ),
        };

        let state = AppState::with_clock(
            Arc::new(self.allocation_store.clone()),
            Arc::new(self.capacity_store.clone()),
            Arc::new(move || now),
            Arc::new(Metrics::new()),
        );
        self.last = Some(handle(body, &state).await);
    }
}

/// Build an AdmissionReview body for a pod requesting `cpu` / `mem_gib` GiB.
fn review_body(cpu: Option<&str>, mem_gib: Option<i64>) -> axum::body::Bytes {
    let cpu = cpu.unwrap_or("5").to_string();
    let mem = match mem_gib {
        Some(g) => format!("{g}Gi"),
        None => "40Gi".to_string(),
    };
    let object = serde_json::json!({
        "kind": "Pod",
        "apiVersion": "v1",
        "metadata": {"name": "bdd-pod", "namespace": "default"},
        "spec": {
            "containers": [{
                "name": "c",
                "image": "nginx",
                "resources": {"requests": {"cpu": cpu, "memory": mem}}
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

#[given(expr = "the cluster capacity is {int} CPU and {int} GiB at {int} percent budget")]
async fn cluster_capacity(world: &mut FailSafeWorld, cpu: i64, mem_gib: i64, budget: i32) {
    world.total_cpu_milli = cpu * 1000;
    world.total_mem_bytes = mem_gib * GIB;
    world.budget_percent = budget;
}

#[given(expr = "the allocation was last refreshed {int} seconds ago")]
async fn allocation_age(world: &mut FailSafeWorld, secs: i64) {
    world.allocation_age_secs = secs;
}

#[given(expr = "the current allocation is {int} CPU and {int} GiB")]
async fn current_allocation(world: &mut FailSafeWorld, cpu: i64, mem_gib: i64) {
    world.allocated_cpu_milli = cpu * 1000;
    world.allocated_mem_bytes = mem_gib * GIB;
}

#[given("the allocation state is not populated")]
async fn no_allocation(world: &mut FailSafeWorld) {
    world.failure = Failure::NoAllocation;
}

#[given("the cluster capacity is not populated")]
async fn no_capacity(world: &mut FailSafeWorld) {
    world.failure = Failure::NoCapacity;
}

#[given("the admission request is malformed")]
async fn malformed(world: &mut FailSafeWorld) {
    world.failure = Failure::Malformed;
}

#[given("a pod requests an unparseable CPU quantity")]
async fn bad_quantity(world: &mut FailSafeWorld) {
    world.failure = Failure::BadQuantity;
}

// ---- When ----

#[when(expr = "a pod requesting {int} CPU and {int} GiB is submitted")]
async fn submit_pod(world: &mut FailSafeWorld, cpu: i64, mem_gib: i64) {
    world.submit(Some(cpu), Some(mem_gib)).await;
}

#[when("it is submitted")]
async fn submit_it(world: &mut FailSafeWorld) {
    world.submit(None, None).await;
}

// ---- Then ----

#[then("the pod is rejected")]
async fn rejected(world: &mut FailSafeWorld) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(!resp.allowed, "expected rejection, but was admitted");
}

#[then(expr = "the rejection message contains {string}")]
async fn message_contains(world: &mut FailSafeWorld, fragment: String) {
    let resp = world.last.as_ref().expect("a pod was submitted");
    assert!(
        resp.result.message.contains(&fragment),
        "rejection message {:?} does not contain {fragment:?}",
        resp.result.message
    );
}

#[tokio::main]
async fn main() {
    FailSafeWorld::run("tests/bdd/features/fail_safe.feature").await;
}
