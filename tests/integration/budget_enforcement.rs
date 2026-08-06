//! Integration tests for budget enforcement (User Story 1, T015).
//!
//! Drives the real admission path end-to-end: a JSON `AdmissionReview` body in →
//! `handle()` (deserialise → read cached allocation → extract pod requests →
//! `check_budget`) → `AdmissionResponse` out. Allocation state is injected through
//! a real kube `reflector::Store`, the same cache the live webhook reads.
//!
//! Covers spec.md US1 acceptance scenarios 1–5 and SC-002 (denial figures).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, ClusterCapacity,
    resolve_effective_budgets,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::parse_rfc3339;
use capacity_admission_webhook::webhook::admission::ceiling_per_resource;
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::core::admission::Operation;
use kube::runtime::reflector::Store;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;

/// Allocation status for the spec's cluster: 100 CPU / 200 GiB allocatable,
/// 80% budget → ceiling 80 CPU / 160 GiB, currently allocated 70 CPU / 110 GiB.
fn spec_allocation_status() -> AllocationStatus {
    AllocationStatus {
        allocated_cpu_milli: 70_000,
        allocated_memory_bytes: 110 * GIB,
        ceiling_cpu_milli: 80_000,
        ceiling_memory_bytes: 160 * GIB,
        utilization_percent_cpu: 0.875,
        utilization_percent_memory: 0.6875,
        last_updated: "2026-07-26T14:32:05Z".to_string(),
        effective_cpu_budget_percent: 80,
        effective_memory_budget_percent: 80,
    }
}

/// A reflector store holding the `cluster-allocation` singleton with `status`.
fn populated_store() -> Store<Allocation> {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    let mut allocation = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
            excluded_namespaces: None,
            excluded_priority_classes: None,
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    );
    allocation.status = Some(spec_allocation_status());
    writer.apply_watcher_event(&watcher::Event::Apply(allocation));
    store
}

/// A capacity store with the 100 CPU / 200 GiB the budget fixtures imply
/// (ceiling 80 CPU = 80% of 100). The webhook now requires the supply cache to
/// be present (Principle I), so the budget tests supply it.
fn capacity_store() -> Store<ClusterCapacity> {
    use capacity_admission_webhook::crd::{ClusterCapacitySpec, ClusterCapacityStatus};
    let (store, mut writer) = kube::runtime::reflector::store::<ClusterCapacity>();
    let mut c = ClusterCapacity::new(
        "cluster-capacity",
        ClusterCapacitySpec {
            node_selectors: None,
        },
    );
    c.status = Some(ClusterCapacityStatus {
        total_allocatable_cpu_milli: 100_000,
        total_allocatable_memory_bytes: 200 * GIB,
        node_count: 2,
        last_updated: spec_allocation_status().last_updated.clone(),
        excluded_node_count: 0,
        excluded_by_unschedulable: 0,
        excluded_by_selector: 0,
    });
    writer.apply_watcher_event(&watcher::Event::Apply(c));
    store
}

/// Application state with the clock pinned to the fixture's `last_updated`, so
/// the (Phase 5) freshness check sees age 0 and the budget decision governs.
fn app_state(store: Store<Allocation>) -> AppState {
    let now = parse_rfc3339(&spec_allocation_status().last_updated).unwrap();
    AppState::with_clock(
        Arc::new(store),
        Arc::new(capacity_store()),
        Arc::new(move || now),
        Arc::new(Metrics::new()),
        "capacity-admission".to_string(),
    )
}

/// Build a pod with one container requesting `cpu` / `memory`.
fn pod(cpu: &str, memory: &str) -> Pod {
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
        ..Default::default()
    }
}

/// A pod whose single container declares no resources (request → 0).
fn bare_pod() -> Pod {
    Pod {
        spec: Some(PodSpec {
            containers: vec![Container::default()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Serialise a pod (optionally an old object for UPDATE) into an AdmissionReview
/// body identical in shape to what the kube-apiserver sends.
fn review_body(name: &str, op: Operation, object: &Pod, old: Option<&Pod>) -> Bytes {
    let object = serde_json::to_value(object).unwrap();
    let old_object = match old {
        Some(o) => serde_json::to_value(o).unwrap(),
        None => serde_json::Value::Null,
    };
    let op_str = match op {
        Operation::Create => "CREATE",
        Operation::Update => "UPDATE",
        Operation::Delete => "DELETE",
        Operation::Connect => "CONNECT",
    };
    let review = serde_json::json!({
        "kind": "AdmissionReview",
        "apiVersion": "admission.k8s.io/v1",
        "request": {
            "uid": name,
            "name": name,
            "namespace": "default",
            "kind": {"group": "", "version": "v1", "kind": "Pod"},
            "resource": {"group": "", "version": "v1", "resource": "pods"},
            "operation": op_str,
            "userInfo": {"username": "operator@example.com"},
            "object": object,
            "oldObject": old_object,
            "dryRun": false,
        }
    });
    Bytes::from(serde_json::to_vec(&review).unwrap())
}

/// Run a review body through the handler against the populated store.
async fn admit(body: Bytes) -> kube::core::admission::AdmissionResponse {
    handle(body, &app_state(populated_store())).await
}

#[tokio::test]
async fn scenario1_pod_under_ceiling_is_admitted() {
    // Pod requests 5 CPU / 40 GiB → projected 75 CPU / 150 GiB, both under ceiling.
    let resp = admit(review_body(
        "s1",
        Operation::Create,
        &pod("5", "40Gi"),
        None,
    ))
    .await;
    assert!(resp.allowed, "pod fitting the budget must be admitted");
    assert_eq!(resp.uid, "s1");
}

#[tokio::test]
async fn scenario2_pod_over_ceiling_is_denied_with_figures() {
    // Pod requests 15 CPU → projected 85 > 80 ceiling. CPU is the violated resource.
    let resp = admit(review_body(
        "s2",
        Operation::Create,
        &pod("15", "10Gi"),
        None,
    ))
    .await;
    assert!(!resp.allowed, "pod exceeding the budget must be rejected");
    assert_eq!(resp.result.code, 403, "policy denial is a 403");
    let message = &resp.result.message;
    // SC-002: a single rejection message names the resource and all four figures.
    assert!(message.contains("CPU budget exceeded"));
    assert!(message.contains("allocated 70000m"));
    assert!(message.contains("requested 15000m"));
    assert!(message.contains("projected 85000m"));
    assert!(message.contains("ceiling 80000m"));
}

#[tokio::test]
async fn scenario3_pod_exactly_at_ceiling_is_admitted() {
    // Cluster at the CPU ceiling already (70 + 10 == 80 ceiling). Ceiling is inclusive.
    let status = AllocationStatus {
        allocated_cpu_milli: 80_000,
        ..spec_allocation_status()
    };
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    let mut allocation = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
            excluded_namespaces: None,
            excluded_priority_classes: None,
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    );
    allocation.status = Some(status);
    writer.apply_watcher_event(&watcher::Event::Apply(allocation));

    let body = review_body("s3", Operation::Create, &pod("5", "1Ki"), None);
    // Projected 85 > 80 → rejected: the budget is a hard ceiling, not a soft target.
    let resp = handle(body, &app_state(store)).await;
    assert!(!resp.allowed, "going one over the hard ceiling must reject");
}

#[tokio::test]
async fn scenario4_zero_request_pod_is_admitted() {
    let resp = admit(review_body("s4", Operation::Create, &bare_pod(), None)).await;
    assert!(resp.allowed, "a pod requesting nothing consumes no budget");
}

#[tokio::test]
async fn scenario5_update_evaluated_as_delta() {
    // An existing pod at 10 CPU is updated to 20 CPU: the system must evaluate the
    // +10 delta (70 → 80, exactly at the inclusive ceiling) and admit it.
    let old = pod("10", "1Ki");
    let new = pod("20", "1Ki");
    let resp = admit(review_body("s5", Operation::Update, &new, Some(&old))).await;
    assert!(
        resp.allowed,
        "update delta of +10 lands at the ceiling and must be admitted"
    );

    // Same pod updated to 30 CPU instead: +20 delta (70 → 90 > 80) must reject.
    let bigger = pod("30", "1Ki");
    let resp = admit(review_body("s5b", Operation::Update, &bigger, Some(&old))).await;
    assert!(!resp.allowed);
    assert!(resp.result.message.contains("projected 90000m"));
}

#[tokio::test]
async fn both_resources_over_budget_listed_in_message() {
    // 15 CPU (→85>80) and 60 GiB (→170>160): both violated, newline-separated.
    let resp = admit(review_body(
        "both",
        Operation::Create,
        &pod("15", "60Gi"),
        None,
    ))
    .await;
    assert!(!resp.allowed);
    let message = &resp.result.message;
    assert!(message.contains("CPU budget exceeded"));
    assert!(message.contains("memory budget exceeded"));
    assert_eq!(
        message.matches('\n').count(),
        1,
        "both resources on separate lines"
    );
}

// ---- spec-012 US1 AC1/AC2: per-resource asymmetric budgets ----
//
// An operator sets cpuBudgetPercent / memoryBudgetPercent overrides. The
// controller would resolve them (resolve_effective_budgets) and compute
// independent ceilings (ceiling_per_resource) into the Allocation status; this
// test builds that status via the real pipeline, injects it, and confirms the
// webhook — whose check_budget is unchanged — denies on the single exceeded
// resource (FR-011).

/// Build the `cluster-allocation` singleton carrying per-resource overrides, with
/// its status ceilings + effective budgets computed exactly as the controller
/// would from the 100 CPU / 200 GiB supply (`capacity_store`). `allocated` is the
/// pre-existing demand.
fn asymmetric_store(
    cpu_override: i32,
    mem_override: i32,
    allocated: (i64, i64),
) -> Store<Allocation> {
    let spec = AllocationSpec {
        budget_percent: 80,
        enforcement_mode: None,
        excluded_namespaces: None,
        excluded_priority_classes: None,
        cpu_budget_percent: Some(cpu_override),
        memory_budget_percent: Some(mem_override),
    };
    let budgets = resolve_effective_budgets(&spec);
    let (ceiling_cpu, ceiling_mem) = ceiling_per_resource((100_000, 200 * GIB), budgets);
    let mut allocation = Allocation::new(CLUSTER_ALLOCATION_NAME, spec);
    allocation.status = Some(AllocationStatus {
        allocated_cpu_milli: allocated.0,
        allocated_memory_bytes: allocated.1,
        ceiling_cpu_milli: ceiling_cpu,
        ceiling_memory_bytes: ceiling_mem,
        utilization_percent_cpu: 0.0,
        utilization_percent_memory: 0.0,
        last_updated: "2026-07-26T14:32:05Z".to_string(),
        effective_cpu_budget_percent: budgets.0,
        effective_memory_budget_percent: budgets.1,
    });
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(allocation));
    store
}

#[tokio::test]
async fn per_resource_asymmetric_cpu_admits_memory_denies() {
    // US1 AC1: cpuBudgetPercent 95 (ceiling 95 CPU), memoryBudgetPercent 30
    // (ceiling 60 GiB). A pod fitting CPU but exceeding memory → denied on memory
    // ONLY (FR-011).
    let store = asymmetric_store(95, 30, (0, 0));
    // Pod: 90 CPU (→ 90_000 ≤ 95_000 ceiling ✓), 150 GiB (→ 150 > 60 GiB ceiling ✗).
    let resp = handle(
        review_body("asym1", Operation::Create, &pod("90", "150Gi"), None),
        &app_state(store),
    )
    .await;
    assert!(!resp.allowed, "memory over its (lower) budget must deny");
    assert_eq!(resp.result.code, 403);
    assert!(
        resp.result.message.contains("memory budget exceeded"),
        "denial names memory: {}",
        resp.result.message
    );
    assert!(
        !resp.result.message.contains("CPU budget exceeded"),
        "CPU is NOT reported as violated (FR-011): {}",
        resp.result.message
    );
}

#[tokio::test]
async fn per_resource_asymmetric_swap_denies_cpu_only() {
    // US1 AC2: swap overrides — cpuBudgetPercent 30 (ceiling 30 CPU),
    // memoryBudgetPercent 95 (ceiling 190 GiB). The same pod now exceeds CPU but
    // not memory → denied on CPU ONLY.
    let store = asymmetric_store(30, 95, (0, 0));
    let resp = handle(
        review_body("asym2", Operation::Create, &pod("90", "150Gi"), None),
        &app_state(store),
    )
    .await;
    assert!(!resp.allowed, "CPU over its (lower) budget must deny");
    assert_eq!(resp.result.code, 403);
    assert!(
        resp.result.message.contains("CPU budget exceeded"),
        "denial names CPU: {}",
        resp.result.message
    );
    assert!(
        !resp.result.message.contains("memory budget exceeded"),
        "memory is NOT reported as violated: {}",
        resp.result.message
    );
}
