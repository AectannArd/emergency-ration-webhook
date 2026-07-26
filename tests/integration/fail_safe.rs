//! Integration tests for fail-safe operation (User Story 3, T030).
//!
//! Every path that cannot authoritatively verify a workload fits MUST reject
//! (Constitution Principle I, NON-NEGOTIABLE). Each enumerated failure condition
//! is driven through `handle()` (or the fail-safe guard it composes) and
//! asserted to return `allowed: false` with the contract reason/code.
//!
//! Covers quickstart.md Scenario 3 and the Error Path Matrix.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::{now_unix, parse_rfc3339, rfc3339_from_unix};
use capacity_admission_webhook::webhook::error::AdmissionError;
use capacity_admission_webhook::webhook::handler::{AppState, handle, with_timeout};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::core::admission::{AdmissionResponse, Operation};
use kube::runtime::reflector::Store;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;

/// A fresh allocation status whose `last_updated` is `age_secs` in the past
/// relative to `clock_now`.
fn status_aged(clock_now: i64, age_secs: i64) -> AllocationStatus {
    AllocationStatus {
        allocated_cpu_milli: 70_000,
        allocated_memory_bytes: 110 * GIB,
        ceiling_cpu_milli: 80_000,
        ceiling_memory_bytes: 160 * GIB,
        utilization_percent_cpu: 0.875,
        utilization_percent_memory: 0.6875,
        last_updated: rfc3339_from_unix(clock_now - age_secs),
    }
}

fn allocation_with(status: AllocationStatus) -> Allocation {
    let mut a = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
        },
    );
    a.status = Some(status);
    a
}

fn capacity_store(total_cpu: i64, total_mem: i64, last_updated: &str) -> Store<ClusterCapacity> {
    let (store, mut writer) = kube::runtime::reflector::store::<ClusterCapacity>();
    let mut c = ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {});
    c.status = Some(ClusterCapacityStatus {
        total_allocatable_cpu_milli: total_cpu,
        total_allocatable_memory_bytes: total_mem,
        node_count: 2,
        last_updated: last_updated.to_string(),
    });
    writer.apply_watcher_event(&watcher::Event::Apply(c));
    store
}

fn allocation_store(status: AllocationStatus) -> Store<Allocation> {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(allocation_with(status)));
    store
}

/// Build AppState with an injected clock pinned to `clock_now`, default
/// timeouts (decision 100ms, freshness 30s).
fn state(
    allocation: Store<Allocation>,
    capacity: Store<ClusterCapacity>,
    clock_now: i64,
) -> AppState {
    AppState::with_clock(
        Arc::new(allocation),
        Arc::new(capacity),
        Arc::new(move || clock_now),
        Arc::new(Metrics::new()),
    )
}

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

fn review_body(uid: &str, op: Operation, object: &Pod) -> Bytes {
    let object = serde_json::to_value(object).unwrap();
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
            "uid": uid,
            "name": uid,
            "namespace": "default",
            "kind": {"group": "", "version": "v1", "kind": "Pod"},
            "resource": {"group": "", "version": "v1", "resource": "pods"},
            "operation": op_str,
            "userInfo": {"username": "operator@example.com"},
            "object": object,
            "oldObject": null,
            "dryRun": false,
        }
    });
    Bytes::from(serde_json::to_vec(&review).unwrap())
}

fn assert_denied(resp: &AdmissionResponse, code: u16, reason_substr: &str) {
    assert!(!resp.allowed, "must be rejected (fail-closed)");
    assert_eq!(resp.result.code, code, "status code mismatch");
    assert!(
        resp.result.message.contains(reason_substr),
        "message {:?} should contain {reason_substr:?}",
        resp.result.message
    );
}

// ---- (1) stale capacity data (T032) ----

#[tokio::test]
async fn stale_capacity_data_is_rejected() {
    let now = parse_rfc3339("2026-07-26T12:00:00Z").unwrap();
    let allocation = allocation_store(status_aged(now, 60)); // 60s old > 30s threshold
    let capacity = capacity_store(100_000, 200 * GIB, &rfc3339_from_unix(now));
    let resp = handle(
        review_body("stale", Operation::Create, &pod("5", "40Gi")),
        &state(allocation, capacity, now),
    )
    .await;
    assert_denied(&resp, 500, "capacity data unavailable");
    assert!(
        resp.result.message.contains("exceeds 30s threshold"),
        "{}",
        resp.result.message
    );
}

#[tokio::test]
async fn fresh_capacity_data_is_not_rejected_for_staleness() {
    let now = parse_rfc3339("2026-07-26T12:00:00Z").unwrap();
    let allocation = allocation_store(status_aged(now, 5)); // 5s old < 30s threshold
    let capacity = capacity_store(100_000, 200 * GIB, &rfc3339_from_unix(now));
    let resp = handle(
        review_body("fresh", Operation::Create, &pod("5", "40Gi")),
        &state(allocation, capacity, now),
    )
    .await;
    // 5 CPU fits → admitted (not rejected for staleness).
    assert!(
        resp.allowed,
        "fresh data must not be stale-denied: {}",
        resp.result.message
    );
}

// ---- (2) Allocation CRD not populated ----

#[tokio::test]
async fn missing_allocation_is_rejected() {
    let now = now_unix();
    let empty_allocation = kube::runtime::reflector::store::<Allocation>().0;
    let capacity = capacity_store(100_000, 200 * GIB, &rfc3339_from_unix(now));
    let resp = handle(
        review_body("noalloc", Operation::Create, &pod("5", "40Gi")),
        &state(empty_allocation, capacity, now),
    )
    .await;
    assert_denied(&resp, 500, "allocation state not initialised");
}

// ---- (3) ClusterCapacity CRD missing ----

#[tokio::test]
async fn missing_cluster_capacity_is_rejected() {
    let now = parse_rfc3339("2026-07-26T12:00:00Z").unwrap();
    let allocation = allocation_store(status_aged(now, 5));
    let empty_capacity = kube::runtime::reflector::store::<ClusterCapacity>().0;
    let resp = handle(
        review_body("nocap", Operation::Create, &pod("5", "40Gi")),
        &state(allocation, empty_capacity, now),
    )
    .await;
    assert_denied(&resp, 500, "cluster capacity state not initialised");
}

// ---- (4) malformed AdmissionReview (T033) ----

#[tokio::test]
async fn malformed_admission_review_is_rejected() {
    let now = now_unix();
    let allocation = allocation_store(status_aged(now, 5));
    let capacity = capacity_store(100_000, 200 * GIB, &rfc3339_from_unix(now));
    let body = Bytes::from_static(b"{ this is not valid json }");
    let resp = handle(body, &state(allocation, capacity, now)).await;
    assert_denied(&resp, 400, "admission request malformed");
}

// ---- (4b) unparseable resource quantity (T034) ----

#[tokio::test]
async fn unparseable_quantity_is_rejected() {
    let now = parse_rfc3339("2026-07-26T12:00:00Z").unwrap();
    let allocation = allocation_store(status_aged(now, 5));
    let capacity = capacity_store(100_000, 200 * GIB, &rfc3339_from_unix(now));
    let resp = handle(
        review_body("badqty", Operation::Create, &pod("not-a-quantity", "40Gi")),
        &state(allocation, capacity, now),
    )
    .await;
    assert_denied(&resp, 400, "cannot parse resource quantity");
}

// ---- (5) decision timeout (T035) ----

#[tokio::test]
async fn decision_timeout_is_rejected() {
    // The real decision is sub-millisecond, so a natural timeout cannot be
    // produced through `handle`. Exercise the timeout guard directly with a
    // deliberately slow future — this is the exact guard `handle` composes.
    let slow = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        AdmissionResponse::invalid("never")
    };
    let result = with_timeout(slow, 10).await;
    match result {
        Err(AdmissionError::Timeout { timeout_ms }) => assert_eq!(timeout_ms, 10),
        other => panic!("expected Timeout, got {other:?}"),
    }
    // The error maps to a fail-closed 500 response with the contract message.
    let resp = AdmissionError::Timeout { timeout_ms: 10 }.into_response("");
    assert_denied(&resp, 500, "timed out after 10ms");
}

// ---- (6) unknown error catch-all (T037) ----

#[tokio::test]
async fn unknown_error_is_rejected() {
    let foreign: Box<dyn std::error::Error> = Box::new(std::io::Error::other("unexpected"));
    let error = capacity_admission_webhook::webhook::handler::classify_error(foreign);
    let resp = error.into_response("");
    assert_denied(&resp, 500, "internal error");
}
