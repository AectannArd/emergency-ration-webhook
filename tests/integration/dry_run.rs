//! Integration tests for dry-run enforcement mode (spec-004, T029–T032).
//!
//! Drives the real admission path end-to-end: a JSON `AdmissionReview` body in →
//! `handle()` (deserialise → read the cached allocation, including
//! `enforcementMode` → extract pod requests → `check_budget` → dry-run
//! admit-with-warning or fail-closed reject) → `AdmissionResponse` out.
//! Allocation state (including the enforcement mode) is injected through a real
//! kube `reflector::Store` — the same cache the live webhook reads — so a mode
//! switch applied to the store takes effect on the very next decision (FR-002).
//!
//! Covers spec-004 US1 (shadow evaluation) and US2 (fail-closed integrity)
//! acceptance scenarios and the dry-run Error Path Matrix.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, ClusterCapacity,
    ClusterCapacitySpec, ClusterCapacityStatus, EnforcementMode, resolve_effective_budgets,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::{parse_rfc3339, rfc3339_from_unix};
use capacity_admission_webhook::webhook::admission::ceiling_per_resource;
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::core::admission::Operation;
use kube::runtime::reflector::Store;
use kube::runtime::reflector::store::Writer;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;
/// Pinned clock so the freshness check sees age 0 (fresh); the budget decision
/// governs. Mirrors the spec-001 budget fixtures: 100 CPU / 200 GiB allocatable,
/// 80% budget → ceiling 80 CPU / 160 GiB, 70 CPU / 110 GiB allocated.
const FIXTURE_TIME: &str = "2026-07-26T14:32:05Z";

fn fixture_now() -> i64 {
    parse_rfc3339(FIXTURE_TIME).unwrap()
}

fn spec_allocation_status() -> AllocationStatus {
    AllocationStatus {
        allocated_cpu_milli: 70_000,
        allocated_memory_bytes: 110 * GIB,
        ceiling_cpu_milli: 80_000,
        ceiling_memory_bytes: 160 * GIB,
        utilization_percent_cpu: 0.875,
        utilization_percent_memory: 0.6875,
        last_updated: FIXTURE_TIME.to_string(),
        effective_cpu_budget_percent: 80,
        effective_memory_budget_percent: 80,
    }
}

/// Capacity store with 100 CPU / 200 GiB total allocatable (feeds the ceilings).
fn capacity_store() -> Store<ClusterCapacity> {
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
        last_updated: FIXTURE_TIME.to_string(),
        excluded_node_count: 0,
        excluded_by_unschedulable: 0,
        excluded_by_selector: 0,
    });
    writer.apply_watcher_event(&watcher::Event::Apply(c));
    store
}

/// Build the `cluster-allocation` singleton in `mode` with the fresh fixture
/// status.
fn allocation_in(mode: EnforcementMode) -> Allocation {
    let mut a = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: Some(mode),
            excluded_namespaces: None,
            excluded_priority_classes: None,
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    );
    a.status = Some(spec_allocation_status());
    a
}

/// The singleton in `mode` whose status is 60s stale relative to the fixture
/// clock (exceeds the 30s freshness threshold).
fn stale_allocation_in(mode: EnforcementMode) -> Allocation {
    let mut a = allocation_in(mode);
    a.status.as_mut().unwrap().last_updated = rfc3339_from_unix(fixture_now() - 60);
    a
}

/// The singleton in `mode` carrying asymmetric per-resource overrides (spec-012),
/// with its ceilings + effective budgets computed exactly as the controller would
/// from the 100 CPU / 200 GiB supply (`capacity_store`). `allocated` starts at 0.
fn asymmetric_allocation_in(
    mode: EnforcementMode,
    cpu_override: i32,
    mem_override: i32,
) -> Allocation {
    let spec = AllocationSpec {
        budget_percent: 80,
        enforcement_mode: Some(mode),
        excluded_namespaces: None,
        excluded_priority_classes: None,
        cpu_budget_percent: Some(cpu_override),
        memory_budget_percent: Some(mem_override),
    };
    let budgets = resolve_effective_budgets(&spec);
    let (ceiling_cpu, ceiling_mem) = ceiling_per_resource((100_000, 200 * GIB), budgets);
    let mut a = Allocation::new(CLUSTER_ALLOCATION_NAME, spec);
    a.status = Some(AllocationStatus {
        allocated_cpu_milli: 0,
        allocated_memory_bytes: 0,
        ceiling_cpu_milli: ceiling_cpu,
        ceiling_memory_bytes: ceiling_mem,
        utilization_percent_cpu: 0.0,
        utilization_percent_memory: 0.0,
        last_updated: FIXTURE_TIME.to_string(),
        effective_cpu_budget_percent: budgets.0,
        effective_memory_budget_percent: budgets.1,
    });
    a
}

/// A reflector store + writer with the singleton in `mode` applied (fresh).
fn allocation_store(mode: EnforcementMode) -> (Store<Allocation>, Writer<Allocation>) {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(allocation_in(mode)));
    (store, writer)
}

/// Application state pinned to the fixture clock with the supplied allocation
/// store and the capacity fixtures.
fn state_with(allocation: Store<Allocation>) -> AppState {
    let now = fixture_now();
    AppState::with_clock(
        Arc::new(allocation),
        Arc::new(capacity_store()),
        Arc::new(move || now),
        Arc::new(Metrics::new()),
        "capacity-admission".to_string(),
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

/// Serialise a pod (optionally with an old object for UPDATE) into the
/// AdmissionReview body shape the kube-apiserver sends.
fn review_body(uid: &str, op: Operation, object: &Pod, old: Option<&Pod>) -> Bytes {
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
            "uid": uid,
            "name": uid,
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

/// An over-budget CREATE: 15 CPU → projected 85 > 80 ceiling.
fn over_budget_body(uid: &str) -> Bytes {
    review_body(uid, Operation::Create, &pod("15", "10Gi"), None)
}

// ---- T029: dry-run admits an over-budget pod with a warning ----

#[tokio::test]
async fn dry_run_admits_over_budget_pod_with_warning() {
    let (store, _writer) = allocation_store(EnforcementMode::DryRun);
    let resp = handle(over_budget_body("dry-run-1"), &state_with(store)).await;

    assert!(resp.allowed, "dry-run admits an over-budget pod");
    assert_eq!(resp.uid, "dry-run-1");
    // status is omitted on a dry-run admit (the pod is admitted, not rejected).
    assert_eq!(resp.result.code, 0, "no HTTP status on an allow");
    let warnings = resp
        .warnings
        .as_ref()
        .expect("dry-run admit carries a warning");
    assert_eq!(warnings.len(), 1, "one warning string");
    assert!(
        warnings[0].starts_with("Budget violations (dry-run): "),
        "warning is prefixed: {warnings:?}"
    );
    // The warning reuses the exact rejection message (same figures).
    assert!(
        warnings[0].contains("CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m"),
        "warning carries the would-be rejection figures: {warnings:?}"
    );
}

// ---- T030: dry-run with stale capacity data still rejects (fail-closed) ----

#[tokio::test]
async fn dry_run_fail_closed_stale_capacity_rejects() {
    // The Allocation is in dry-run mode, but its status is stale. The webhook
    // cannot authoritatively verify capacity → reject regardless of mode (FR-006).
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(stale_allocation_in(
        EnforcementMode::DryRun,
    )));
    let resp = handle(over_budget_body("dry-run-stale"), &state_with(store)).await;

    assert!(
        !resp.allowed,
        "stale data rejects even in dry-run mode (FR-006)"
    );
    assert_eq!(resp.result.code, 500);
    assert!(
        resp.warnings.is_none(),
        "a fail-closed reject carries no warning"
    );
    assert!(
        resp.result.message.contains("capacity data unavailable"),
        "{}",
        resp.result.message
    );
}

// ---- T031: enforce mode rejects an over-budget pod (no behaviour change) ----

#[tokio::test]
async fn enforce_rejects_over_budget_pod_unchanged() {
    let (store, _writer) = allocation_store(EnforcementMode::Enforce);
    let resp = handle(over_budget_body("enforce-1"), &state_with(store)).await;

    assert!(!resp.allowed, "enforce mode rejects an over-budget pod");
    assert_eq!(resp.result.code, 403, "policy denial is a 403");
    assert!(
        resp.warnings.is_none(),
        "an enforce deny carries no warning"
    );
    let message = &resp.result.message;
    assert!(
        message.contains("CPU budget exceeded") && message.contains("projected 85000m"),
        "enforce denial carries the figures: {message}"
    );
}

// ---- T032: a mode switch takes effect on the next decision ----

#[tokio::test]
async fn mode_switch_dry_run_to_enforce_affects_next_decision() {
    // FR-002: patching `enforcementMode` propagates through the reflector cache
    // and governs the very next decision, with no restart.
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    let state = state_with(store);

    // Start in dry-run: the same over-budget pod is admitted.
    writer.apply_watcher_event(&watcher::Event::Apply(allocation_in(
        EnforcementMode::DryRun,
    )));
    let resp_dry = handle(over_budget_body("switch-1"), &state).await;
    assert!(
        resp_dry.allowed,
        "dry-run admits the over-budget pod before the switch"
    );
    assert!(
        resp_dry.warnings.is_some(),
        "dry-run admit carries a warning"
    );

    // Operator patches the spec to enforce. The cache update is visible to the
    // next handle() call against the SAME state — no restart, no new AppState.
    writer.apply_watcher_event(&watcher::Event::Apply(allocation_in(
        EnforcementMode::Enforce,
    )));
    let resp_enforce = handle(over_budget_body("switch-2"), &state).await;
    assert!(
        !resp_enforce.allowed,
        "after switching to enforce, the same over-budget pod is rejected"
    );
    assert_eq!(resp_enforce.result.code, 403);
}

#[tokio::test]
async fn mode_switch_enforce_to_dry_run_affects_next_decision() {
    // The reverse switch: enforce → dry-run admits what was previously rejected.
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    let state = state_with(store);

    writer.apply_watcher_event(&watcher::Event::Apply(allocation_in(
        EnforcementMode::Enforce,
    )));
    assert!(
        !handle(over_budget_body("rev-1"), &state).await.allowed,
        "enforce rejects before the switch"
    );

    writer.apply_watcher_event(&watcher::Event::Apply(allocation_in(
        EnforcementMode::DryRun,
    )));
    let resp = handle(over_budget_body("rev-2"), &state).await;
    assert!(resp.allowed, "dry-run admits after the switch");
    assert!(resp.warnings.is_some(), "dry-run admit carries a warning");
}

// ---- spec-012: per-resource dry-run warning is resource-specific (edge case) ----
//
// With cpuBudgetPercent:95 (generous) / memoryBudgetPercent:30 (tight), a pod that
// fits CPU but exceeds memory, admitted in dry-run, carries a memory-ONLY warning.
// The per-resource split is symmetric between enforce and dry-run (contract §4.1:
// a memory-only violation produces a memory-only warning).

#[tokio::test]
async fn dry_run_asymmetric_memory_only_violation_warns_memory_only() {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(asymmetric_allocation_in(
        EnforcementMode::DryRun,
        95,
        30,
    )));
    // 1 CPU fits the 95% ceiling (95 CPU); 150 GiB exceeds the 30% ceiling (60 GiB).
    let resp = handle(
        review_body("dry-asym", Operation::Create, &pod("1", "150Gi"), None),
        &state_with(store),
    )
    .await;
    assert!(resp.allowed, "dry-run admits the memory-over pod");
    let warnings = resp
        .warnings
        .as_ref()
        .expect("dry-run admit carries a warning");
    assert_eq!(warnings.len(), 1, "one warning string: {warnings:?}");
    assert!(
        warnings[0].contains("memory budget exceeded"),
        "warning names memory only: {warnings:?}"
    );
    assert!(
        !warnings[0].contains("CPU budget exceeded"),
        "CPU is NOT in the warning (per-resource): {warnings:?}"
    );
}
