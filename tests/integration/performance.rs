//! Performance benchmark for the admission decision (T042, SC-005).
//!
//! Measures p50/p99 admission-decision latency over 10,000 iterations against a
//! pre-populated reflector cache (the hot path is an in-memory read + budget
//! arithmetic). Asserts p99 < 100 ms and p50 < 50 ms.
//!
//! `#[ignore]`d by default so it stays out of the normal `cargo test` gate (and
//! T044's "no ignored test runs by default" check). Run with:
//!
//! ```sh
//! cargo test --test performance -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::parse_rfc3339;
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use kube::runtime::watcher;

const ITERATIONS: usize = 10_000;
const GIB: i64 = 1024 * 1024 * 1024;
const FIXTURE_TIME: &str = "2026-07-26T00:00:00Z";

#[tokio::test]
#[ignore = "performance benchmark — run with --ignored"]
async fn admission_decision_meets_latency_targets() {
    // Pre-populate the caches: 100 CPU / 200 GiB allocatable, 80% budget, 70 CPU
    // currently allocated — the decision is a budget check against a small pod.
    let (allocation_store, mut alloc_writer) = kube::runtime::reflector::store::<Allocation>();
    let mut allocation = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec { budget_percent: 80 },
    );
    allocation.status = Some(AllocationStatus {
        allocated_cpu_milli: 70_000,
        allocated_memory_bytes: 110 * GIB,
        ceiling_cpu_milli: 80_000,
        ceiling_memory_bytes: 160 * GIB,
        utilization_percent_cpu: 0.875,
        utilization_percent_memory: 0.6875,
        last_updated: FIXTURE_TIME.to_string(),
    });
    alloc_writer.apply_watcher_event(&watcher::Event::Apply(allocation));

    let (capacity_store, mut cap_writer) = kube::runtime::reflector::store::<ClusterCapacity>();
    let mut capacity = ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {});
    capacity.status = Some(ClusterCapacityStatus {
        total_allocatable_cpu_milli: 100_000,
        total_allocatable_memory_bytes: 200 * GIB,
        node_count: 2,
        last_updated: FIXTURE_TIME.to_string(),
    });
    cap_writer.apply_watcher_event(&watcher::Event::Apply(capacity));

    let now = parse_rfc3339(FIXTURE_TIME).unwrap();
    let state = AppState::with_clock(
        Arc::new(allocation_store),
        Arc::new(capacity_store),
        Arc::new(move || now),
        Arc::new(Metrics::new()),
    );

    // A small pod (5 CPU / 40 GiB) that fits → admit each iteration.
    let body = Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "kind": "AdmissionReview",
            "apiVersion": "admission.k8s.io/v1",
            "request": {
                "uid": "perf",
                "name": "perf-pod",
                "namespace": "default",
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "resource": {"group": "", "version": "v1", "resource": "pods"},
                "operation": "CREATE",
                "userInfo": {"username": "perf"},
                "object": {
                    "kind": "Pod", "apiVersion": "v1",
                    "metadata": {"name": "perf-pod"},
                    "spec": {"containers": [{
                        "name": "c", "image": "nginx",
                        "resources": {"requests": {"cpu": "5", "memory": "40Gi"}}
                    }]}
                },
                "oldObject": null,
                "dryRun": false,
            }
        }))
        .unwrap(),
    );

    // Warm up (first call pays one-time parsing/allocation costs).
    handle(body.clone(), &state).await;

    let mut samples_us: Vec<u64> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let response = handle(body.clone(), &state).await;
        let elapsed = start.elapsed().as_micros() as u64;
        assert!(response.allowed, "benchmark must use an admitting request");
        samples_us.push(elapsed);
    }

    samples_us.sort_unstable();
    let p50 = percentile(&samples_us, 50);
    let p99 = percentile(&samples_us, 99);
    let p50_ms = p50 as f64 / 1000.0;
    let p99_ms = p99 as f64 / 1000.0;

    println!(
        "admission decision over {ITERATIONS} iterations: p50 = {p50_ms:.3} ms, p99 = {p99_ms:.3} ms"
    );

    // SC-005.
    assert!(p99_ms < 100.0, "p99 {p99_ms} ms exceeds 100 ms target");
    assert!(p50_ms < 50.0, "p50 {p50_ms} ms exceeds 50 ms target");
}

/// Nearest-rank percentile of a sorted sample (microseconds).
fn percentile(sorted: &[u64], pct: u32) -> u64 {
    let n = sorted.len();
    let idx = (pct as usize * n)
        .div_ceil(100)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}
