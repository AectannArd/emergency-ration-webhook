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
    ClusterObservation, ClusterState, EqualizerConfig, EqualizerConfigSpec, EqualizerConfigStatus,
    FLEET_EQUALIZER_NAME, FleetCondition, SecretRef, TargetCluster,
};
use capacity_admission_webhook::equalizer::reconcile::reconcile;
use kube::Client;

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

/// Like [`serve_secret_gets`], but the Secret named `missing` gets a 404 NotFound
/// (so the reconcile loop classifies that target's kubeconfig Secret as absent →
/// `ConfigError`). Used by the missing-Secret reachability test (T034).
async fn serve_secret_gets_with_missing(
    handle: &mut common::MockHandle,
    secrets: &HashMap<String, serde_json::Value>,
    missing: &str,
    namespace: &str,
    count: usize,
) {
    for _ in 0..count {
        let (req, respond) = handle.next_request().await.expect("a Secret GET");
        assert_eq!(req.method().as_str(), "GET", "home reads Secrets via GET");
        let name =
            common::secret_name_from_path(req.uri().path()).expect("Secret name in GET path");
        if name == missing {
            respond.send_response(common::not_found_status(name, namespace));
        } else {
            let secret = secrets
                .get(name)
                .unwrap_or_else(|| panic!("no fixture Secret for requested name `{name}`"));
            respond.send_response(common::ok_object(secret));
        }
    }
}

/// Run one reconcile cycle against the home mock, serving the kubeconfig Secret
/// GETs the loop issues for `target_count` targets, and return the resulting
/// status. The home client + handle are reused across calls by multi-cycle
/// scenarios (T029/T035).
async fn run_reconcile(
    home_client: &Client,
    home_handle: &mut common::MockHandle,
    secrets: &HashMap<String, serde_json::Value>,
    eq_config: &EqualizerConfig,
    target_count: usize,
) -> EqualizerConfigStatus {
    let home = home_client.clone();
    let cfg = eq_config.clone();
    let task = tokio::spawn(async move { reconcile(&home, &cfg).await });
    serve_secret_gets(home_handle, secrets, target_count).await;
    task.await.expect("reconcile completed without panicking")
}

/// Spawn one mock target per `(name, cpu_util, mem_util)` at uniform capacity
/// (100_000m CPU / 200 GiB RAM), and the matching kubeconfig Secret per target.
/// Returns the mocks (same order as `targets`) + the Secret fixtures.
async fn spawn_fleet(
    targets: &[Target],
) -> (Vec<common::MockTarget>, HashMap<String, serde_json::Value>) {
    let mut mocks = Vec::new();
    let mut secrets = HashMap::new();
    for (name, cpu_util, mem_util) in targets {
        let mock = common::spawn_mock_target(*cpu_util, *mem_util, 100_000, 200 * GIB).await;
        let secret_name = format!("target-{name}");
        let yaml = common::kubeconfig_yaml(&mock.addr);
        let secret = common::kubeconfig_secret_value(&secret_name, NS, &yaml);
        secrets.insert(secret_name, secret);
        mocks.push(mock);
    }
    (mocks, secrets)
}

/// Look up a cluster's observation in status by name.
fn obs_for<'a>(status: &'a EqualizerConfigStatus, name: &str) -> &'a ClusterObservation {
    status
        .clusters
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("status has an observation for cluster `{name}`"))
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

// ===========================================================================
// Phase 4 / US2 — over-limit compensation (T028–T031). Verification-only: the
// reconcile loop from T025 is generic (it calls `equalize()` once per resource
// regardless of over/good partition), so these exercise the same code path as
// the all-under test — they assert the over-compensation contract holds through
// the full read → compute → patch cycle.
// ===========================================================================

#[tokio::test]
async fn equalize_one_over_compensates_mocked() {
    // T028 / data-model §2.3 Example 2: 3 mocks, CPU util 65/55/90%, uniform
    // 100_000m. C is over target (frozen at 90); its overflow (10% × 100_000m =
    // 10_000m) is split across A and B, dropping each from 80 to 75. Memory is
    // held under target on every cluster so CPU is the only varying dimension.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 50.0), ("b", 55.0, 50.0), ("c", 90.0, 50.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();
    let status = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;

    // Each target received exactly one PATCH; C frozen at 90, A/B compensated to 75.
    // FR-007: the patch touches only the override keys, never `budgetPercent`.
    let expected_cpu = [("a", 75), ("b", 75), ("c", 90)];
    for (i, (name, cpu)) in expected_cpu.iter().enumerate() {
        let patches = mocks[i].patches();
        assert_eq!(patches.len(), 1, "one PATCH per target: {patches:?}");
        let spec = &patches[0]["spec"];
        assert_eq!(
            spec["cpuBudgetPercent"].as_i64(),
            Some(*cpu as i64),
            "cluster {name} cpuBudgetPercent: {patches:?}"
        );
        // Memory all-under → compensated to the 80% target.
        assert_eq!(
            spec["memoryBudgetPercent"].as_i64(),
            Some(80),
            "cluster {name} memoryBudgetPercent: {patches:?}"
        );
        assert!(
            spec.get("budgetPercent").is_none(),
            "FR-007: must never patch budgetPercent: {patches:?}"
        );
    }

    // C is Over on CPU; A/B Healthy. The fleet is compensating.
    assert_eq!(obs_for(&status, "a").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "b").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "c").state, ClusterState::Over);
    assert_eq!(status.condition, FleetCondition::Compensating);
}

#[tokio::test]
async fn equalize_two_cycle_utilization_drop_mocked() {
    // T029: a two-cycle scenario. Cycle 1: CPU 65/55/90 → 75/75/90. Between
    // cycles C's load drops to 86% (still over). Cycle 2: CPU 65/55/86 → 77/77/86.
    // C's smaller overflow (6% × 100_000m = 6_000m, /2 = 3_000m → 3pp) drops A/B to
    // 77. Asserts the per-cluster CPU budget PATCH changes between cycles.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 50.0), ("b", 55.0, 50.0), ("c", 90.0, 50.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();

    // Cycle 1 — C at 90%.
    let status1 = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;
    let cycle1_cpu = [("a", 75), ("b", 75), ("c", 90)];
    for (i, (name, cpu)) in cycle1_cpu.iter().enumerate() {
        let patches = mocks[i].patches();
        assert_eq!(
            patches.len(),
            1,
            "cycle 1: one PATCH per {name}: {patches:?}"
        );
        assert_eq!(
            patches[0]["spec"]["cpuBudgetPercent"].as_i64(),
            Some(*cpu as i64),
            "cycle 1 cluster {name}: {patches:?}"
        );
    }
    assert_eq!(status1.condition, FleetCondition::Compensating);

    // C drops to 86% (still over target) — load changed between cycles.
    mocks[2].set_utilization(86.0, 50.0);

    // Cycle 2 — C at 86%.
    let status2 = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;
    let cycle2_cpu = [("a", 77), ("b", 77), ("c", 86)];
    for (i, (name, cpu)) in cycle2_cpu.iter().enumerate() {
        let patches = mocks[i].patches();
        assert_eq!(
            patches.len(),
            2,
            "cycle 2: a second PATCH per {name}: {patches:?}"
        );
        assert_eq!(
            patches[1]["spec"]["cpuBudgetPercent"].as_i64(),
            Some(*cpu as i64),
            "cycle 2 cluster {name}: {patches:?}"
        );
        // The budget moved between cycles (75→77 for A/B, 90→86 for C).
        assert_ne!(
            patches[0]["spec"]["cpuBudgetPercent"], patches[1]["spec"]["cpuBudgetPercent"],
            "cluster {name} budget changed across cycles"
        );
    }
    assert_eq!(status2.condition, FleetCondition::Compensating);
}

#[tokio::test]
async fn equalize_all_over_freezes_mocked() {
    // T030 / data-model §2.3 Example 4: every cluster over target (85/85/85%).
    // No good cluster can absorb overflow, so each is frozen at its current
    // utilization (85) — there is no compensation. Every cluster still receives a
    // PATCH setting its (frozen) budget; none is reduced below target.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 85.0, 50.0), ("b", 85.0, 50.0), ("c", 85.0, 50.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();
    let status = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;

    for (i, mock) in mocks.iter().enumerate() {
        let patches = mock.patches();
        assert_eq!(patches.len(), 1, "one PATCH per over cluster: {patches:?}");
        let spec = &patches[0]["spec"];
        // Frozen at current utilization — NOT reduced.
        assert_eq!(
            spec["cpuBudgetPercent"].as_i64(),
            Some(85),
            "cluster {} frozen at 85: {patches:?}",
            targets[i].0
        );
        // Memory all-under → target.
        assert_eq!(spec["memoryBudgetPercent"].as_i64(), Some(80));
    }

    // Every cluster Over on CPU; the fleet is compensating (no good clusters).
    for obs in &status.clusters {
        assert_eq!(obs.state, ClusterState::Over, "cluster {} Over", obs.name);
    }
    assert_eq!(status.condition, FleetCondition::Compensating);
}

#[tokio::test]
async fn equalize_cpu_ram_independent_mocked() {
    // T031 / FR-014: CPU and memory are equalized independently. CPU is at target
    // on every cluster (80/80/80 → 80/80/80, all good); memory has one over-cluster
    // (75/75/90 → 75/75/90, C frozen, A/B compensated). On the same cluster the two
    // override fields therefore carry different values, proving the two dimensions
    // are computed and patched independently.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 80.0, 75.0), ("b", 80.0, 75.0), ("c", 80.0, 90.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();
    let status = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;

    // (cluster, expected CPU budget, expected memory budget).
    let expected = [("a", 80, 75), ("b", 80, 75), ("c", 80, 90)];
    for (i, (name, cpu, mem)) in expected.iter().enumerate() {
        let patches = mocks[i].patches();
        assert_eq!(patches.len(), 1, "one PATCH per {name}: {patches:?}");
        let spec = &patches[0]["spec"];
        assert_eq!(
            spec["cpuBudgetPercent"].as_i64(),
            Some(*cpu as i64),
            "cluster {name} cpuBudgetPercent: {patches:?}"
        );
        assert_eq!(
            spec["memoryBudgetPercent"].as_i64(),
            Some(*mem as i64),
            "cluster {name} memoryBudgetPercent: {patches:?}"
        );
        // The two resource dimensions differ on the same cluster — independence.
        assert_ne!(
            spec["cpuBudgetPercent"], spec["memoryBudgetPercent"],
            "cluster {name} CPU and memory budgets differ"
        );
    }

    // C is Over on memory (CPU good); A/B Healthy on both. Fleet compensating.
    assert_eq!(obs_for(&status, "a").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "b").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "c").state, ClusterState::Over);
    assert_eq!(status.condition, FleetCondition::Compensating);
}

// ===========================================================================
// Phase 5 / US3 — target reachability & status reporting (T033–T036).
// Verification-only: the reconcile loop from T025 already resolves each target
// independently and records per-cluster failures (Unreachable / ConfigError) in
// status, continuing with the remaining clusters (FR-009, FR-012).
// ===========================================================================

#[tokio::test]
async fn unreachable_cluster_skipped_mocked() {
    // T033: cluster C's apiserver errors on Allocation reads (HTTP 500). C is
    // classified Unreachable and skipped (no PATCH); A/B are patched with their
    // computed budgets. The fleet is Degraded.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 60.0), ("b", 55.0, 50.0), ("c", 45.0, 40.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    // C's apiserver is up but erroring on Allocation reads.
    mocks[2].set_failing(true);

    let (home_client, mut home_handle) = common::mock_home_client();
    let status = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;

    // A/B patched (all-under → 80); C NOT patched (Unreachable, never reached the
    // PATCH phase — it failed at the Allocation read).
    let a = mocks[0].patches();
    let b = mocks[1].patches();
    let c = mocks[2].patches();
    assert_eq!(a.len(), 1, "reachable A patched: {a:?}");
    assert_eq!(b.len(), 1, "reachable B patched: {b:?}");
    assert_eq!(c.len(), 0, "unreachable C is not patched: {c:?}");
    assert_eq!(a[0]["spec"]["cpuBudgetPercent"].as_i64(), Some(80));
    assert_eq!(b[0]["spec"]["cpuBudgetPercent"].as_i64(), Some(80));

    // C Unreachable with an error message; A/B Healthy. Fleet Degraded.
    assert_eq!(obs_for(&status, "a").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "b").state, ClusterState::Healthy);
    let c_obs = obs_for(&status, "c");
    assert_eq!(c_obs.state, ClusterState::Unreachable);
    let err = c_obs
        .last_error
        .as_ref()
        .expect("C carries an error message");
    assert!(!err.is_empty(), "error message is non-empty: {err}");
    assert_eq!(status.condition, FleetCondition::Degraded);
}

#[tokio::test]
async fn config_error_missing_secret_mocked() {
    // T034: cluster C's kubeconfig Secret is absent from the home cluster (the
    // home mock answers C's Secret GET with 404). C is classified ConfigError and
    // skipped before any target call; A/B resolve + patch normally.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 60.0), ("b", 55.0, 50.0), ("c", 45.0, 40.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();
    let home = home_client.clone();
    let cfg = eq_config.clone();
    let task = tokio::spawn(async move { reconcile(&home, &cfg).await });

    // C's Secret ("target-c") is absent → home mock answers 404 for it.
    serve_secret_gets_with_missing(&mut home_handle, &secrets, "target-c", NS, targets.len()).await;
    let status = task.await.expect("reconcile completed without panicking");

    // A/B patched; C never reached (the Secret read failed first).
    assert_eq!(mocks[0].patches().len(), 1, "A patched");
    assert_eq!(mocks[1].patches().len(), 1, "B patched");
    assert_eq!(mocks[2].patches().len(), 0, "C (ConfigError) not patched");

    // C ConfigError naming the missing Secret; A/B Healthy. Fleet Degraded.
    assert_eq!(obs_for(&status, "a").state, ClusterState::Healthy);
    assert_eq!(obs_for(&status, "b").state, ClusterState::Healthy);
    let c_obs = obs_for(&status, "c");
    assert_eq!(c_obs.state, ClusterState::ConfigError);
    let err = c_obs
        .last_error
        .as_ref()
        .expect("C carries an error message");
    assert!(
        err.contains("target-c"),
        "error names the missing Secret: {err}"
    );
    assert_eq!(status.condition, FleetCondition::Degraded);
}

#[tokio::test]
async fn recovery_unreachable_to_healthy_mocked() {
    // T035 / FR-012: cluster C is unreachable on cycle 1 (Unreachable, skipped),
    // then recovers on cycle 2 (reachable → Healthy, budget applied). The cycle
    // is stateless per tick, so recovery is just the next reconcile succeeding.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 60.0), ("b", 55.0, 50.0), ("c", 45.0, 40.0)];
    let (mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();

    // Cycle 1 — C failing.
    mocks[2].set_failing(true);
    let status1 = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;
    assert_eq!(obs_for(&status1, "c").state, ClusterState::Unreachable);
    assert_eq!(obs_for(&status1, "a").state, ClusterState::Healthy);
    assert_eq!(status1.condition, FleetCondition::Degraded);
    assert_eq!(
        mocks[2].patches().len(),
        0,
        "C not patched while unreachable"
    );

    // Cycle 2 — C recovers.
    mocks[2].set_failing(false);
    let status2 = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;
    let c_obs = obs_for(&status2, "c");
    assert_eq!(c_obs.state, ClusterState::Healthy, "C recovered to Healthy");
    assert!(c_obs.last_error.is_none(), "no error after recovery");
    // C patched exactly once (on cycle 2 — it was skipped on cycle 1).
    let c_patches = mocks[2].patches();
    assert_eq!(
        c_patches.len(),
        1,
        "C patched once recovered: {c_patches:?}"
    );
    assert_eq!(c_patches[0]["spec"]["cpuBudgetPercent"].as_i64(), Some(80));
    assert_eq!(
        status2.condition,
        FleetCondition::Healthy,
        "fleet Healthy once C recovered"
    );
}

#[tokio::test]
async fn full_status_shape_mocked() {
    // T036: every ClusterObservation field (the 10 per the CRD contract §3) is
    // populated for a reachable cluster, plus the top-level condition +
    // lastReconciled. Asserted on the returned EqualizerConfigStatus struct.
    common::ensure_crypto_provider();
    let targets: Vec<Target> = vec![("a", 65.0, 60.0), ("b", 55.0, 50.0), ("c", 45.0, 40.0)];
    let (_mocks, secrets) = spawn_fleet(&targets).await;
    let eq_config = equalizer_config(&targets, 80, 80);

    let (home_client, mut home_handle) = common::mock_home_client();
    let status = run_reconcile(
        &home_client,
        &mut home_handle,
        &secrets,
        &eq_config,
        targets.len(),
    )
    .await;

    assert_eq!(status.clusters.len(), targets.len());
    assert_eq!(status.condition, FleetCondition::Healthy);
    assert!(
        !status.last_reconciled.is_empty(),
        "lastReconciled populated"
    );

    // Inspect cluster A — all 10 ClusterObservation fields populated.
    let a = obs_for(&status, "a");
    assert_eq!(a.name, "a"); // 1. name
    assert_eq!(a.cpu_utilization_percent, 65.0); // 2. observed CPU util
    assert_eq!(a.memory_utilization_percent, 60.0); // 3. observed memory util
    assert_eq!(a.total_allocatable_cpu_milli, 100_000); // 4. allocatable CPU (milli)
    assert_eq!(a.total_allocatable_memory_bytes, 200 * GIB); // 5. allocatable memory (bytes)
    assert_eq!(a.computed_cpu_budget_percent, 80); // 6. computed CPU budget
    assert_eq!(a.computed_memory_budget_percent, 80); // 7. computed memory budget
    assert_eq!(a.state, ClusterState::Healthy); // 8. state
    assert!(a.last_error.is_none()); // 9. last_error (None when healthy)
    assert!(!a.last_observed.is_empty(), "lastObserved populated"); // 10. last_observed
}
