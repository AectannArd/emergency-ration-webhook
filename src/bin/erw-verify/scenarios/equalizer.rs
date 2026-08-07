//! spec-013 multi-cluster equalizer verification scenarios E1-E5 (FR-015, T046).
//!
//! These scenarios validate the equalizer's cross-cluster orchestration against
//! REAL clusters — the one piece of spec-013 not covered by the unit (algorithm)
//! or mocked-integration (reconcile) suites. They are the heaviest verification
//! in the tool: they require a multi-cluster fixture (the home cluster, where the
//! webhook + equalizer run, plus ≥1 additional target cluster, each with the
//! webhook installed). Like the enforcement/degradation scenarios they are NOT
//! unit-testable — they exercise a live fleet (Constitution Principle VI: the
//! tool IS the integration coverage). Only the pure helpers are unit-tested.
//!
//! **Self-contained.** Like its sibling scenario modules, this file depends only
//! on the library crate and its `scenarios` siblings (`super::`) — never on the
//! verify binary's other modules (`args`, `setup`, `image`, `error`). That keeps
//! it compiling inside the lightweight `verify_report`/`verify_args` test
//! harnesses, which `#[path]`-include `scenarios/mod.rs` for the pure types. The
//! small slice of verify config it needs arrives via [`EqualizerRunConfig`]
//! (constructed from `VerifyConfig` in `main.rs`), and the manifest-apply image
//! helpers are inlined below (the same serde_yaml + server-side-apply pattern as
//! `setup.rs`).
//!
//! **Opt-in (constraint 1).** `erw-verify` takes a single `--kubeconfig` (the
//! home cluster). The additional target clusters are supplied via the
//! environment, one path per target:
//!
//! ```text
//! ERW_EQUALIZER_TARGET_KUBECONFIG_1=/path/to/target-1.kubeconfig
//! ERW_EQUALIZER_TARGET_KUBECONFIG_2=/path/to/target-2.kubeconfig
//! …
//! ```
//!
//! The home cluster's own kubeconfig (so it can be an equalizer target too —
//! FR-003) is taken from `ERW_EQUALIZER_HOME_KUBECONFIG`, falling back to
//! `--kubeconfig`. When no `ERW_EQUALIZER_TARGET_KUBECONFIG_*` are set, ALL of
//! E1-E5 are reported `Skip` with guidance — the standard single-cluster run is
//! never broken.
//!
//! The equalizer image is resolved from `ERW_EQUALIZER_IMAGE` (a pre-built ref);
//! otherwise built + pushed from `Dockerfile.equalizer` when a registry is
//! configured (mirroring the webhook image pipeline). When neither is available
//! the manifest placeholder is left in place — the pods fail to pull, surfaced as
//! a readiness failure with a clear diagnostic.
//!
//! **Precondition.** The webhook (Allocation + ClusterCapacity CRDs and their
//! controllers) must already be installed in every target cluster (quickstart
//! prerequisites). The scenario assumes that and observes whatever the equalizer
//! does; a target without the webhook surfaces as `Unreachable`/`ConfigError`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::{Api, DeleteParams, DynamicObject, ListParams, ObjectMeta, Patch, PatchParams};
use kube::core::gvk::GroupVersionKind;
use kube::discovery::{ApiResource, Scope, pinned_kind};
use serde::Deserialize;
use tracing::{info, warn};

use capacity_admission_webhook::crd::{Allocation, CLUSTER_ALLOCATION_NAME};
use capacity_admission_webhook::equalizer::cluster_client::build_target_client;
use capacity_admission_webhook::equalizer::crd::{
    ClusterObservation, ClusterState, EqualizerConfig, EqualizerConfigSpec, EqualizerConfigStatus,
    FLEET_EQUALIZER_NAME, SecretRef, TargetCluster,
};
use capacity_admission_webhook::time_util;

use super::enforcement::{create_pod, delete_pod};
use super::{ScenarioGroup, ScenarioResult, ScenarioStatus};

/// Namespace the equalizer runs in (`deploy/equalizer/deployment.yaml`).
const NAMESPACE: &str = "capacity-equalizer";
/// Key under which each target's kubeconfig YAML is stored in its Secret
/// (contract §2.3.2.2 — matches `SecretRef`'s default key).
const KUBECONFIG_KEY: &str = "kubeconfig";
/// Fleet budget targets used by every E1-E4 scenario (quickstart V1.x: 80/80).
const CPU_TARGET_PERCENT: i32 = 80;
const MEMORY_TARGET_PERCENT: i32 = 80;
/// How long to wait for the equalizer to reconcile the fleet after a change.
const RECONCILE_TIMEOUT: Duration = Duration::from_secs(90);
/// A status `lastReconciled` within this window counts as "fresh" (the equalizer
/// ticked since the change under test).
const FRESHNESS_WINDOW_SECS: i64 = 90;
/// Poll interval while waiting for status to settle.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Server-side-apply field manager identity (mirrors `setup.rs`).
const FIELD_MANAGER: &str = "erw-verify";
/// Env var holding a fully-qualified pre-built equalizer image ref.
const EQUALIZER_IMAGE_ENV: &str = "ERW_EQUALIZER_IMAGE";
/// Env var override for the home cluster's kubeconfig path.
const HOME_KUBECONFIG_ENV: &str = "ERW_EQUALIZER_HOME_KUBECONFIG";
/// Env var prefix for target cluster kubeconfig paths (`_1`, `_2`, …).
const TARGET_KUBECONFIG_PREFIX: &str = "ERW_EQUALIZER_TARGET_KUBECONFIG_";

// E2 load-shaping constants: a few small pods nudge the loaded cluster's
// allocated CPU upward. The exact values are not load-bearing — E2 only needs
// the loaded cluster's utilization to cross the target; the scenario reports the
// observed outcome regardless.
const E2_LOAD_PODS: usize = 4;
const E2_LOAD_CPU: &str = "200m";
const E2_LOAD_MEM: &str = "128Mi";

// Embedded equalizer manifests (compiled in — single-binary property, like the
// webhook stack in `setup.rs`).
const EQ_CRDS: &str = include_str!("../../../../deploy/equalizer/crds.yaml");
const EQ_RBAC: &str = include_str!("../../../../deploy/equalizer/rbac.yaml");
const EQ_DEPLOYMENT: &str = include_str!("../../../../deploy/equalizer/deployment.yaml");

const E1_NAME: &str = "all-under → every cluster set to target";
const E2_NAME: &str = "over-cluster compensation";
const E3_NAME: &str = "unreachable/degraded cluster handled";
const E4_NAME: &str = "EqualizerConfig status shape";
const E5_NAME: &str = "cleanup equalizer resources";
const E1_E4_NAMES: &[(&str, &str)] = &[
    ("E1", E1_NAME),
    ("E2", E2_NAME),
    ("E3", E3_NAME),
    ("E4", E4_NAME),
];

// ===========================================================================
// Configuration (carved out of VerifyConfig so this module is self-contained)
// ===========================================================================

/// The subset of the verify tool's configuration the equalizer scenarios read.
/// Constructed from `VerifyConfig` (in `args.rs`) by `main.rs`. Kept here so the
/// module does not depend on `crate::args` and stays compilable in the verify
/// test harness (see the module docs).
#[derive(Clone)]
pub struct EqualizerRunConfig {
    /// Home cluster kubeconfig path (so the home cluster can be an equalizer
    /// target — FR-003). `None` when the verify run used an inferred kubeconfig.
    pub home_kubeconfig: Option<PathBuf>,
    /// Image registry (for building/pushing the equalizer image).
    pub registry: Option<String>,
    /// Image tag (default `latest`).
    pub image_tag: String,
    /// Skip the Docker build+push phase and reuse an already-pushed image.
    pub skip_build: bool,
}

// ===========================================================================
// Entry point
// ===========================================================================

/// Run the equalizer scenarios E1-E5, returning their results.
///
/// Opt-in: when no target cluster kubeconfigs are supplied, every scenario is
/// reported `Skip` with guidance on how to enable them — the single-cluster run
/// is unaffected (constraint 1). When the fixture IS supplied, the equalizer
/// stack is installed once, E1-E4 run against it, and E5 tears it down
/// unconditionally (even if setup or an earlier scenario failed — constraint 4).
pub async fn run(config: &EqualizerRunConfig, client: &Client) -> Vec<ScenarioResult> {
    let Some(fixture) = resolve_fixture(config) else {
        return skip_all();
    };

    let mut results = Vec::new();
    match setup_fixture(client, &fixture).await {
        Ok(()) => {
            results.push(timed("E1", E1_NAME, e1(client, &fixture)).await);
            results.push(timed("E2", E2_NAME, e2(client, &fixture)).await);
            results.push(timed("E3", E3_NAME, e3(client, &fixture)).await);
            results.push(timed("E4", E4_NAME, e4(client, &fixture)).await);
        }
        Err(e) => {
            warn!(error = %e, "equalizer fixture setup failed");
            let detail = format!("fixture setup failed: {e}");
            for &(id, name) in E1_E4_NAMES {
                results.push(scenario(
                    id,
                    name,
                    ScenarioStatus::Fail,
                    detail.clone(),
                    Duration::ZERO,
                ));
            }
        }
    }

    // E5 cleanup always runs — even when setup or an earlier scenario failed.
    results.push(timed("E5", E5_NAME, e5(client, &fixture)).await);
    results
}

/// Report every E1-E5 scenario as `Skip` (the opt-in gate: no target clusters).
fn skip_all() -> Vec<ScenarioResult> {
    let detail = format!(
        "skipped: no {TARGET_KUBECONFIG_PREFIX}* env vars set. \
         To enable the equalizer scenarios, export one kubeconfig path per \
         additional target cluster (e.g. {TARGET_KUBECONFIG_PREFIX}1=/path/to/target.kubeconfig) \
         and ensure the webhook is installed in each target."
    );
    let mut out: Vec<ScenarioResult> = E1_E4_NAMES
        .iter()
        .map(|&(id, name)| {
            scenario(
                id,
                name,
                ScenarioStatus::Skip,
                detail.clone(),
                Duration::ZERO,
            )
        })
        .collect();
    out.push(scenario(
        "E5",
        E5_NAME,
        ScenarioStatus::Skip,
        detail,
        Duration::ZERO,
    ));
    out
}

// ===========================================================================
// Fixture resolution
// ===========================================================================

/// One cluster in the multi-cluster fixture: its name, the kubeconfig path the
/// scenario reads, and the Secret name the kubeconfig lands in (in [`NAMESPACE`]).
struct TargetDef {
    name: String,
    kubeconfig_path: PathBuf,
    secret_name: String,
}

/// The resolved multi-cluster fixture: the target list + the equalizer image ref
/// to substitute into the Deployment (`None` → leave the manifest placeholder)
/// and whether that ref still needs building + pushing.
struct Fixture {
    targets: Vec<TargetDef>,
    image: Option<String>,
    build_image: bool,
}

impl Fixture {
    /// The `EqualizerConfig` singleton spec targeting every fixture cluster.
    fn equalizer_spec(&self) -> EqualizerConfigSpec {
        EqualizerConfigSpec {
            cpu_target_budget_percent: CPU_TARGET_PERCENT,
            memory_target_budget_percent: MEMORY_TARGET_PERCENT,
            targets: self
                .targets
                .iter()
                .map(|t| TargetCluster {
                    name: t.name.clone(),
                    kubeconfig_secret_ref: SecretRef {
                        name: t.secret_name.clone(),
                        key: KUBECONFIG_KEY.to_string(),
                        namespace: NAMESPACE.to_string(),
                    },
                })
                .collect(),
        }
    }
}

/// Resolve the multi-cluster fixture from the environment + verify config.
///
/// Returns `None` (→ all E1-E5 `Skip`) when no `ERW_EQUALIZER_TARGET_KUBECONFIG_*`
/// are set (the opt-in gate, constraint 1). The home cluster is added as the
/// first target when its kubeconfig is available (`ERW_EQUALIZER_HOME_KUBECONFIG`
/// → `--kubeconfig`); the target clusters follow, in index order. Index gaps end
/// the list (a missing `_N` stops the scan — later indices are ignored).
fn resolve_fixture(config: &EqualizerRunConfig) -> Option<Fixture> {
    let mut targets: Vec<TargetDef> = Vec::new();

    // Home cluster first (so the equalizer manages the cluster it runs in —
    // FR-003). Only included when a kubeconfig path is known.
    if let Some(home) = home_kubeconfig(config) {
        targets.push(TargetDef {
            name: "home".to_string(),
            kubeconfig_path: home,
            secret_name: secret_name("home"),
        });
    }

    // Target clusters from ERW_EQUALIZER_TARGET_KUBECONFIG_1, _2, …
    for index in 1.. {
        let Some(path) = std::env::var(format!("{TARGET_KUBECONFIG_PREFIX}{index}"))
            .ok()
            .filter(|s| !s.is_empty())
        else {
            break;
        };
        let name = format!("target-{index}");
        targets.push(TargetDef {
            name: name.clone(),
            kubeconfig_path: PathBuf::from(path),
            secret_name: secret_name(&name),
        });
    }

    // Opt-in gate: no target clusters supplied → skip the whole group.
    let has_target_cluster = targets.iter().any(|t| t.name != "home");
    if !has_target_cluster {
        return None;
    }

    // Image resolution: explicit pre-built ref → registry-derived (built when
    // not --skip-build) → none (leave the manifest placeholder).
    let explicit = std::env::var(EQUALIZER_IMAGE_ENV)
        .ok()
        .filter(|s| !s.is_empty());
    let (image, build_image) = if let Some(img) = explicit {
        (Some(img), false)
    } else if let Some(registry) = &config.registry {
        let ref_ = format!("{registry}/capacity-equalizer:{}", config.image_tag);
        (Some(ref_), !config.skip_build)
    } else {
        (None, false)
    };

    Some(Fixture {
        targets,
        image,
        build_image,
    })
}

/// Secret name for a cluster's kubeconfig (`erw-eq-<name>-kubeconfig`).
fn secret_name(cluster: &str) -> String {
    format!("erw-eq-{cluster}-kubeconfig")
}

/// Home cluster kubeconfig path: `ERW_EQUALIZER_HOME_KUBECONFIG` → the verify
/// run's `--kubeconfig`.
fn home_kubeconfig(config: &EqualizerRunConfig) -> Option<PathBuf> {
    std::env::var(HOME_KUBECONFIG_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| config.home_kubeconfig.clone())
}

// ===========================================================================
// Fixture setup
// ===========================================================================

/// Install the equalizer fixture: (build + push the image when needed) → apply
/// the equalizer manifests → wait for the CRD to establish → create each
/// cluster's kubeconfig Secret → create the `EqualizerConfig` singleton. The
/// equalizer then reconciles the fleet on its own tick.
async fn setup_fixture(client: &Client, fixture: &Fixture) -> Result<(), String> {
    if fixture.build_image
        && let Some(image_ref) = &fixture.image
    {
        info!(image = %image_ref, "building + pushing equalizer image");
        build_and_push_equalizer(image_ref).await?;
    }

    apply_equalizer_manifests(client, fixture.image.as_deref()).await?;
    wait_for_equalizer_crd(client).await?;

    for target in &fixture.targets {
        let kubeconfig = std::fs::read(&target.kubeconfig_path).map_err(|e| {
            format!(
                "reading kubeconfig for `{}` ({}): {e}",
                target.name,
                target.kubeconfig_path.display()
            )
        })?;
        create_kubeconfig_secret(client, &target.secret_name, NAMESPACE, &kubeconfig).await?;
    }

    create_equalizer_config(client, &fixture.equalizer_spec()).await?;
    info!(
        clusters = fixture.targets.len(),
        "equalizer fixture installed"
    );
    Ok(())
}

/// Apply the embedded `deploy/equalizer/*.yaml` manifests in dependency order:
/// CRDs → RBAC → Deployment (which also declares the Namespace). The resolved
/// image is substituted into the Deployment when available. Mirrors
/// `setup::apply_manifests` (server-side apply, serde_yaml multi-doc parsing).
async fn apply_equalizer_manifests(client: &Client, image: Option<&str>) -> Result<(), String> {
    let mut deployment_docs = parse_docs(EQ_DEPLOYMENT)?;
    if let Some(image_ref) = image {
        for doc in deployment_docs
            .iter_mut()
            .filter(|d| kind_is(d, "Deployment"))
        {
            substitute_image(doc, image_ref);
        }
    }
    let crd_docs = parse_docs(EQ_CRDS)?;
    let rbac_docs = parse_docs(EQ_RBAC)?;

    for d in &crd_docs {
        apply_doc(client, d).await?;
    }
    for d in &rbac_docs {
        apply_doc(client, d).await?;
    }
    for d in &deployment_docs {
        apply_doc(client, d).await?;
    }
    info!("equalizer stack manifests applied");
    Ok(())
}

/// Wait until the `EqualizerConfig` CRD is established (a list against it stops
/// 404-ing). Creating the typed singleton races the CRD establishment otherwise.
async fn wait_for_equalizer_crd(client: &Client) -> Result<(), String> {
    let api: Api<EqualizerConfig> = Api::all(client.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match api.list(&ListParams::default().limit(1)).await {
            Ok(_) => return Ok(()),
            Err(kube::Error::Api(status)) if status.code == 404 => {}
            Err(e) => return Err(format!("listing EqualizerConfig CRD: {e}")),
        }
        if Instant::now() >= deadline {
            return Err("EqualizerConfig CRD never became established within 30s of apply".into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Create/update the kubeconfig `Secret` for one target cluster (server-side
/// apply). The kubeconfig YAML is stored base64-encoded under key
/// [`KUBECONFIG_KEY`] (contract §2.3.2.2).
async fn create_kubeconfig_secret(
    client: &Client,
    name: &str,
    namespace: &str,
    kubeconfig_bytes: &[u8],
) -> Result<(), String> {
    let secret = kubeconfig_secret(name, namespace, kubeconfig_bytes);
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    api.patch(name, &pp, &Patch::Apply(&secret))
        .await
        .map_err(|e| format!("applying kubeconfig Secret {namespace}/{name}: {e}"))?;
    Ok(())
}

/// Create/update the `EqualizerConfig` singleton (server-side apply). The
/// equalizer does NOT auto-create it (contract §4.2), so the scenario must.
async fn create_equalizer_config(
    client: &Client,
    spec: &EqualizerConfigSpec,
) -> Result<(), String> {
    let config = EqualizerConfig::new(FLEET_EQUALIZER_NAME, spec.clone());
    let api: Api<EqualizerConfig> = Api::all(client.clone());
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    api.patch(FLEET_EQUALIZER_NAME, &pp, &Patch::Apply(&config))
        .await
        .map_err(|e| format!("applying EqualizerConfig singleton: {e}"))?;
    info!(
        name = FLEET_EQUALIZER_NAME,
        "EqualizerConfig singleton applied"
    );
    Ok(())
}

// ===========================================================================
// Scenario E1 — all clusters under target → every cluster set to the target
// ===========================================================================

// With every cluster reporting utilization below the 80% target, the equalizer
// has no overflow to distribute: every cluster receives the target budget. We
// assert each cluster's Allocation.spec per-resource override converged to 80%.
async fn e1(client: &Client, fixture: &Fixture) -> Result<String, String> {
    let since = time_util::now_unix();
    let status = wait_for_reconcile(client, fixture.targets.len(), since)
        .await
        .map_err(|e| format!("E1: {e}"))?;

    let mut report = Vec::new();
    for target in &fixture.targets {
        let (cpu, mem) = read_overrides(client, target)
            .await
            .map_err(|e| format!("E1: {e}"))?;
        match (cpu, mem) {
            (Some(c), Some(m)) if c == CPU_TARGET_PERCENT && m == MEMORY_TARGET_PERCENT => {
                report.push(format!("{}=cpu{c}%/mem{m}%", target.name));
            }
            _ => {
                return Err(format!(
                    "E1: cluster `{}` overrides did not converge to target \
                     (cpu={cpu:?}/{CPU_TARGET_PERCENT}, mem={mem:?}/{MEMORY_TARGET_PERCENT}); \
                     fleet={:?}: [{}]",
                    target.name,
                    status.condition,
                    summarize_budgets(&status)
                ));
            }
        }
    }
    Ok(format!(
        "all {} cluster(s) at target cpu={CPU_TARGET_PERCENT}%/mem={MEMORY_TARGET_PERCENT}%; \
         fleet={:?}: [{}]",
        fixture.targets.len(),
        status.condition,
        report.join(", ")
    ))
}

// ===========================================================================
// Scenario E2 — one cluster pushed over the target → frozen, others compensated
// ===========================================================================

// Create dummy load pods in the first non-home target cluster to raise its
// allocated CPU. When that cluster's utilization exceeds the target, the
// equalizer freezes it at floor(utilization) and lowers the other (good)
// clusters' CPU budgets below the target to absorb the overflow (research R5).
//
// CAVEAT: the admission webhook is fail-closed, so it denies pods that would
// breach the current ceiling. The loaded cluster must have headroom under the
// target for the load to be admissible (or the webhook must be in dry-run). If
// no cluster ends up Over, the scenario reports the observed budgets and fails
// with a diagnostic — the precondition could not be established in this fixture.
async fn e2(client: &Client, fixture: &Fixture) -> Result<String, String> {
    let Some(loaded) = fixture.targets.iter().find(|t| t.name != "home") else {
        return Err("E2: no non-home target cluster to load".into());
    };
    let loaded_client = target_client(client, loaded)
        .await
        .map_err(|e| format!("E2: {e}"))?;

    let pod_names = create_load_pods(&loaded_client, &loaded.name).await?;
    let result = assert_over_compensation(client, fixture, loaded).await;
    // Best-effort cleanup of the load pods regardless of the assertion outcome,
    // so E3/E4 start from an unloaded fleet.
    let _ = cleanup_pods(&loaded_client, &pod_names).await;
    result
}

/// Create dummy load pods in `cluster` to raise its allocated CPU, returning the
/// names of the pods actually admitted. A 403 (fail-closed denial — no headroom
/// under the target) stops creation early without error; the equalizer's response
/// is still observable from whatever load landed.
async fn create_load_pods(client: &Client, cluster: &str) -> Result<Vec<String>, String> {
    let mut admitted = Vec::new();
    for i in 0..E2_LOAD_PODS {
        let name = format!("erw-eq-e2-load-{i}");
        match create_pod(client, &name, E2_LOAD_CPU, E2_LOAD_MEM).await {
            Ok(_) => admitted.push(name),
            Err(kube::Error::Api(status)) if status.code == 403 => {
                warn!(%cluster, %name, "E2 load pod denied by admission (no headroom under target)");
                break;
            }
            Err(e) => {
                let _ = cleanup_pods(client, &admitted).await;
                return Err(format!("E2: creating load pod in `{cluster}`: {e}"));
            }
        }
    }
    Ok(admitted)
}

/// Delete a set of pods by name (best-effort; 404s are ignored by `delete_pod`).
async fn cleanup_pods(client: &Client, names: &[String]) -> Result<(), String> {
    for name in names {
        let _ = delete_pod(client, name).await;
    }
    Ok(())
}

/// Wait until the equalizer observes an Over cluster (or the reconcile timeout
/// lapses), then assert the over-cluster is frozen and the good clusters are
/// compensated on the loaded (CPU) dimension.
async fn assert_over_compensation(
    client: &Client,
    fixture: &Fixture,
    loaded: &TargetDef,
) -> Result<String, String> {
    let status = wait_for_over_or_timeout(client, fixture.targets.len(), RECONCILE_TIMEOUT)
        .await
        .map_err(|e| format!("E2: {e}"))?;

    let over: Vec<&ClusterObservation> = status
        .clusters
        .iter()
        .filter(|c| c.state == ClusterState::Over)
        .collect();
    // Every Over cluster is frozen on the resource(s) it is over.
    let all_frozen = over.iter().all(|c| frozen_on_over_dimension(c));
    // Every good cluster is compensated on CPU (the loaded dimension).
    let good_compensated = status
        .clusters
        .iter()
        .filter(|c| c.state == ClusterState::Healthy)
        .all(|c| c.computed_cpu_budget_percent < CPU_TARGET_PERCENT);

    if all_frozen && good_compensated {
        Ok(format!(
            "E2: loaded `{}` → {} over-cluster(s) frozen, good clusters compensated \
             (cpu<{CPU_TARGET_PERCENT}%); fleet={:?}: [{}]",
            loaded.name,
            over.len(),
            status.condition,
            summarize_budgets(&status)
        ))
    } else {
        Err(format!(
            "E2: compensation failed (all_frozen={all_frozen}, good_compensated={good_compensated}): [{}]",
            summarize_budgets(&status)
        ))
    }
}

// ===========================================================================
// Scenario E3 — a target's kubeconfig Secret removed → degraded, others managed
// ===========================================================================

// Delete one non-home target cluster's kubeconfig Secret. The equalizer's next
// reconcile fails to read that Secret → classifies the cluster as `ConfigError`
// (a missing kubeconfig), a degraded state — while the remaining clusters stay
// managed. The Secret is restored afterwards so E4 sees a healthy fleet.
//
// The assertion accepts either `Unreachable` or `ConfigError`: both are the
// contract's "degraded" states (FR-009) for a target the equalizer could not
// reconcile this cycle. Secret deletion specifically yields `ConfigError`.
async fn e3(client: &Client, fixture: &Fixture) -> Result<String, String> {
    let Some(victim) = fixture.targets.iter().find(|t| t.name != "home") else {
        return Err("E3: no non-home target cluster to disconnect".into());
    };
    let secrets: Api<Secret> = Api::namespaced(client.clone(), NAMESPACE);

    secrets
        .delete(&victim.secret_name, &DeleteParams::default())
        .await
        .map_err(|e| format!("E3: deleting Secret for `{}`: {e}", victim.name))?;

    let since = time_util::now_unix();
    let status = match wait_for_reconcile(client, fixture.targets.len(), since).await {
        Ok(s) => s,
        Err(e) => {
            restore_secret(client, victim).await;
            return Err(format!("E3: {e}"));
        }
    };

    let victim_obs = status
        .clusters
        .iter()
        .find(|c| c.name == victim.name)
        .ok_or_else(|| format!("E3: no observation for `{}` in status", victim.name))?;
    let degraded = matches!(
        victim_obs.state,
        ClusterState::Unreachable | ClusterState::ConfigError
    );
    let others_managed = status
        .clusters
        .iter()
        .filter(|c| c.name != victim.name)
        .all(|c| matches!(c.state, ClusterState::Healthy | ClusterState::Over));

    // Restore the Secret regardless of the assertion (do not leave the cluster
    // disconnected for E4).
    restore_secret(client, victim).await;

    if degraded && others_managed {
        Ok(format!(
            "E3: `{}` degraded (state={:?}) after Secret removal; remaining clusters managed; \
             fleet={:?}",
            victim.name, victim_obs.state, status.condition
        ))
    } else {
        Err(format!(
            "E3: expected `{}` degraded + others managed; got state={:?}, others_managed={others_managed}: [{}]",
            victim.name,
            victim_obs.state,
            summarize_budgets(&status)
        ))
    }
}

/// Recreate a victim cluster's kubeconfig Secret (best-effort).
async fn restore_secret(client: &Client, target: &TargetDef) {
    let Ok(bytes) = std::fs::read(&target.kubeconfig_path) else {
        warn!(cluster = %target.name, "E3: kubeconfig unreadable; cannot restore Secret");
        return;
    };
    if let Err(e) = create_kubeconfig_secret(client, &target.secret_name, NAMESPACE, &bytes).await {
        warn!(cluster = %target.name, error = %e, "E3: failed to restore kubeconfig Secret");
    }
}

// ===========================================================================
// Scenario E4 — EqualizerConfig status carries the full per-cluster observation
// ===========================================================================

// Reads the singleton status and verifies it has the contract's shape: a
// `clusters[]` entry per target with utilization + allocatable + computed
// budgets + state + lastObserved, an overall `condition`, and a
// `lastReconciled` timestamp. Mirrors the mocked T036 status-shape test against
// real clusters.
async fn e4(client: &Client, fixture: &Fixture) -> Result<String, String> {
    let since = time_util::now_unix();
    let status = wait_for_reconcile(client, fixture.targets.len(), since)
        .await
        .map_err(|e| format!("E4: {e}"))?;

    if status.clusters.len() != fixture.targets.len() {
        return Err(format!(
            "E4: expected {} cluster observation(s), got {}",
            fixture.targets.len(),
            status.clusters.len()
        ));
    }
    if status.last_reconciled.is_empty() {
        return Err("E4: status.lastReconciled is empty".into());
    }
    let expected_names: Vec<&str> = fixture.targets.iter().map(|t| t.name.as_str()).collect();
    for c in &status.clusters {
        if !expected_names.contains(&c.name.as_str()) {
            return Err(format!("E4: unexpected cluster `{}` in status", c.name));
        }
        if c.last_observed.is_empty() {
            return Err(format!("E4: cluster `{}` has empty lastObserved", c.name));
        }
        // Degraded clusters carry a last_error; healthy/over clusters do not.
        let has_error = c.last_error.is_some();
        let degraded = matches!(
            c.state,
            ClusterState::Unreachable | ClusterState::ConfigError
        );
        if has_error != degraded {
            return Err(format!(
                "E4: cluster `{}` state={:?} but lastError={:?}",
                c.name, c.state, c.last_error
            ));
        }
    }

    Ok(format!(
        "E4: status well-formed — {} cluster(s), condition={:?}, lastReconciled={}",
        status.clusters.len(),
        status.condition,
        status.last_reconciled
    ))
}

// ===========================================================================
// Scenario E5 — tear down the equalizer resources (webhook installs left intact)
// ===========================================================================

// Removes the EqualizerConfig singleton, the equalizer stack (Deployment +
// Namespace + RBAC + CRD), and every kubeconfig Secret. The webhook
// installations in the home + target clusters are left intact — the standard
// teardown handles those. Runs even if an earlier scenario failed (constraint 4).
async fn e5(client: &Client, fixture: &Fixture) -> Result<String, String> {
    let mut errors: Vec<String> = Vec::new();

    // 1. EqualizerConfig singleton (stop status writes before the stack goes).
    let eq_api: Api<EqualizerConfig> = Api::all(client.clone());
    if let Err(e) = eq_api
        .delete(FLEET_EQUALIZER_NAME, &DeleteParams::default())
        .await
        && !is_not_found(&e)
    {
        errors.push(format!("delete EqualizerConfig: {e}"));
    }

    // 2. Equalizer stack manifests (Deployment + Namespace + RBAC + CRD).
    if let Err(e) = delete_equalizer_manifests(client).await {
        errors.push(e);
    }

    // 3. Kubeconfig Secrets (best-effort; the Namespace delete may already have
    //    removed them — 404s are ignored).
    let secrets: Api<Secret> = Api::namespaced(client.clone(), NAMESPACE);
    for target in &fixture.targets {
        match secrets
            .delete(&target.secret_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {}
            Err(e) if is_not_found(&e) => {}
            Err(e) => errors.push(format!("delete Secret {}: {e}", target.secret_name)),
        }
    }

    if errors.is_empty() {
        Ok(format!(
            "E5: removed EqualizerConfig, equalizer stack, and {} kubeconfig Secret(s)",
            fixture.targets.len()
        ))
    } else {
        Err(format!(
            "E5: partial cleanup failure: {}",
            errors.join("; ")
        ))
    }
}

/// Delete every document in the equalizer manifests (best-effort; 404s ignored).
async fn delete_equalizer_manifests(client: &Client) -> Result<(), String> {
    // Delete in reverse apply order: Deployment (+Namespace) → RBAC → CRDs.
    for manifest in [EQ_DEPLOYMENT, EQ_RBAC, EQ_CRDS] {
        let docs = parse_docs(manifest)?;
        for doc in &docs {
            if let Err(e) = delete_doc(client, doc).await {
                warn!(error = %e, "equalizer cleanup: delete_doc failed (continuing)");
            }
        }
    }
    Ok(())
}

// ===========================================================================
// Shared helpers
// ===========================================================================

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
    scenario(id, name, status, detail, start.elapsed())
}

/// Build a [`ScenarioResult`] in this group (used for the skip / setup-failed
/// rows that do not run a timed body).
fn scenario(
    id: &str,
    name: &str,
    status: ScenarioStatus,
    detail: String,
    duration: Duration,
) -> ScenarioResult {
    ScenarioResult {
        id: id.into(),
        name: name.into(),
        group: ScenarioGroup::Equalizer,
        status,
        duration,
        detail,
    }
}

/// Poll the EqualizerConfig status until the equalizer has reconciled the full
/// fleet since `since`: `lastReconciled` is fresh (within [`FRESHNESS_WINDOW_SECS`])
/// AND every target has an observation. Times out after [`RECONCILE_TIMEOUT`].
async fn wait_for_reconcile(
    client: &Client,
    expected_targets: usize,
    since: i64,
) -> Result<EqualizerConfigStatus, String> {
    let api: Api<EqualizerConfig> = Api::all(client.clone());
    let deadline = Instant::now() + RECONCILE_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "equalizer did not reconcile {expected_targets} cluster(s) \
                 (fresh lastReconciled ≥ {since}) within {}s",
                RECONCILE_TIMEOUT.as_secs()
            ));
        }
        if let Ok(obj) = api.get(FLEET_EQUALIZER_NAME).await
            && let Some(status) = &obj.status
            && status.clusters.len() == expected_targets
            && is_fresh(&status.last_reconciled, since)
        {
            return Ok(status.clone());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll the status until an `Over` cluster appears (the equalizer detected the
/// load) or the timeout lapses. Returns the latest status for diagnostics on
/// timeout.
async fn wait_for_over_or_timeout(
    client: &Client,
    expected_targets: usize,
    timeout: Duration,
) -> Result<EqualizerConfigStatus, String> {
    let api: Api<EqualizerConfig> = Api::all(client.clone());
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            let detail = api
                .get(FLEET_EQUALIZER_NAME)
                .await
                .ok()
                .and_then(|o| o.status)
                .map(|s| summarize_budgets(&s))
                .unwrap_or_else(|| "no status".into());
            return Err(format!(
                "no Over cluster observed within {}s — the admission webhook likely \
                 denied the load (fail-closed) or the cluster had too much headroom: [{detail}]",
                timeout.as_secs()
            ));
        }
        if let Ok(obj) = api.get(FLEET_EQUALIZER_NAME).await
            && let Some(status) = obj.status
            && status.clusters.len() == expected_targets
            && status
                .clusters
                .iter()
                .any(|c| c.state == ClusterState::Over)
        {
            return Ok(status);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Whether `last_reconciled` parses to a Unix time ≥ `since` (the equalizer
/// ticked after the change under test) and within the freshness window of now.
fn is_fresh(last_reconciled: &str, since: i64) -> bool {
    let Some(ts) = time_util::parse_rfc3339(last_reconciled) else {
        return false;
    };
    ts >= since && (time_util::now_unix() - ts) <= FRESHNESS_WINDOW_SECS
}

/// Whether an Over cluster is frozen at floor(utilization) on the resource(s) it
/// is over. (Pure — used by E2's assertion.)
fn frozen_on_over_dimension(c: &ClusterObservation) -> bool {
    let cpu_over = c.cpu_utilization_percent > CPU_TARGET_PERCENT as f64;
    let mem_over = c.memory_utilization_percent > MEMORY_TARGET_PERCENT as f64;
    if cpu_over && c.computed_cpu_budget_percent != c.cpu_utilization_percent.floor() as i32 {
        return false;
    }
    if mem_over && c.computed_memory_budget_percent != c.memory_utilization_percent.floor() as i32 {
        return false;
    }
    // An Over cluster must be over on at least one dimension.
    cpu_over || mem_over
}

/// Read a target cluster's `Allocation.spec` per-resource overrides
/// (`cpuBudgetPercent` / `memoryBudgetPercent`). The home cluster is read via the
/// verify client directly; every other cluster via a client built from its
/// kubeconfig.
async fn read_overrides(
    home_client: &Client,
    target: &TargetDef,
) -> Result<(Option<i32>, Option<i32>), String> {
    let alloc = if target.name == "home" {
        Api::<Allocation>::all(home_client.clone())
            .get(CLUSTER_ALLOCATION_NAME)
            .await
    } else {
        let bytes = std::fs::read(&target.kubeconfig_path)
            .map_err(|e| format!("reading kubeconfig for `{}`: {e}", target.name))?;
        let client = build_target_client(&bytes)
            .await
            .map_err(|e| format!("building client for `{}`: {e}", target.name))?;
        Api::<Allocation>::all(client)
            .get(CLUSTER_ALLOCATION_NAME)
            .await
    }
    .map_err(|e| format!("reading Allocation in `{}`: {e}", target.name))?;
    Ok((
        alloc.spec.cpu_budget_percent,
        alloc.spec.memory_budget_percent,
    ))
}

/// Build a client for a target cluster from its kubeconfig. The home cluster
/// reuses the verify client (it is already connected).
async fn target_client(home_client: &Client, target: &TargetDef) -> Result<Client, String> {
    if target.name == "home" {
        return Ok(home_client.clone());
    }
    let bytes = std::fs::read(&target.kubeconfig_path)
        .map_err(|e| format!("reading kubeconfig for `{}`: {e}", target.name))?;
    build_target_client(&bytes)
        .await
        .map_err(|e| format!("building client for `{}`: {e}", target.name))
}

/// Build the kubeconfig `Secret` object from raw bytes (pure — no I/O). The
/// kubeconfig is base64-encoded under key [`KUBECONFIG_KEY`] (contract §2.3.2.2).
fn kubeconfig_secret(name: &str, namespace: &str, kubeconfig_bytes: &[u8]) -> Secret {
    let mut data = BTreeMap::new();
    data.insert(
        KUBECONFIG_KEY.to_string(),
        ByteString(STANDARD.encode(kubeconfig_bytes).into_bytes()),
    );
    Secret {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some(namespace.into()),
            ..Default::default()
        },
        data: Some(data),
        type_: Some("Opaque".into()),
        ..Default::default()
    }
}

/// One-line per-cluster summary for diagnostics (utilization → computed budget).
fn summarize_budgets(status: &EqualizerConfigStatus) -> String {
    status
        .clusters
        .iter()
        .map(|c| {
            format!(
                "{}(state={:?}, cpu {}%→{}, mem {}%→{})",
                c.name,
                c.state,
                c.cpu_utilization_percent.round() as i32,
                c.computed_cpu_budget_percent,
                c.memory_utilization_percent.round() as i32,
                c.computed_memory_budget_percent,
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Manifest apply/delete (inlined from `setup.rs` so this module is
// self-contained — serde_yaml multi-doc parse + server-side apply, matching
// `setup::apply_doc` / `setup::parse_docs`).
// ---------------------------------------------------------------------------

/// Parse a multi-document YAML manifest into JSON values, skipping empty docs.
fn parse_docs(manifest: &str) -> Result<Vec<serde_json::Value>, String> {
    let mut docs = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(manifest) {
        let value: serde_json::Value = serde_json::Value::deserialize(doc)
            .map_err(|e| format!("parsing manifest doc: {e}"))?;
        if !value.is_null() {
            docs.push(value);
        }
    }
    Ok(docs)
}

/// Whether a parsed manifest document is of the given kind.
fn kind_is(doc: &serde_json::Value, kind: &str) -> bool {
    doc.get("kind").and_then(|v| v.as_str()) == Some(kind)
}

/// Set `spec.template.spec.containers[0].image` on a Deployment doc to the
/// resolved image reference (a targeted JSON walk, no-op if the path is absent).
fn substitute_image(doc: &mut serde_json::Value, image: &str) {
    let Some(containers) = doc
        .get_mut("spec")
        .and_then(|s| s.get_mut("template"))
        .and_then(|t| t.get_mut("spec"))
        .and_then(|s| s.get_mut("containers"))
        .and_then(|c| c.as_array_mut())
    else {
        return;
    };
    if let Some(first) = containers.get_mut(0) {
        first["image"] = serde_json::Value::String(image.to_string());
    }
}

/// Split `apiVersion` into `(group, version)` and build a [`GroupVersionKind`].
fn parse_gvk(api_version: &str, kind: &str) -> GroupVersionKind {
    let (group, version) = match api_version.rsplit_once('/') {
        Some((g, v)) => (g, v),
        None => ("", api_version),
    };
    GroupVersionKind::gvk(group, version, kind)
}

/// Resolve the [`ApiResource`] + scope for a GVK via discovery, falling back to a
/// guessed plural (matches `setup::resolve_api_resource`).
async fn resolve_api_resource(
    client: &Client,
    gvk: &GroupVersionKind,
    namespace: Option<&str>,
) -> Result<(ApiResource, Scope), String> {
    match pinned_kind(client, gvk).await {
        Ok((ar, caps)) => Ok((ar, caps.scope)),
        Err(e) => {
            warn!(
                group = %gvk.group,
                version = %gvk.version,
                kind = %gvk.kind,
                error = %e,
                "api discovery failed; guessing the resource plural from the kind"
            );
            let scope = if namespace.is_some() {
                Scope::Namespaced
            } else {
                Scope::Cluster
            };
            Ok((ApiResource::from_gvk(gvk), scope))
        }
    }
}

/// Apply one manifest document via server-side apply (create-or-update).
async fn apply_doc(client: &Client, doc: &serde_json::Value) -> Result<(), String> {
    let api_version = doc
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing apiVersion")?;
    let kind = doc
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing kind")?;
    let name = doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing metadata.name")?;
    let namespace = doc
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str());

    let gvk = parse_gvk(api_version, kind);
    let (ar, scope) = resolve_api_resource(client, &gvk, namespace).await?;
    let pp = PatchParams {
        field_manager: Some(FIELD_MANAGER.into()),
        force: true,
        ..Default::default()
    };
    match scope {
        Scope::Namespaced => {
            let ns = namespace.unwrap_or("default");
            let api = Api::<DynamicObject>::namespaced_with(client.clone(), ns, &ar);
            api.patch(name, &pp, &Patch::Apply(doc))
                .await
                .map_err(|e| format!("applying {kind} {ns}/{name}: {e}"))?;
        }
        Scope::Cluster => {
            let api = Api::<DynamicObject>::all_with(client.clone(), &ar);
            api.patch(name, &pp, &Patch::Apply(doc))
                .await
                .map_err(|e| format!("applying {kind} {name}: {e}"))?;
        }
    }
    Ok(())
}

/// Delete the object described by one manifest document (the inverse of
/// [`apply_doc`]). A 404 (already gone) is treated as success so cleanup is
/// idempotent.
async fn delete_doc(client: &Client, doc: &serde_json::Value) -> Result<(), String> {
    let api_version = doc
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing apiVersion")?;
    let kind = doc
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing kind")?;
    let name = doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .ok_or("manifest document missing metadata.name")?;
    let namespace = doc
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str());

    let gvk = parse_gvk(api_version, kind);
    let (ar, scope) = resolve_api_resource(client, &gvk, namespace).await?;
    let dp = DeleteParams::default();
    match scope {
        Scope::Namespaced => {
            let ns = namespace.unwrap_or("default");
            let api = Api::<DynamicObject>::namespaced_with(client.clone(), ns, &ar);
            delete_or_404(&api, name, &dp).await?;
        }
        Scope::Cluster => {
            let api = Api::<DynamicObject>::all_with(client.clone(), &ar);
            delete_or_404(&api, name, &dp).await?;
        }
    }
    Ok(())
}

/// Delete by name, treating a 404 (already absent) as success.
async fn delete_or_404(
    api: &Api<DynamicObject>,
    name: &str,
    dp: &DeleteParams,
) -> Result<(), String> {
    match api.delete(name, dp).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(e) => Err(format!("deleting {name}: {e}")),
    }
}

/// Build + push the equalizer image from `Dockerfile.equalizer` (mirrors the
/// webhook image pipeline: `docker build -f Dockerfile.equalizer -t <ref> .`).
async fn build_and_push_equalizer(image_ref: &str) -> Result<(), String> {
    let image_ref = image_ref.to_string();
    tokio::task::spawn_blocking(move || {
        run_docker(&["build", "-f", "Dockerfile.equalizer", "-t", &image_ref, "."])?;
        run_docker(&["push", &image_ref])
    })
    .await
    .map_err(|e| format!("equalizer image build/push task failed: {e}"))?
}

/// Invoke `docker <args>`, capturing output. Returns `Err` with stdout+stderr on
/// a non-zero exit, or if the `docker` binary cannot be spawned at all.
fn run_docker(args: &[&str]) -> Result<(), String> {
    let command = args.join(" ");
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("failed to invoke docker {command}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "docker {command} exited with status {}.\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status
    ))
}

/// Whether a kube error is a 404 NotFound.
fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 404)
}

// ===========================================================================
// Unit tests — pure helpers only (the scenario bodies run against real clusters)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_name_format() {
        assert_eq!(secret_name("home"), "erw-eq-home-kubeconfig");
        assert_eq!(secret_name("target-3"), "erw-eq-target-3-kubeconfig");
    }

    #[test]
    fn kubeconfig_secret_carries_base64_under_kubeconfig_key() {
        let yaml = b"apiVersion: v1\nclusters: []\n";
        let secret = kubeconfig_secret("target-1", "capacity-equalizer", yaml);

        assert_eq!(secret.metadata.name.as_deref(), Some("target-1"));
        assert_eq!(
            secret.metadata.namespace.as_deref(),
            Some("capacity-equalizer")
        );
        assert_eq!(secret.type_.as_deref(), Some("Opaque"));

        let data = secret.data.expect("data map present");
        let ByteString(encoded) = data.get(KUBECONFIG_KEY).expect("`kubeconfig` key present");
        // The stored value is the standard base64 encoding of the input bytes.
        assert_eq!(&STANDARD.decode(encoded).unwrap(), yaml);
        // No other keys leaked in.
        assert_eq!(data.len(), 1, "only the kubeconfig key: {data:?}");
    }

    #[test]
    fn kubeconfig_secret_is_deterministic() {
        // Same input → identical Secret.
        let a = kubeconfig_secret("home", "ns", b"kubeconfig-bytes");
        let b = kubeconfig_secret("home", "ns", b"kubeconfig-bytes");
        assert_eq!(
            a.data.as_ref().and_then(|d| d.get(KUBECONFIG_KEY)),
            b.data.as_ref().and_then(|d| d.get(KUBECONFIG_KEY)),
        );
    }

    #[test]
    fn is_fresh_true_when_recent_and_after_since() {
        let now = time_util::now_unix();
        let ts = now - 10; // reconciled 10s ago
        let stamp = time_util::rfc3339_from_unix(ts);
        assert!(is_fresh(&stamp, now - 60));
    }

    #[test]
    fn is_fresh_false_when_before_since() {
        // A reconcile older than the change under test must not count.
        let now = time_util::now_unix();
        let stale = now - 120;
        let stamp = time_util::rfc3339_from_unix(stale);
        assert!(!is_fresh(&stamp, now - 30));
    }

    #[test]
    fn is_fresh_false_when_outside_window() {
        let now = time_util::now_unix();
        // Older than the freshness window even if after `since`.
        let old = now - (FRESHNESS_WINDOW_SECS + 60);
        let stamp = time_util::rfc3339_from_unix(old);
        assert!(!is_fresh(&stamp, old - 10));
    }

    #[test]
    fn is_fresh_false_for_unparseable() {
        assert!(!is_fresh("not-a-timestamp", 0));
        assert!(!is_fresh("", 0));
    }

    #[test]
    fn frozen_on_over_dimension_cpu_over_frozen_at_floor() {
        let c = ClusterObservation {
            name: "target-1".into(),
            cpu_utilization_percent: 90.7,
            memory_utilization_percent: 40.0,
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 1_000_000,
            computed_cpu_budget_percent: 90, // floor(90.7)
            computed_memory_budget_percent: MEMORY_TARGET_PERCENT,
            state: ClusterState::Over,
            last_error: None,
            last_observed: "2026-08-07T00:00:00Z".into(),
        };
        assert!(frozen_on_over_dimension(&c));
    }

    #[test]
    fn frozen_on_over_dimension_not_frozen_when_mismatched() {
        let mut c = ClusterObservation {
            name: "target-1".into(),
            cpu_utilization_percent: 90.0,
            memory_utilization_percent: 40.0,
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 1_000_000,
            computed_cpu_budget_percent: 80, // NOT frozen at floor(90)=90
            computed_memory_budget_percent: MEMORY_TARGET_PERCENT,
            state: ClusterState::Over,
            last_error: None,
            last_observed: "2026-08-07T00:00:00Z".into(),
        };
        assert!(!frozen_on_over_dimension(&c));
        // Memory-over path:
        c.cpu_utilization_percent = 40.0;
        c.memory_utilization_percent = 95.0;
        c.computed_memory_budget_percent = 80; // not floor(95)=95
        assert!(!frozen_on_over_dimension(&c));
    }

    #[test]
    fn frozen_on_over_dimension_neither_over_is_false() {
        // A cluster over on neither dimension is not "frozen" — contradicts Over.
        let c = ClusterObservation {
            name: "target-1".into(),
            cpu_utilization_percent: 50.0,
            memory_utilization_percent: 50.0,
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 1_000_000,
            computed_cpu_budget_percent: CPU_TARGET_PERCENT,
            computed_memory_budget_percent: MEMORY_TARGET_PERCENT,
            state: ClusterState::Over,
            last_error: None,
            last_observed: "2026-08-07T00:00:00Z".into(),
        };
        assert!(!frozen_on_over_dimension(&c));
    }

    #[test]
    fn parse_docs_splits_multi_document_yaml() {
        let manifest = "---\nkind: A\nmetadata: { name: a }\n---\nkind: B\nmetadata: { name: b }\n";
        let docs = parse_docs(manifest).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(kind_is(&docs[0], "A"));
        assert!(kind_is(&docs[1], "B"));
    }

    #[test]
    fn parse_docs_skips_empty_documents() {
        let manifest = "---\nkind: A\nmetadata: { name: a }\n---\n---\n";
        let docs = parse_docs(manifest).unwrap();
        assert_eq!(docs.len(), 1, "empty docs dropped: {docs:?}");
    }

    #[test]
    fn substitute_image_sets_first_container_image() {
        let mut doc = serde_json::json!({
            "kind": "Deployment",
            "spec": { "template": { "spec": { "containers": [
                { "name": "eq", "image": "ERW_EQUALIZER_IMAGE_PLACEHOLDER" }
            ] } } }
        });
        substitute_image(&mut doc, "cr.example/capacity-equalizer:latest");
        assert_eq!(
            doc["spec"]["template"]["spec"]["containers"][0]["image"],
            "cr.example/capacity-equalizer:latest"
        );
    }

    #[test]
    fn substitute_image_no_op_when_path_absent() {
        let mut doc = serde_json::json!({ "kind": "Deployment", "spec": {} });
        substitute_image(&mut doc, "ignored:tag");
        // Unchanged, no panic.
        assert_eq!(doc["spec"], serde_json::json!({}));
    }

    #[test]
    fn summarize_budgets_renders_each_cluster() {
        let status = EqualizerConfigStatus {
            clusters: vec![
                ClusterObservation {
                    name: "home".into(),
                    cpu_utilization_percent: 65.0,
                    memory_utilization_percent: 60.0,
                    total_allocatable_cpu_milli: 100_000,
                    total_allocatable_memory_bytes: 1_000_000,
                    computed_cpu_budget_percent: 80,
                    computed_memory_budget_percent: 80,
                    state: ClusterState::Healthy,
                    last_error: None,
                    last_observed: "2026-08-07T00:00:00Z".into(),
                },
                ClusterObservation {
                    name: "target-1".into(),
                    cpu_utilization_percent: 90.0,
                    memory_utilization_percent: 40.0,
                    total_allocatable_cpu_milli: 100_000,
                    total_allocatable_memory_bytes: 1_000_000,
                    computed_cpu_budget_percent: 90,
                    computed_memory_budget_percent: 80,
                    state: ClusterState::Over,
                    last_error: None,
                    last_observed: "2026-08-07T00:00:00Z".into(),
                },
            ],
            condition: capacity_admission_webhook::equalizer::crd::FleetCondition::Compensating,
            last_reconciled: "2026-08-07T00:00:05Z".into(),
        };
        let s = summarize_budgets(&status);
        assert!(s.contains("home(state=Healthy"), "{s}");
        assert!(s.contains("target-1(state=Over"), "{s}");
        assert!(s.contains("cpu 90%→90"), "{s}");
    }
}
