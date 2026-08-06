//! Integration tests for the workload exclusion policy (spec-008, T007–T009).
//!
//! Drives the real admission path end-to-end: a JSON `AdmissionReview` body in →
//! `handle()` (deserialise → read the cached allocation, including the new
//! `excludedNamespaces` / `excludedPriorityClasses` → `check_exemption` → exempt
//! admit OR the unchanged budget path) → `AdmissionResponse` out. Allocation
//! state is injected through a real kube `reflector::Store` — the same cache the
//! live webhook reads — so an exclusion list applied to the store takes effect on
//! the very next decision.
//!
//! Coverage:
//! - US1 (T007): namespace exclusion end-to-end + FR-007 webhook-ns
//!   self-exemption + FR-008 exemption counter.
//! - US2 (T008): priority-class exclusion end-to-end (match / no pc / unlisted /
//!   empty-string).
//! - US3 (T009): combined namespace + priority-class OR semantics, including the
//!   "matching both counts once" first-match rule.
//!
//! The budget fixtures mirror `dry_run.rs`: 100 CPU / 200 GiB allocatable, 80%
//! budget → ceiling 80 CPU / 160 GiB, 70 CPU / 110 GiB allocated. A 15 CPU pod
//! projects 85 > 80 → over budget, so a non-exempt pod is denied (enforce mode).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, ClusterCapacity,
    ClusterCapacitySpec, ClusterCapacityStatus, EnforcementMode,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::parse_rfc3339;
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::runtime::reflector::Store;
use kube::runtime::watcher;

const GIB: i64 = 1024 * 1024 * 1024;
/// Pinned clock so the freshness check sees age 0 (fresh); the budget decision
/// governs for non-exempt pods.
const FIXTURE_TIME: &str = "2026-07-26T14:32:05Z";
/// The webhook's own namespace threaded into `AppState` for the FR-007
/// self-exemption check. Mirrors the production default (`capacity-admission`);
/// the budget-fixture pods live elsewhere (`monitoring`, `app-team-a`,
/// `kube-system`), so they are NOT accidentally self-exempt.
const WEBHOOK_NS: &str = "capacity-admission";

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

/// Build the `cluster-allocation` singleton in enforce mode (so a non-exempt
/// over-budget pod is denied) carrying the given exclusion lists.
fn allocation_excluded(
    excluded_namespaces: Option<Vec<&str>>,
    excluded_priority_classes: Option<Vec<&str>>,
) -> Allocation {
    let mut a = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: Some(EnforcementMode::Enforce),
            excluded_namespaces: excluded_namespaces
                .map(|v| v.into_iter().map(String::from).collect()),
            excluded_priority_classes: excluded_priority_classes
                .map(|v| v.into_iter().map(String::from).collect()),
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    );
    a.status = Some(spec_allocation_status());
    a
}

/// A reflector store with the singleton applied (exclusion lists as given).
fn allocation_store_excluded(
    excluded_namespaces: Option<Vec<&str>>,
    excluded_priority_classes: Option<Vec<&str>>,
) -> Store<Allocation> {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    writer.apply_watcher_event(&watcher::Event::Apply(allocation_excluded(
        excluded_namespaces,
        excluded_priority_classes,
    )));
    store
}

/// Application state pinned to the fixture clock with a fresh metrics registry.
fn state_with(allocation: Store<Allocation>, webhook_ns: &str) -> AppState {
    state_with_metrics(allocation, Arc::new(Metrics::new()), webhook_ns)
}

/// Like [`state_with`], but the caller retains a handle to the metrics registry
/// so the exemption counter can be inspected after a decision.
fn state_with_metrics(
    allocation: Store<Allocation>,
    metrics: Arc<Metrics>,
    webhook_ns: &str,
) -> AppState {
    let now = fixture_now();
    AppState::with_clock(
        Arc::new(allocation),
        Arc::new(capacity_store()),
        Arc::new(move || now),
        metrics,
        webhook_ns.to_string(),
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

/// A pod with an optional `priorityClassName` (string-match only — no
/// PriorityClass resource resolution, R3). `Some("")` models an empty-string
/// priority class, which must never match.
fn pod_with_priority(cpu: &str, memory: &str, priority_class: Option<&str>) -> Pod {
    let mut pod = pod(cpu, memory);
    if let Some(spec) = pod.spec.as_mut() {
        spec.priority_class_name = priority_class.map(str::to_string);
    }
    pod
}

/// Serialise a pod into the AdmissionReview body shape the kube-apiserver sends,
/// scoped to `namespace`. All admissions here are CREATEs.
fn review_body_in(uid: &str, namespace: &str, object: &Pod) -> Bytes {
    let object = serde_json::to_value(object).unwrap();
    let review = serde_json::json!({
        "kind": "AdmissionReview",
        "apiVersion": "admission.k8s.io/v1",
        "request": {
            "uid": uid,
            "name": uid,
            "namespace": namespace,
            "kind": {"group": "", "version": "v1", "kind": "Pod"},
            "resource": {"group": "", "version": "v1", "resource": "pods"},
            "operation": "CREATE",
            "userInfo": {"username": "operator@example.com"},
            "object": object,
            "oldObject": serde_json::Value::Null,
            "dryRun": false,
        }
    });
    Bytes::from(serde_json::to_vec(&review).unwrap())
}

/// An over-budget (15 CPU → projected 85 > 80 ceiling) CREATE in `namespace`.
fn over_budget_body_in(uid: &str, namespace: &str) -> Bytes {
    review_body_in(uid, namespace, &pod("15", "10Gi"))
}

/// An over-budget CREATE in `namespace` carrying `priority_class`.
fn body_in_priority(uid: &str, namespace: &str, priority_class: Option<&str>) -> Bytes {
    review_body_in(
        uid,
        namespace,
        &pod_with_priority("15", "10Gi", priority_class),
    )
}

// ===========================================================================
// US1 / T007: namespace exclusion end-to-end through `handle()`
// ===========================================================================

#[tokio::test]
async fn us1_over_budget_pod_in_excluded_namespace_is_admitted() {
    // US1 AC1: an over-budget pod in an excluded namespace is admitted (exempt),
    // uid echoed, no warning. The Allocation is in enforce mode, so the exemption
    // — not the mode — is what admits it.
    let store = allocation_store_excluded(Some(vec!["monitoring"]), None);
    let resp = handle(
        over_budget_body_in("us1-exempt", "monitoring"),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(resp.allowed, "pod in excluded namespace is admitted");
    assert_eq!(resp.uid, "us1-exempt", "uid is echoed");
    assert!(
        resp.warnings.is_none(),
        "an exempt admit carries no warning"
    );
    assert_eq!(resp.result.code, 0, "no HTTP status on an allow");
}

#[tokio::test]
async fn us1_over_budget_pod_in_other_namespace_is_denied() {
    // US1 AC2: a non-excluded namespace is still budget-checked → denied 403.
    let store = allocation_store_excluded(Some(vec!["monitoring"]), None);
    let resp = handle(
        over_budget_body_in("us1-gated", "app-team-a"),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(
        !resp.allowed,
        "pod in a non-excluded namespace is budget-checked"
    );
    assert_eq!(resp.result.code, 403, "policy denial is a 403");
    assert!(
        resp.result.message.contains("CPU budget exceeded"),
        "denial carries the over-budget figures: {}",
        resp.result.message
    );
}

#[tokio::test]
async fn fr007_webhook_own_namespace_exempt_with_empty_config() {
    // FR-007: the webhook's own namespace is exempt even with both exclusion
    // lists empty (cold-start / unconfigured CRD). The apiserver namespaceSelector
    // is the cold-start defence-in-depth; once the Allocation is cached the
    // webhook layer self-exempts via check_exemption.
    let store = allocation_store_excluded(None, None);
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        over_budget_body_in("self-exempt", WEBHOOK_NS),
        &state_with_metrics(store, Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;

    assert!(
        resp.allowed,
        "the webhook's own namespace never self-gates (FR-007)"
    );
    assert!(resp.warnings.is_none(), "no warning on an exempt admit");

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="webhook_namespace"} 1"#),
        "webhook-ns exemption bumps the webhook_namespace counter: {text}"
    );
}

#[tokio::test]
async fn us1_namespace_exemption_increments_counter_not_verdicts() {
    // FR-008 / SC-003: an exempt decision bumps
    // capacity_admission_exemptions_total{reason="namespace"} and does NOT bump
    // the verdicts counter.
    let store = allocation_store_excluded(Some(vec!["monitoring"]), None);
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        over_budget_body_in("us1-counter", "monitoring"),
        &state_with_metrics(store, Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;
    assert!(resp.allowed);

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 1"#),
        "exempt decision bumps the namespace exemption counter: {text}"
    );
    assert!(
        text.contains(r#"capacity_admission_verdicts_total{resource="cpu",verdict="allow"} 0"#),
        "exempt decision does NOT bump the verdicts counter: {text}"
    );
}

// ===========================================================================
// US2 / T008: priority-class exclusion end-to-end
// ===========================================================================

#[tokio::test]
async fn us2_over_budget_pod_with_excluded_priority_class_admitted() {
    // US2 AC1: an over-budget pod with an excluded priority class is admitted
    // regardless of namespace.
    let store = allocation_store_excluded(None, Some(vec!["system-node-critical"]));
    let resp = handle(
        body_in_priority("us2-exempt", "app-team-a", Some("system-node-critical")),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(resp.allowed, "excluded priority class admits the pod");
    assert!(resp.warnings.is_none(), "no warning on an exempt admit");
}

#[tokio::test]
async fn us2_priority_class_exemption_increments_counter() {
    // FR-008: the priority_class reason counter increments (and only it does).
    let store = allocation_store_excluded(None, Some(vec!["system-node-critical"]));
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        body_in_priority("us2-counter", "app-team-a", Some("system-node-critical")),
        &state_with_metrics(store, Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;
    assert!(resp.allowed);

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="priority_class"} 1"#),
        "priority-class exemption bumps the priority_class counter: {text}"
    );
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 0"#),
        "priority-class match does not fire the namespace reason: {text}"
    );
}

#[tokio::test]
async fn us2_pod_with_no_priority_class_is_denied() {
    // US2 AC2: a pod with no priorityClassName is subject to the budget.
    let store = allocation_store_excluded(None, Some(vec!["system-node-critical"]));
    let resp = handle(
        over_budget_body_in("us2-none", "app-team-a"),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(!resp.allowed, "no priority class → budget-checked → denied");
    assert_eq!(resp.result.code, 403);
}

#[tokio::test]
async fn us2_pod_with_unlisted_priority_class_is_denied() {
    // Only an exact match exempts; a different priority class is budget-checked.
    let store = allocation_store_excluded(None, Some(vec!["system-node-critical"]));
    let resp = handle(
        body_in_priority("us2-gold", "app-team-a", Some("gold")),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(!resp.allowed, "unlisted priority class → denied");
    assert_eq!(resp.result.code, 403);
}

#[tokio::test]
async fn us2_pod_with_empty_string_priority_class_is_denied() {
    // Edge case: an empty-string priorityClassName must never match (absent == "").
    let store = allocation_store_excluded(None, Some(vec!["system-node-critical"]));
    let resp = handle(
        body_in_priority("us2-empty", "app-team-a", Some("")),
        &state_with(store, WEBHOOK_NS),
    )
    .await;

    assert!(!resp.allowed, "empty-string priority class → denied");
    assert_eq!(resp.result.code, 403);
}

// ===========================================================================
// US3 / T009: combined namespace + priority-class (OR) semantics
// ===========================================================================
//
// Both excludedNamespaces: ["kube-system"] and excludedPriorityClasses:
// ["system-node-critical"] configured. A pod matching EITHER is exempt; matching
// both counts once (first-match wins, namespace before priority class).

fn combined_store() -> Store<Allocation> {
    allocation_store_excluded(
        Some(vec!["kube-system"]),
        Some(vec!["system-node-critical"]),
    )
}

#[tokio::test]
async fn us3_priority_class_only_match_admits_as_priority_class() {
    // US3 AC1: pod with the excluded priority class in a non-excluded namespace.
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        body_in_priority("us3-pc", "app-team-a", Some("system-node-critical")),
        &state_with_metrics(combined_store(), Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;
    assert!(resp.allowed, "priority-class match → exempt");

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="priority_class"} 1"#),
        "pc-only match → priority_class reason: {text}"
    );
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 0"#),
        "pc-only match does not fire the namespace reason: {text}"
    );
}

#[tokio::test]
async fn us3_namespace_only_match_admits_as_namespace() {
    // US3 AC2: pod with no priority class in the excluded namespace.
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        over_budget_body_in("us3-ns", "kube-system"),
        &state_with_metrics(combined_store(), Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;
    assert!(resp.allowed, "namespace match → exempt");

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 1"#),
        "ns-only match → namespace reason: {text}"
    );
}

#[tokio::test]
async fn us3_neither_match_is_denied() {
    // US3 AC3: pod matching neither list is budget-checked → denied.
    let resp = handle(
        over_budget_body_in("us3-neither", "app-team-a"),
        &state_with(combined_store(), WEBHOOK_NS),
    )
    .await;
    assert!(!resp.allowed, "neither match → denied");
    assert_eq!(resp.result.code, 403);
}

#[tokio::test]
async fn us3_both_match_counts_once_as_namespace() {
    // US3 AC4: a pod matching BOTH lists is admitted ONCE (exemption is boolean,
    // not double-counted). First-match wins per data-model §3.2 order (webhook ns
    // → namespaces → priority classes), so the reason is "namespace" and the
    // namespace counter increments by exactly 1, not 2.
    let metrics = Arc::new(Metrics::new());
    let resp = handle(
        body_in_priority("us3-both", "kube-system", Some("system-node-critical")),
        &state_with_metrics(combined_store(), Arc::clone(&metrics), WEBHOOK_NS),
    )
    .await;
    assert!(resp.allowed, "either match → exempt");

    let text = metrics.render();
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 1"#),
        "both-match counts once as namespace: {text}"
    );
    assert!(
        text.contains(r#"capacity_admission_exemptions_total{reason="priority_class"} 0"#),
        "both-match does not also fire the priority_class reason: {text}"
    );
}
