//! US2 active-degradation scenarios S9-S11 (spec-005, research R11).
//!
//! Each scenario deliberately breaks one fail-closed precondition of the running
//! webhook, asserts the apiserver rejects a pod submission while that
//! precondition is broken, then restores the webhook to health so the next
//! scenario starts from a known-good baseline. Like the enforcement scenarios
//! they are NOT unit-testable — they run against a real cluster (Constitution
//! Principle VI: the tool IS the integration coverage).
//!
//! Each scenario restores health unconditionally before returning (the restore is
//! best-effort and logged on failure); a restore that times out does not flip a
//! passed assertion, but it will cascade into the next scenario failing, which
//! surfaces the problem to the operator.

use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams};

use capacity_admission_webhook::crd::{
    Allocation, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity,
};
use capacity_admission_webhook::time_util;

use super::enforcement::{create_pod, delete_pod};
use super::{ScenarioGroup, ScenarioResult, ScenarioStatus};

/// Namespace the webhook pods run in (the singletons are cluster-scoped).
const NAMESPACE: &str = "capacity-admission";
/// Label selector matching every webhook pod (the Deployment's replica set).
const APP_LABEL: &str = "app=capacity-admission-webhook";
/// Freshness threshold the webhook enforces. Matches the
/// `--capacity-freshness-timeout-secs=30` arg in `deploy/deployment.yaml`, which
/// is the manifest the verify tool applies — so within a run this is exact.
const FRESHNESS_THRESHOLD_SECS: i64 = 30;
/// How far S11 backdates `Allocation.status.lastUpdated` (well beyond the
/// threshold → unambiguously stale).
const STALE_BACKDATE_SECS: i64 = 120;
/// Window each scenario probes for the degraded rejection before giving up.
const PROBE_WINDOW: Duration = Duration::from_secs(30);
/// How long a restore may take (pods recreated / singletons repopulated /
/// `lastUpdated` refreshed) before it is treated as a failed restore.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(60);
/// Sleep between probe attempts (lets the degradation propagate to the webhook's
/// reflector caches and the apiserver's endpoint set).
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// Run the three degradation scenarios sequentially, returning their results.
///
/// Each scenario restores the webhook to health before returning, so they are
/// safe to run back-to-back.
pub async fn run(client: &Client) -> Vec<ScenarioResult> {
    vec![
        timed("S9", "webhook pods killed → admission rejected", s9(client)).await,
        timed(
            "S10",
            "CRD instances deleted → admission rejected",
            s10(client),
        )
        .await,
        timed("S11", "stale capacity → admission rejected", s11(client)).await,
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
        group: ScenarioGroup::Degradation,
        status,
        duration: start.elapsed(),
        detail,
    }
}

// ---- S9: kill every webhook pod → apiserver fails closed (failurePolicy: Fail) ----

async fn s9(client: &Client) -> Result<String, String> {
    // Degrade: delete every webhook pod. While the Deployment recreates them the
    // Service has no ready endpoints, so the apiserver cannot forward the
    // admission call and (failurePolicy: Fail) rejects the create itself.
    let pods: Api<Pod> = Api::namespaced(client.clone(), NAMESPACE);
    pods.delete_collection(
        &DeleteParams::default(),
        &ListParams::default().labels(APP_LABEL),
    )
    .await
    .map_err(|e| format!("deleting webhook pods: {e}"))?;

    let outcome = probe_rejection(client, "erw-verify-s9", expect_unreachable).await;
    let _ = delete_pod(client, "erw-verify-s9").await;

    // Restore: wait for the Deployment to recreate pods and the capacity state to
    // be repopulated, so the next scenario starts from a healthy baseline.
    restore_readiness(client).await;

    outcome
}

// ---- S10: delete the capacity singletons → CapacityDataMissing fail-closed ----

async fn s10(client: &Client) -> Result<String, String> {
    // Degrade: delete both capacity singletons. The webhook's reflector drops
    // them, so the next admission hits the CapacityDataMissing path. The
    // controllers auto-recreate both within a reconcile or two (spec-003).
    delete_if_present(
        &Api::<Allocation>::all(client.clone()),
        CLUSTER_ALLOCATION_NAME,
    )
    .await;
    delete_if_present(
        &Api::<ClusterCapacity>::all(client.clone()),
        CLUSTER_CAPACITY_NAME,
    )
    .await;

    let outcome = probe_rejection(client, "erw-verify-s10", expect_capacity_unavailable).await;
    let _ = delete_pod(client, "erw-verify-s10").await;

    // Restore: the controllers recreate the singletons; wait for the ceiling to
    // be repopulated (non-zero) before continuing.
    restore_readiness(client).await;

    outcome
}

// ---- S11: backdate lastUpdated → CapacityDataStale fail-closed ----

async fn s11(client: &Client) -> Result<String, String> {
    let allocs: Api<Allocation> = Api::all(client.clone());

    // The Allocation Controller rewrites a fresh lastUpdated every ~2s, so we
    // re-backdate on every probe attempt and race to submit before the next
    // reconcile overwrites us.
    let deadline = Instant::now() + PROBE_WINDOW;
    let mut outcome: Result<String, String> =
        Err("capacity data never read as stale: pod was admitted for the whole window".to_string());
    while Instant::now() < deadline {
        if let Err(e) = backdate_last_updated(&allocs).await {
            outcome = Err(e);
            break;
        }
        match create_pod(client, "erw-verify-s11", "10m", "10Mi").await {
            Ok(_) => {
                // The webhook has not yet observed the stale value (watch lag) or
                // the controller already overwrote it — clean up and retry.
                let _ = delete_pod(client, "erw-verify-s11").await;
                tokio::time::sleep(PROBE_INTERVAL).await;
            }
            Err(e) => {
                outcome = classify_rejection(&e, expect_stale);
                break;
            }
        }
    }
    let _ = delete_pod(client, "erw-verify-s11").await;

    // Restore: wait for the Allocation Controller to rewrite a fresh lastUpdated.
    if let Err(e) = wait_for_fresh_last_updated(&allocs).await {
        tracing::warn!(error = %e, "S11 restore: Allocation lastUpdated did not become fresh");
    }

    outcome
}

// ===================== probe + classification helpers =====================

/// Whether a rejection message matches what a scenario expected.
#[derive(Clone, Copy)]
enum ProbeKind {
    Expected,
    Unexpected,
}

/// Turn a create-pod error into a scenario outcome. `classify` decides whether
/// the rejection reason is the one the scenario was testing for.
fn classify_rejection(
    err: &kube::Error,
    classify: fn(&str) -> ProbeKind,
) -> Result<String, String> {
    match rejection_detail(err) {
        Some(detail) => match classify(&detail) {
            ProbeKind::Expected => Ok(format!("admission rejected: {detail}")),
            ProbeKind::Unexpected => Err(format!(
                "rejected, but not for the expected reason: {detail}"
            )),
        },
        None => Err(format!("unexpected non-admission error: {err}")),
    }
}

/// S9: the webhook itself is unreachable, so ANY apiserver admission rejection
/// counts (failurePolicy: Fail made the apiserver reject the create).
fn expect_unreachable(_detail: &str) -> ProbeKind {
    ProbeKind::Expected
}

/// S10: the webhook fail-closes with "capacity data unavailable: ... not
/// initialised" when a singleton is missing.
fn expect_capacity_unavailable(detail: &str) -> ProbeKind {
    if detail.contains("capacity data unavailable") {
        ProbeKind::Expected
    } else {
        ProbeKind::Unexpected
    }
}

/// S11: the stale path reads "capacity data unavailable: last refresh Ns ago
/// exceeds 30s threshold".
fn expect_stale(detail: &str) -> ProbeKind {
    if detail.contains("exceeds") || detail.contains("threshold") {
        ProbeKind::Expected
    } else {
        ProbeKind::Unexpected
    }
}

/// Probe admission repeatedly across [`PROBE_WINDOW`]. On each attempt create a
/// small test pod; classify the result with `classify`. The first rejection wins
/// (matched → `Ok`, other reason → `Err`); a non-admission error aborts
/// immediately. If every attempt is admitted the window expires with an `Err`.
async fn probe_rejection(
    client: &Client,
    pod_name: &str,
    classify: fn(&str) -> ProbeKind,
) -> Result<String, String> {
    let deadline = Instant::now() + PROBE_WINDOW;
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "degradation never took effect: pod was admitted for the whole {}s window",
                PROBE_WINDOW.as_secs()
            ));
        }
        match create_pod(client, pod_name, "10m", "10Mi").await {
            Ok(_) => {
                let _ = delete_pod(client, pod_name).await;
                tokio::time::sleep(PROBE_INTERVAL).await;
            }
            Err(e) => return classify_rejection(&e, classify),
        }
    }
}

/// First line of an apiserver admission-rejection message, or `None` when the
/// error is not an admission decision (e.g. a transport failure to the
/// apiserver). Mirrors enforcement's `denial_message` but signals non-rejections.
fn rejection_detail(err: &kube::Error) -> Option<String> {
    match err {
        kube::Error::Api(status) => Some(
            status
                .message
                .lines()
                .next()
                .unwrap_or("(no message)")
                .to_string(),
        ),
        _ => None,
    }
}

// ===================== degrade + restore helpers =====================

/// Delete a cluster-scoped singleton, treating 404 (already gone) as success.
/// Best-effort: a failure is logged but does not abort the scenario — the probe
/// that follows will simply fail to observe the degradation.
async fn delete_if_present<K>(api: &Api<K>, name: &str)
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => tracing::info!(%name, "degradation: deleted singleton"),
        Err(kube::Error::Api(status)) if status.code == 404 => {}
        Err(e) => tracing::warn!(%name, error = %e, "degradation: failed to delete singleton"),
    }
}

/// Backdate the Allocation `status.lastUpdated` past the freshness threshold,
/// wrapped in the `{"status": ...}` envelope a `/status` merge patch requires
/// (the kube-rs Patch::Merge status gotcha — a bare status object is a silent
/// no-op on the subresource).
async fn backdate_last_updated(api: &Api<Allocation>) -> Result<(), String> {
    let stale = time_util::rfc3339_from_unix(time_util::now_unix() - STALE_BACKDATE_SECS);
    let patch = serde_json::json!({ "status": { "lastUpdated": stale } });
    api.patch_status(
        CLUSTER_ALLOCATION_NAME,
        &PatchParams::default(),
        &Patch::Merge(&patch),
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("backdating Allocation lastUpdated: {e}"))
}

/// Wait for the webhook to return to a healthy baseline (≥1 pod Ready + non-zero
/// Allocation ceiling) after a scenario induced degradation. Self-contained
/// (mirrors `setup::wait_for_readiness`) so this module compiles both inside the
/// binary and `#[path]`-included by the report unit tests. Best-effort.
async fn restore_readiness(client: &Client) {
    if let Err(e) = wait_for_readiness(client, RESTORE_TIMEOUT).await {
        tracing::warn!(
            error = %e,
            "degradation restore: webhook did not return to Ready + non-zero ceiling"
        );
    }
}

/// Poll until at least one webhook pod is `Running` with ready containers, then
/// until the Allocation `ceilingCpuMilli` is non-zero again (supply repopulated).
async fn wait_for_readiness(client: &Client, timeout: Duration) -> Result<(), String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), NAMESPACE);
    let lp = ListParams::default().labels(APP_LABEL);
    let allocs: Api<Allocation> = Api::all(client.clone());

    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err("webhook did not return to Ready + non-zero ceiling".into());
        }
        let pods_ready = pods
            .list(&lp)
            .await
            .map(|list| list.items.iter().any(pod_ready))
            .unwrap_or(false);
        if pods_ready
            && let Ok(allocation) = allocs.get(CLUSTER_ALLOCATION_NAME).await
            && let Some(status) = &allocation.status
            && status.ceiling_cpu_milli > 0
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Whether a pod is fully ready: `Running` phase + every container ready.
fn pod_ready(pod: &Pod) -> bool {
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let running = status.phase.as_deref() == Some("Running");
    let containers_ready = status
        .container_statuses
        .as_ref()
        .map(|cs| !cs.is_empty() && cs.iter().all(|c| c.ready))
        .unwrap_or(false);
    running && containers_ready
}

/// Wait for the Allocation Controller to rewrite a fresh `lastUpdated` (age at or
/// below the freshness threshold). The controller recomputes on a ~2s tick, so
/// this resolves within a few seconds.
async fn wait_for_fresh_last_updated(api: &Api<Allocation>) -> Result<(), String> {
    let deadline = Instant::now() + RESTORE_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err("Allocation lastUpdated never became fresh".into());
        }
        if let Ok(allocation) = api.get(CLUSTER_ALLOCATION_NAME).await
            && let Some(status) = &allocation.status
            && let Some(refreshed) = time_util::parse_rfc3339(&status.last_updated)
        {
            let age = time_util::now_unix().saturating_sub(refreshed);
            if age <= FRESHNESS_THRESHOLD_SECS {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
