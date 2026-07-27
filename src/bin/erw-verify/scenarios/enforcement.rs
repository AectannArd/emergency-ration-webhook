//! US1 enforcement scenarios S1-S8 (spec-005, research R5-R10).
//!
//! Each scenario exercises one aspect of the admission webhook's enforcement
//! contract against the live cluster and returns a [`ScenarioResult`]. They are
//! NOT unit-testable — they run against a real cluster (Constitution Principle
//! VI: the tool IS the integration coverage). Scenarios S3-S6 patch the shared
//! Allocation singleton and restore it afterwards, so they MUST run sequentially.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::{Container, Node, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, ObjectMeta, Patch, PatchParams, PostParams};

use capacity_admission_webhook::crd::{
    Allocation, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity, EnforcementMode,
};
use capacity_admission_webhook::resources::quantity::{parse_cpu, parse_memory};

use super::{ScenarioGroup, ScenarioResult, ScenarioStatus};

/// The budget the cluster is expected to start at (controller-seeded default)
/// and that S3-S6 restore to after mutating it.
const DEFAULT_BUDGET_PERCENT: i32 = 80;
const NAMESPACE: &str = "capacity-admission";
/// Namespace the test pods land in (must be admission-covered, i.e. not the
/// webhook's own excluded namespace).
const POD_NAMESPACE: &str = "default";
const POD_IMAGE: &str = "nginx";

/// Run all eight enforcement scenarios sequentially, returning their results.
pub async fn run(client: &Client) -> Vec<ScenarioResult> {
    vec![
        timed("S1", "within-budget pod admitted", s1(client)).await,
        timed("S2", "over-budget pod denied", s2(client)).await,
        timed("S3", "budgetPercent 0 (circuit-breaker)", s3(client)).await,
        timed(
            "S4",
            "budgetPercent 100 (physical overcommit guard)",
            s4(client),
        )
        .await,
        timed("S5", "runtime budget adjustment (no restart)", s5(client)).await,
        timed("S6", "dry-run mode (admit + warning)", s6(client)).await,
        timed(
            "S7",
            "capacity tracking accuracy (CRD vs nodes)",
            s7(client),
        )
        .await,
        timed("S8", "metrics + health endpoints respond", s8(client)).await,
    ]
}

/// Time an async scenario body and wrap its outcome in a [`ScenarioResult`].
async fn timed(
    id: &str,
    name: &str,
    body: impl std::future::Future<Output = Result<String, String>>,
) -> ScenarioResult {
    let start = Instant::now();
    let (status, detail) = match body.await {
        Ok(detail) => (ScenarioStatus::Pass, detail),
        Err(detail) => (ScenarioStatus::Fail, detail),
    };
    ScenarioResult {
        id: id.into(),
        name: name.into(),
        group: ScenarioGroup::Enforcement,
        status,
        duration: start.elapsed(),
        detail,
    }
}

// ---- S1: a small pod is admitted (research R6) ----

async fn s1(client: &Client) -> Result<String, String> {
    let name = "erw-verify-s1";
    match create_pod(client, name, "10m", "10Mi").await {
        Ok(_) => {
            let _ = delete_pod(client, name).await;
            Ok(format!("pod {POD_NAMESPACE}/{name} admitted"))
        }
        Err(e) => Err(format!("expected pod admitted; apiserver returned: {e}")),
    }
}

// ---- S2: an over-budget pod is denied with HTTP 403 (research R6) ----

async fn s2(client: &Client) -> Result<String, String> {
    let name = "erw-verify-s2";
    match create_pod(client, name, "999", "999Gi").await {
        Ok(_) => {
            let _ = delete_pod(client, name).await;
            Err("expected pod denied (HTTP 403) but it was admitted".into())
        }
        Err(e) if is_denied_403(&e) => {
            let msg = denial_message(&e);
            Ok(format!("pod denied with HTTP 403: {msg}"))
        }
        Err(e) => Err(format!("expected HTTP 403 denial; got: {e}")),
    }
}

// ---- S3: budgetPercent 0 rejects everything (circuit-breaker) (research R7) ----

async fn s3(client: &Client) -> Result<String, String> {
    apply_budget(client, 0).await?;
    let denied = matches!(
        create_pod(client, "erw-verify-s3", "10m", "10Mi").await,
        Err(e) if is_denied_403(&e)
    );
    let _ = delete_pod(client, "erw-verify-s3").await;
    restore_budget(client).await;
    if denied {
        Ok("budgetPercent=0: small pod denied (ceiling is 0 → circuit-breaker)".into())
    } else {
        Err("expected denial at budgetPercent=0 but the pod was admitted".into())
    }
}

// ---- S4: budgetPercent 100 denies only genuine overcommit (research R7) ----

async fn s4(client: &Client) -> Result<String, String> {
    apply_budget(client, 100).await?;

    // A request far beyond physical capacity must still be denied.
    let over_denied = matches!(
        create_pod(client, "erw-verify-s4-over", "99999", "99999Gi").await,
        Err(e) if is_denied_403(&e)
    );
    let _ = delete_pod(client, "erw-verify-s4-over").await;

    // A small request within physical capacity must be admitted.
    let within_admitted = create_pod(client, "erw-verify-s4-ok", "10m", "10Mi")
        .await
        .is_ok();
    let _ = delete_pod(client, "erw-verify-s4-ok").await;

    restore_budget(client).await;

    if over_denied && within_admitted {
        Ok("budgetPercent=100: over-physical denied, within-physical admitted".into())
    } else {
        Err(format!(
            "budgetPercent=100 guard failed (over denied={over_denied}, within admitted={within_admitted})"
        ))
    }
}

// ---- S5: runtime budget patch takes effect without a restart (research R7) ----

async fn s5(client: &Client) -> Result<String, String> {
    let name = "erw-verify-s5";

    // Budget 0 → the test pod is denied.
    apply_budget(client, 0).await?;
    let denied_at_zero =
        matches!(create_pod(client, name, "10m", "10Mi").await, Err(e) if is_denied_403(&e));
    let _ = delete_pod(client, name).await;

    // Budget 80 → the SAME pod is admitted, with no webhook restart in between.
    apply_budget(client, DEFAULT_BUDGET_PERCENT).await?;
    let admitted_at_80 = create_pod(client, name, "10m", "10Mi").await.is_ok();
    let _ = delete_pod(client, name).await;

    // Budget is left at the default.
    if denied_at_zero && admitted_at_80 {
        Ok("runtime budget patch (0→80) took effect without a webhook restart".into())
    } else {
        Err(format!(
            "runtime adjust failed (denied@0={denied_at_zero}, admitted@80={admitted_at_80})"
        ))
    }
}

// ---- S6: dry-run mode admits over-budget pods + warns (research R8) ----

async fn s6(client: &Client) -> Result<String, String> {
    let name = "erw-verify-s6";

    patch_enforcement(client, EnforcementMode::DryRun).await?;
    let before = dry_run_deny_count(client).await.unwrap_or(0);

    // The webhook reads enforcementMode from its Allocation reflector, so the
    // mode change takes ~1 s to propagate. In dry-run an over-budget pod is
    // ADMITTED; retry briefly until that is observed (a 403 means the webhook is
    // still enforcing). The warning is not observable via the create response, so
    // the metrics counter is the reliable signal the dry-run path fired.
    let mut admitted = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match create_pod(client, name, "999", "999Gi").await {
            Ok(_) => {
                let _ = delete_pod(client, name).await;
                admitted = true;
                break;
            }
            Err(e) if is_denied_403(&e) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                let _ = delete_pod(client, name).await;
                return Err(format!("dry-run pod create: unexpected error {e}"));
            }
        }
    }

    let after = dry_run_deny_count(client).await.unwrap_or(0);
    patch_enforcement(client, EnforcementMode::Enforce).await?;

    if admitted && after > before {
        Ok(format!(
            "dry-run: over-budget pod admitted; dry_run_deny counter {before}→{after}"
        ))
    } else {
        Err(format!(
            "dry-run failed (over-budget admitted={admitted}, dry_run_deny {before}→{after})"
        ))
    }
}

// ---- S7: ClusterCapacity status matches an independent node sum (research R9) ----

async fn s7(client: &Client) -> Result<String, String> {
    let cap_api: Api<ClusterCapacity> = Api::all(client.clone());
    let cc = cap_api
        .get(CLUSTER_CAPACITY_NAME)
        .await
        .map_err(|e| format!("getting ClusterCapacity: {e}"))?;
    let status = cc
        .status
        .clone()
        .ok_or_else(|| "ClusterCapacity has no status yet".to_string())?;

    let node_api: Api<Node> = Api::all(client.clone());
    let nodes = node_api
        .list(&ListParams::default())
        .await
        .map_err(|e| format!("listing nodes: {e}"))?;

    let (mut cpu, mut mem) = (0i64, 0i64);
    for node in &nodes.items {
        let Some(alloc) = node.status.as_ref().and_then(|s| s.allocatable.as_ref()) else {
            continue;
        };
        if let Some(q) = alloc.get("cpu") {
            cpu += parse_cpu(&q.0).unwrap_or(0);
        }
        if let Some(q) = alloc.get("memory") {
            mem += parse_memory(&q.0).unwrap_or(0);
        }
    }

    let cpu_ok = status.total_allocatable_cpu_milli == cpu;
    let mem_ok = status.total_allocatable_memory_bytes == mem;
    let count_ok = status.node_count == nodes.items.len() as i32;

    if cpu_ok && mem_ok && count_ok {
        Ok(format!(
            "CRD matches nodes: cpu={cpu}m, memory={mem} bytes, {} nodes",
            status.node_count
        ))
    } else {
        Err(format!(
            "mismatch (cpu CRD={}m nodes={}m ok={cpu_ok}; mem CRD={} bytes nodes={} bytes ok={mem_ok}; nodes CRD={} actual={} ok={count_ok})",
            status.total_allocatable_cpu_milli,
            cpu,
            status.total_allocatable_memory_bytes,
            mem,
            status.node_count,
            nodes.items.len()
        ))
    }
}

// ---- S8: /healthz and /metrics respond via the API proxy (research R10) ----

async fn s8(client: &Client) -> Result<String, String> {
    let health = proxy_get(client, "healthz")
        .await
        .map_err(|e| format!("/healthz: {e}"))?;
    if !health.trim().eq_ignore_ascii_case("ok") {
        return Err(format!("/healthz returned {health:?}, expected \"ok\""));
    }
    let metrics = proxy_get(client, "metrics")
        .await
        .map_err(|e| format!("/metrics: {e}"))?;
    if !metrics.contains("capacity_admission_verdicts_total") {
        return Err("/metrics is missing capacity_admission_verdicts_total".into());
    }
    Ok("/healthz=ok; /metrics exposes capacity_admission_verdicts_total".into())
}

// ===================== helpers =====================

/// Whether a kube error is the webhook's HTTP 403 over-budget rejection.
fn is_denied_403(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 403)
}

/// Extract the first line of the rejection message (the per-resource budget line).
fn denial_message(err: &kube::Error) -> String {
    match err {
        kube::Error::Api(status) => status
            .message
            .lines()
            .next()
            .unwrap_or("(no message)")
            .to_string(),
        _ => err.to_string(),
    }
}

/// Create a test pod with explicit resource requests in the default namespace.
async fn create_pod(
    client: &Client,
    name: &str,
    cpu: &str,
    memory: &str,
) -> Result<(), kube::Error> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), POD_NAMESPACE);
    pods.create(&PostParams::default(), &test_pod(name, cpu, memory))
        .await
        .map(|_| ())
}

/// Delete a test pod, treating a 404 (already gone) as success.
async fn delete_pod(client: &Client, name: &str) -> Result<(), kube::Error> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), POD_NAMESPACE);
    match pods.delete(name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(e) => Err(e),
    }
}

/// Build a minimal Pod with `restartPolicy: Never` and explicit requests.
fn test_pod(name: &str, cpu: &str, memory: &str) -> Pod {
    let mut requests = BTreeMap::new();
    requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
    requests.insert("memory".to_string(), Quantity(memory.to_string()));
    Pod {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some(POD_NAMESPACE.into()),
            ..Default::default()
        },
        spec: Some(PodSpec {
            restart_policy: Some("Never".into()),
            containers: vec![Container {
                name: "test".into(),
                image: Some(POD_IMAGE.into()),
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

/// Patch `spec.budgetPercent` (R7 merge patch) AND wait until the Allocation
/// Controller has recomputed the ceiling for the new budget. The webhook reads the
/// ceiling from the Allocation *status* via its reflector — patching the spec and
/// testing immediately would race the controller's recompute (≤2 s tick) and read
/// the stale ceiling. `expected == floor(total_cpu * budget / 100)`, matching the
/// webhook's own ceiling computation.
async fn apply_budget(client: &Client, budget: i32) -> Result<(), String> {
    let alloc_api: Api<Allocation> = Api::all(client.clone());
    let patch = serde_json::json!({ "spec": { "budgetPercent": budget } });
    alloc_api
        .patch(
            CLUSTER_ALLOCATION_NAME,
            &PatchParams::default(),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(|e| format!("patching budgetPercent={budget}: {e}"))?;

    let total_cpu = Api::<ClusterCapacity>::all(client.clone())
        .get(CLUSTER_CAPACITY_NAME)
        .await
        .map_err(|e| format!("reading ClusterCapacity supply: {e}"))?
        .status
        .as_ref()
        .map(|s| s.total_allocatable_cpu_milli)
        .unwrap_or(0);
    let expected = total_cpu.saturating_mul(budget as i64) / 100;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "budgetPercent={budget} did not propagate: ceiling never reached {expected}m"
            ));
        }
        if let Ok(a) = alloc_api.get(CLUSTER_ALLOCATION_NAME).await
            && let Some(status) = &a.status
            && status.ceiling_cpu_milli == expected
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Restore the budget to the controller-seeded default and wait for the ceiling
/// to settle, so the next scenario starts from a known baseline.
async fn restore_budget(client: &Client) {
    let _ = apply_budget(client, DEFAULT_BUDGET_PERCENT).await;
}

/// Patch the Allocation `spec.enforcementMode` (R7 merge patch).
async fn patch_enforcement(client: &Client, mode: EnforcementMode) -> Result<(), String> {
    let api: Api<Allocation> = Api::all(client.clone());
    let patch = serde_json::json!({ "spec": { "enforcementMode": mode } });
    api.patch(
        CLUSTER_ALLOCATION_NAME,
        &PatchParams::default(),
        &Patch::Merge(&patch),
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("patching enforcementMode={:?}: {e}", mode))
}

/// GET `{path}` on the webhook's metrics port via the Kubernetes API proxy.
async fn proxy_get(client: &Client, path: &str) -> Result<String, String> {
    let uri = format!(
        "/api/v1/namespaces/{NAMESPACE}/services/capacity-admission-webhook:metrics/proxy/{path}"
    );
    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri(uri)
        .body(Vec::new())
        .map_err(|e| format!("building proxy request: {e}"))?;
    client
        .request_text(req)
        .await
        .map_err(|e| format!("proxy GET /{path}: {e}"))
}

/// Sum the `capacity_admission_verdicts_total{verdict="dry_run_deny"}` samples
/// scraped from `/metrics`.
async fn dry_run_deny_count(client: &Client) -> Result<u64, String> {
    let metrics = proxy_get(client, "metrics").await?;
    let mut total: u64 = 0;
    for line in metrics.lines() {
        if line.starts_with("capacity_admission_verdicts_total{")
            && line.contains(r#"verdict="dry_run_deny""#)
            && let Some(value) = line.split_whitespace().next_back()
        {
            total += value.parse::<u64>().unwrap_or(0);
        }
    }
    Ok(total)
}
