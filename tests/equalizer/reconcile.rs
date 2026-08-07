//! Integration tests for the equalizer reconcile loop (spec-013, Phase 3 / US1).
//!
//! Drives the real [`reconcile`] path end-to-end against mocked target
//! apiservers: a tower-test mock home apiserver serves the kubeconfig `Secret`s,
//! and real local axum HTTP servers serve each target cluster's `Allocation` +
//! `ClusterCapacity` (and capture the spec-override PATCHes). This exercises the
//! full multi-cluster wiring — Secret read → `build_target_client` →
//! `Config::from_custom_kubeconfig` → GET status → `equalize()` → PATCH overrides
//! → status — without real clusters.
//!
//! Run with `cargo test --test reconcile`.

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashMap;

use capacity_admission_webhook::equalizer::crd::{
    ClusterState, EqualizerConfig, EqualizerConfigSpec, FLEET_EQUALIZER_NAME, FleetCondition,
    SecretRef, TargetCluster,
};
use capacity_admission_webhook::equalizer::reconcile::reconcile;

const GIB: i64 = 1024 * 1024 * 1024;
const NS: &str = "fleet-equalizer";

/// `(cluster name, CPU util %, memory util %)` for the three-target fleet.
type Target = (&'static str, f64, f64);

/// Build the EqualizerConfig singleton targeting `targets`, each referencing a
/// Secret named `target-<name>` in namespace [`NS`].
fn equalizer_config(targets: &[Target], cpu_target: i32, mem_target: i32) -> EqualizerConfig {
    EqualizerConfig::new(
        FLEET_EQUALIZER_NAME,
        EqualizerConfigSpec {
            cpu_target_budget_percent: cpu_target,
            memory_target_budget_percent: mem_target,
            targets: targets
                .iter()
                .map(|(name, _, _)| TargetCluster {
                    name: (*name).to_string(),
                    kubeconfig_secret_ref: SecretRef {
                        name: format!("target-{name}"),
                        key: "kubeconfig".to_string(),
                        namespace: NS.to_string(),
                    },
                })
                .collect(),
        },
    )
}

/// Drive `count` kubeconfig-Secret GETs against the home mock in arrival order,
/// responding with the matching Secret from `secrets`. (The reconcile loop reads
/// all Secrets concurrently, so the arrival order is non-deterministic — this
/// keys the response off the requested Secret name, not position.)
async fn serve_secret_gets(
    handle: &mut common::MockHandle,
    secrets: &HashMap<String, serde_json::Value>,
    count: usize,
) {
    for _ in 0..count {
        let (req, respond) = handle.next_request().await.expect("a Secret GET");
        assert_eq!(req.method().as_str(), "GET", "home reads Secrets via GET");
        let name =
            common::secret_name_from_path(req.uri().path()).expect("Secret name in GET path");
        let secret = secrets
            .get(name)
            .unwrap_or_else(|| panic!("no fixture Secret for requested name `{name}`"));
        respond.send_response(common::ok_object(secret));
    }
}

#[tokio::test]
async fn equalize_all_under_target_mocked() {
    // T023 / quickstart V1.3: 3 target clusters all under the 80% target. After
    // reconcile, each target receives a PATCH setting both per-resource overrides
    // to 80; the status reports all clusters Healthy and the fleet Healthy.
    common::ensure_crypto_provider();

    // (name, CPU util%, memory util%) — all under the 80% target on both dims.
    let targets: Vec<Target> = vec![("a", 65.0, 60.0), ("b", 55.0, 50.0), ("c", 45.0, 40.0)];

    // Spawn one mock target apiserver per cluster (uniform 100_000m / 200 GiB).
    let mut mocks = Vec::new();
    for (_, cpu_util, mem_util) in &targets {
        let mock = common::spawn_mock_target(*cpu_util, *mem_util, 100_000, 200 * GIB).await;
        mocks.push(mock);
    }

    // Build a kubeconfig Secret per target, pointing at its mock apiserver.
    let mut secrets = HashMap::new();
    for (i, (name, _, _)) in targets.iter().enumerate() {
        let secret_name = format!("target-{name}");
        let yaml = common::kubeconfig_yaml(&mocks[i].addr);
        let secret = common::kubeconfig_secret_value(&secret_name, NS, &yaml);
        secrets.insert(secret_name, secret);
    }

    let eq_config = equalizer_config(&targets, 80, 80);

    // Run reconcile on a task while driving the home mock's Secret GETs.
    let (home_client, mut home_handle) = common::mock_home_client();
    let home_for_reconcile = home_client.clone();
    let task = tokio::spawn(async move { reconcile(&home_for_reconcile, &eq_config).await });

    serve_secret_gets(&mut home_handle, &secrets, targets.len()).await;

    let status = task.await.expect("reconcile completed without panicking");

    // Each target received exactly one PATCH with both overrides = 80, and the
    // patch touched ONLY the override keys — never `budgetPercent` (FR-007).
    for mock in &mocks {
        let patches = mock.patches();
        assert_eq!(patches.len(), 1, "one PATCH per target: {patches:?}");
        let spec = &patches[0]["spec"];
        assert_eq!(
            spec["cpuBudgetPercent"].as_i64(),
            Some(80),
            "cpuBudgetPercent patched to target: {patches:?}"
        );
        assert_eq!(
            spec["memoryBudgetPercent"].as_i64(),
            Some(80),
            "memoryBudgetPercent patched to target: {patches:?}"
        );
        assert!(
            spec.get("budgetPercent").is_none(),
            "FR-007: must never patch budgetPercent: {patches:?}"
        );
    }

    // Status: one observation per cluster, all Healthy; fleet Healthy.
    assert_eq!(status.clusters.len(), targets.len());
    for obs in &status.clusters {
        assert_eq!(
            obs.state,
            ClusterState::Healthy,
            "cluster {} Healthy",
            obs.name
        );
        assert_eq!(obs.computed_cpu_budget_percent, 80);
        assert_eq!(obs.computed_memory_budget_percent, 80);
        assert!(obs.last_error.is_none(), "no error on a reachable cluster");
    }
    assert_eq!(status.condition, FleetCondition::Healthy);
}
