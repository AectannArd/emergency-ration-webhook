//! Integration tests for capacity awareness (User Story 2, T024).
//!
//! Proves every admission decision is observable: structured log entries carry
//! every Logging Contract field (contracts/admission-webhook.md §Logging),
//! rejection messages carry actionable figures (SC-002), and the metrics
//! surface exposes verdict counters + capacity gauges (data-model.md §Metrics).

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util::parse_rfc3339;
use capacity_admission_webhook::webhook::handler::{AppState, handle};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::core::admission::Operation;
use kube::runtime::reflector::Store;
use kube::runtime::watcher;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

const GIB: i64 = 1024 * 1024 * 1024;
const FIXTURE_TIME: &str = "2026-07-26T00:00:00Z";

/// Allocation status: 100 CPU / 200 GiB allocatable, 80% budget → ceiling
/// 80 CPU / 160 GiB, currently allocated 70 CPU / 110 GiB.
fn allocation_status() -> AllocationStatus {
    AllocationStatus {
        allocated_cpu_milli: 70_000,
        allocated_memory_bytes: 110 * GIB,
        ceiling_cpu_milli: 80_000,
        ceiling_memory_bytes: 160 * GIB,
        utilization_percent_cpu: 0.875,
        utilization_percent_memory: 0.6875,
        last_updated: FIXTURE_TIME.to_string(),
    }
}

fn populated_allocation_store() -> Store<Allocation> {
    let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
    let mut allocation = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
        },
    );
    allocation.status = Some(allocation_status());
    writer.apply_watcher_event(&watcher::Event::Apply(allocation));
    store
}

/// Capacity store: 100 CPU / 200 GiB total allocatable (feeds the gauges).
fn populated_capacity_store() -> Store<ClusterCapacity> {
    let (store, mut writer) = kube::runtime::reflector::store::<ClusterCapacity>();
    let mut capacity = ClusterCapacity::new(
        CLUSTER_CAPACITY_NAME,
        ClusterCapacitySpec {
            node_selector: None,
        },
    );
    capacity.status = Some(ClusterCapacityStatus {
        total_allocatable_cpu_milli: 100_000,
        total_allocatable_memory_bytes: 200 * GIB,
        node_count: 2,
        last_updated: FIXTURE_TIME.to_string(),
        excluded_node_count: 0,
        excluded_by_unschedulable: 0,
        excluded_by_selector: 0,
    });
    writer.apply_watcher_event(&watcher::Event::Apply(capacity));
    store
}

fn state(metrics: Arc<Metrics>) -> AppState {
    let now = parse_rfc3339(FIXTURE_TIME).unwrap();
    AppState::with_clock(
        Arc::new(populated_allocation_store()),
        Arc::new(populated_capacity_store()),
        Arc::new(move || now),
        metrics,
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

// ---------------------------------------------------------------------------
// Structured-log capture
// ---------------------------------------------------------------------------

/// One captured tracing event: its level + structured fields (quotes stripped).
type Fields = HashMap<String, String>;

#[derive(Default)]
struct CapturedEvents(Mutex<Vec<Fields>>);

struct CaptureLayer(Arc<CapturedEvents>);

impl<S> Layer<S> for CaptureLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = FieldCollector(HashMap::new());
        event.record(&mut fields);
        fields
            .0
            .insert("level".to_string(), event.metadata().level().to_string());
        self.0.0.lock().unwrap().push(fields.0);
    }
}

/// Visit impl that records every field's value, stripping the Debug quotes that
/// `tracing` adds around string literals so assertions are exact.
struct FieldCollector(Fields);

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let raw = format!("{value:?}");
        // tracing wraps string-literal fields in Debug quotes; strip a matching
        // pair so string values compare exactly. Numeric Debug output is left as-is.
        let cleaned = if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
            raw[1..raw.len() - 1].to_string()
        } else {
            raw
        };
        self.0.insert(field.name().to_string(), cleaned);
    }
}

/// Install a capturing subscriber for the current thread, returning the events
/// and a guard whose lifetime scopes the capture. `set_default` is thread-local
/// and the single-threaded tokio test runtime runs `handle` on this thread, so
/// the events it emits are recorded. Events emit synchronously, so they are
/// present by the time `handle` returns — no delay is needed.
fn capture() -> (Arc<CapturedEvents>, tracing::subscriber::DefaultGuard) {
    let events = Arc::new(CapturedEvents::default());
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(Arc::clone(&events)));
    let guard = tracing::subscriber::set_default(subscriber);
    (events, guard)
}

/// Find the first captured event whose `resource_type` field matches.
fn event_for_resource<'a>(events: &'a [Fields], resource_type: &str) -> &'a Fields {
    events
        .iter()
        .find(|e| e.get("resource_type").is_some_and(|r| r == resource_type))
        .unwrap_or_else(|| panic!("no captured event for resource_type={resource_type}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admit_logs_every_contract_field() {
    let (events, _guard) = capture();
    let state = state(Arc::new(Metrics::new()));
    // 5 CPU / 40 GiB → projected 75 / 150, both under ceiling → admit.
    handle(
        review_body("admit", Operation::Create, &pod("5", "40Gi")),
        &state,
    )
    .await;

    let captured = events.0.lock().unwrap();
    let cpu = event_for_resource(&captured, "cpu");
    assert_eq!(cpu.get("decision"), Some(&"allow".to_string()));
    assert_eq!(cpu.get("operation"), Some(&"CREATE".to_string()));
    assert_eq!(cpu.get("workload"), Some(&"default/admit".to_string()));
    assert_eq!(cpu.get("resource_type"), Some(&"cpu".to_string()));
    assert_eq!(cpu.get("allocated"), Some(&"70000".to_string()));
    assert_eq!(cpu.get("requested"), Some(&"5000".to_string()));
    assert_eq!(cpu.get("projected"), Some(&"75000".to_string()));
    assert_eq!(cpu.get("ceiling"), Some(&"80000".to_string()));
    assert_eq!(cpu.get("budget_percent"), Some(&"80".to_string()));
    assert_eq!(cpu.get("freshness_seconds"), Some(&"0".to_string()));
    assert!(
        cpu.get("latency_ms").is_some(),
        "latency_ms must be present"
    );
    assert_eq!(cpu.get("level"), Some(&"INFO".to_string()));
}

#[tokio::test]
async fn deny_logs_reason_and_figures() {
    let (events, _guard) = capture();
    let state = state(Arc::new(Metrics::new()));
    // 15 CPU → projected 85 > 80 → CPU over budget.
    handle(
        review_body("deny", Operation::Create, &pod("15", "10Gi")),
        &state,
    )
    .await;

    let captured = events.0.lock().unwrap();
    let cpu = event_for_resource(&captured, "cpu");
    assert_eq!(cpu.get("decision"), Some(&"deny".to_string()));
    assert_eq!(cpu.get("reason"), Some(&"cpu_over_budget".to_string()));
    assert_eq!(cpu.get("projected"), Some(&"85000".to_string()));
    assert_eq!(cpu.get("ceiling"), Some(&"80000".to_string()));
    assert_eq!(cpu.get("level"), Some(&"WARN".to_string()));
    // Memory was within budget on this deny → its event is still WARN (the
    // decision was a deny) but carries the memory figures.
    let memory = event_for_resource(&captured, "memory");
    assert_eq!(memory.get("decision"), Some(&"deny".to_string()));
}

#[tokio::test]
async fn rejection_message_carries_actionable_figures() {
    let state = state(Arc::new(Metrics::new()));
    let resp = handle(
        review_body("deny", Operation::Create, &pod("15", "10Gi")),
        &state,
    )
    .await;
    assert!(!resp.allowed);
    let message = &resp.result.message;
    // SC-002: a workload owner can act on the message alone.
    assert!(message.contains("CPU budget exceeded"));
    assert!(message.contains("allocated 70000m"));
    assert!(message.contains("requested 15000m"));
    assert!(message.contains("projected 85000m"));
    assert!(message.contains("ceiling 80000m"));
}

#[tokio::test]
async fn metrics_endpoint_exposes_verdicts_and_gauges() {
    let metrics = Arc::new(Metrics::new());
    let state = state(Arc::clone(&metrics));
    // One admit (cpu+memory allow), one cpu-over deny.
    handle(
        review_body("a", Operation::Create, &pod("5", "40Gi")),
        &state,
    )
    .await;
    handle(
        review_body("d", Operation::Create, &pod("15", "10Gi")),
        &state,
    )
    .await;

    let text = metrics.render();
    // Verdict counters: 1 cpu allow, 1 cpu deny, 2 memory allow (admit + the
    // deny whose memory was within budget).
    assert!(
        text.contains(r#"verdicts_total{resource="cpu",verdict="allow"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"verdicts_total{resource="cpu",verdict="deny"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"verdicts_total{resource="memory",verdict="allow"} 2"#),
        "{text}"
    );
    // Capacity gauges reflect the last decision's view (SC-003).
    assert!(
        text.contains(r#"current_allocation{resource="cpu"} 70000"#),
        "{text}"
    );
    assert!(text.contains(r#"ceiling{resource="cpu"} 80000"#), "{text}");
    assert!(
        text.contains(r#"total_allocatable{resource="cpu"} 100000"#),
        "{text}"
    );
    // 70000 / 80000 = 0.875.
    assert!(
        text.contains(r#"allocation_ratio{resource="cpu"} 0.875"#),
        "{text}"
    );
    // Decision latency was observed.
    assert!(text.contains("decision_duration_seconds_count 2"), "{text}");
}
