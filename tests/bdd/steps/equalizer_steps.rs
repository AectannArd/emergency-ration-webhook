//! BDD step definitions for the multi-cluster capacity equalizer (spec-013,
//! T024 / quickstart V1.4).
//!
//! Drives the real [`reconcile`] loop through Cucumber steps against mocked
//! target apiservers (one real local HTTP server per cluster, fed via a
//! tower-test mock home apiserver that serves the kubeconfig Secrets). The
//! `World` materialises the cluster fixtures from the scenario figures, runs the
//! reconcile, and asserts on the issued budget PATCHes and the fleet condition.
//!
//! Run with `cargo test --test equalizer_bdd`.

#[path = "../../common/mod.rs"]
mod common;

use std::collections::HashMap;

use capacity_admission_webhook::equalizer::crd::{
    EqualizerConfig, EqualizerConfigSpec, EqualizerConfigStatus, FLEET_EQUALIZER_NAME,
    FleetCondition, SecretRef, TargetCluster,
};
use capacity_admission_webhook::equalizer::reconcile::reconcile;
use cucumber::{World as _, given, then, when};

const GIB: i64 = 1024 * 1024 * 1024;
const NS: &str = "fleet-equalizer";

#[derive(cucumber::World)]
struct EqualizerWorld {
    /// CPU utilization figures from the `Given` step.
    cpu_utils: Vec<f64>,
    names: Vec<String>,
    mocks: Vec<common::MockTarget>,
    secrets: HashMap<String, serde_json::Value>,
    cpu_target: i32,
    mem_target: i32,
    status: Option<EqualizerConfigStatus>,
}

impl std::fmt::Debug for EqualizerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The mocks/secrets/status are not all Debug; report the scenario figures.
        f.debug_struct("EqualizerWorld")
            .field("cpu_utils", &self.cpu_utils)
            .field("names", &self.names)
            .field("cpu_target", &self.cpu_target)
            .field(
                "fleet_condition",
                &self.status.as_ref().map(|s| s.condition),
            )
            .finish()
    }
}

impl Default for EqualizerWorld {
    fn default() -> Self {
        Self {
            cpu_utils: vec![],
            names: vec![],
            mocks: vec![],
            secrets: HashMap::new(),
            cpu_target: 80,
            mem_target: 80,
            status: None,
        }
    }
}

// ---- Given ----

#[given(regex = r"(\d+) target clusters with CPU utilization (.+)")]
async fn given_clusters(world: &mut EqualizerWorld, count: usize, utils: String) {
    common::ensure_crypto_provider();
    let cpu_utils: Vec<f64> = utils
        .split(',')
        .map(|s| {
            s.trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f64>()
                .unwrap_or_else(|e| panic!("invalid utilization `{s}`: {e}"))
        })
        .collect();
    assert_eq!(
        cpu_utils.len(),
        count,
        "the utilisation list length must match the stated cluster count"
    );

    let mut mocks = Vec::new();
    let mut secrets = HashMap::new();
    let mut names = Vec::new();
    for (i, util) in cpu_utils.iter().enumerate() {
        let name = char::from(b'a' + i as u8).to_string();
        // Memory utilization mirrors CPU (all under target) for this scenario.
        let mock = common::spawn_mock_target(*util, *util, 100_000, 200 * GIB).await;
        let secret_name = format!("target-{name}");
        let yaml = common::kubeconfig_yaml(&mock.addr);
        let secret = common::kubeconfig_secret_value(&secret_name, NS, &yaml);
        secrets.insert(secret_name, secret);
        names.push(name);
        mocks.push(mock);
    }
    world.cpu_utils = cpu_utils;
    world.names = names;
    world.mocks = mocks;
    world.secrets = secrets;
}

#[given(regex = r"the EqualizerConfig has cpuTargetBudgetPercent (\d+)")]
async fn cpu_target_budget(world: &mut EqualizerWorld, target: i32) {
    world.cpu_target = target;
    // Memory equalizes to the same target for this scenario (both under it).
    world.mem_target = target;
}

// ---- When ----

#[when(expr = "the equalizer reconciles the fleet")]
async fn reconcile_fleet(world: &mut EqualizerWorld) {
    let targets: Vec<TargetCluster> = world
        .names
        .iter()
        .map(|name| TargetCluster {
            name: name.clone(),
            kubeconfig_secret_ref: SecretRef {
                name: format!("target-{name}"),
                key: "kubeconfig".to_string(),
                namespace: NS.to_string(),
            },
        })
        .collect();
    let eq_config = EqualizerConfig::new(
        FLEET_EQUALIZER_NAME,
        EqualizerConfigSpec {
            cpu_target_budget_percent: world.cpu_target,
            memory_target_budget_percent: world.mem_target,
            targets,
        },
    );

    let (home_client, mut home_handle) = common::mock_home_client();
    let secret_count = world.secrets.len();
    let secrets = world.secrets.clone();
    let home_for_reconcile = home_client.clone();
    let task = tokio::spawn(async move { reconcile(&home_for_reconcile, &eq_config).await });

    // Serve the kubeconfig Secret GETs (arrival order is non-deterministic).
    for _ in 0..secret_count {
        let (req, respond) = home_handle.next_request().await.expect("a Secret GET");
        let name =
            common::secret_name_from_path(req.uri().path()).expect("Secret name in GET path");
        let secret = &secrets[name];
        respond.send_response(common::ok_object(secret));
    }

    world.status = Some(task.await.expect("reconcile completed without panicking"));
}

// ---- Then ----

#[then(regex = r"each cluster receives cpuBudgetPercent (\d+)")]
async fn each_cluster_cpu_budget(world: &mut EqualizerWorld, expected: i32) {
    let status = world.status.as_ref().expect("reconciled status available");
    for mock in &world.mocks {
        let patches = mock.patches();
        assert_eq!(patches.len(), 1, "one PATCH per cluster: {patches:?}");
        assert_eq!(
            patches[0]["spec"]["cpuBudgetPercent"].as_i64(),
            Some(expected as i64),
            "cpuBudgetPercent PATCHed to {expected}: {patches:?}"
        );
    }
    for obs in &status.clusters {
        assert_eq!(
            obs.computed_cpu_budget_percent, expected,
            "status reports the computed CPU budget"
        );
    }
}

#[then(regex = r"the fleet condition is (\w+)")]
async fn fleet_condition_is(world: &mut EqualizerWorld, condition: String) {
    let status = world.status.as_ref().expect("reconciled status available");
    let expected = match condition.as_str() {
        "Healthy" => FleetCondition::Healthy,
        "Compensating" => FleetCondition::Compensating,
        "Degraded" => FleetCondition::Degraded,
        other => panic!("unknown fleet condition `{other}`"),
    };
    assert_eq!(status.condition, expected);
}

#[tokio::main]
async fn main() {
    EqualizerWorld::run("tests/bdd/features/equalizer.feature").await;
}
