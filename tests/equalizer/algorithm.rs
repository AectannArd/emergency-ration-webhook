//! Unit tests for the multi-cluster capacity equalizer's pure + foundational
//! layer (spec-013, Phases 2).
//!
//! Covers the `EqualizerConfig` CRD serialisation/identity (T003–T006), the pure
//! `equalize()` algorithm truth table + edge cases (T007–T015), the
//! `fleet_condition()` aggregator (T016), and `build_target_client` (T017). Run
//! with `cargo test --test algorithm`.

use capacity_admission_webhook::equalizer::algorithm::{
    BudgetState, ClusterResourceObservation, ComputedBudget, equalize,
};
use capacity_admission_webhook::equalizer::cluster_client::build_target_client;
use capacity_admission_webhook::equalizer::crd::{
    ClusterObservation, ClusterState, EqualizerConfig, EqualizerConfigSpec, EqualizerConfigStatus,
    FLEET_EQUALIZER_NAME, FleetCondition, SecretRef, TargetCluster, fleet_condition,
};
use kube::CustomResourceExt;

const GIB: i64 = 1024 * 1024 * 1024;

// ============================================================================
// T003–T006: EqualizerConfig CRD identity + serialisation
// ============================================================================

#[test]
fn equalizer_config_crd_identity() {
    // T003: the CRD object has the contract identity — name, cluster scope, kind,
    // short name `eqconf`, and a declared status subresource.
    let crd = EqualizerConfig::crd();
    assert_eq!(
        crd.metadata.name.as_deref(),
        Some("equalizerconfigs.emergency-ration.dev"),
        "CRD name is <plural>.<group>"
    );
    assert_eq!(
        crd.spec.scope, "Cluster",
        "EqualizerConfig is cluster-scoped"
    );
    assert_eq!(crd.spec.names.kind, "EqualizerConfig");
    let short: Vec<&str> = crd
        .spec
        .names
        .short_names
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    assert_eq!(short, vec!["eqconf"], "short name is eqconf");
    let has_status = crd.spec.versions[0]
        .subresources
        .as_ref()
        .map(|s| s.status.is_some())
        .unwrap_or(false);
    assert!(has_status, "status subresource is declared");
    // The singleton instance name constant matches the contract.
    assert_eq!(FLEET_EQUALIZER_NAME, "fleet-equalizer");
}

#[test]
fn equalizer_config_spec_serialises_camel_case_round_trips_and_range() {
    // T004: spec fields serialise camelCase, round-trip, and the budget fields
    // carry #[schemars(range(min=0,max=100))] into the CRD schema.
    let spec = EqualizerConfigSpec {
        cpu_target_budget_percent: 80,
        memory_target_budget_percent: 70,
        targets: vec![TargetCluster {
            name: "home".to_string(),
            kubeconfig_secret_ref: SecretRef {
                name: "home-kubeconfig".to_string(),
                key: "kubeconfig".to_string(),
                namespace: "fleet-equalizer".to_string(),
            },
        }],
    };
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        json.get("cpuTargetBudgetPercent").and_then(|v| v.as_i64()),
        Some(80),
        "cpuTargetBudgetPercent serialises camelCase: {json}"
    );
    assert_eq!(
        json.get("memoryTargetBudgetPercent")
            .and_then(|v| v.as_i64()),
        Some(70),
        "memoryTargetBudgetPercent serialises camelCase: {json}"
    );
    let targets = json
        .get("targets")
        .and_then(|v| v.as_array())
        .expect("targets is an array");
    assert_eq!(targets.len(), 1);
    assert_eq!(
        targets[0].get("name").and_then(|v| v.as_str()),
        Some("home")
    );
    assert!(
        targets[0].get("kubeconfigSecretRef").is_some(),
        "kubeconfigSecretRef serialises camelCase: {json}"
    );

    // Round-trips through serde.
    let back: EqualizerConfigSpec = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.cpu_target_budget_percent, 80);
    assert_eq!(back.memory_target_budget_percent, 70);
    assert_eq!(back.targets[0].name, "home");
    assert_eq!(
        back.targets[0].kubeconfig_secret_ref.name,
        "home-kubeconfig"
    );

    // Range constraints land in the generated CRD schema.
    let crd_v = serde_json::to_value(EqualizerConfig::crd()).unwrap();
    for field in ["cpuTargetBudgetPercent", "memoryTargetBudgetPercent"] {
        let schema = crd_v
            .pointer(&format!(
                "/spec/versions/0/schema/openAPIV3Schema/properties/spec/properties/{field}"
            ))
            .unwrap_or_else(|| panic!("{field} schema present"));
        assert_eq!(
            schema.get("minimum").and_then(|m| m.as_f64()),
            Some(0.0),
            "{field} has minimum 0: {schema}"
        );
        assert_eq!(
            schema.get("maximum").and_then(|m| m.as_f64()),
            Some(100.0),
            "{field} has maximum 100: {schema}"
        );
    }
}

#[test]
fn secret_ref_key_defaults_to_kubeconfig_when_absent() {
    // T005: a SecretRef without an explicit `key` deserialises with key =
    // "kubeconfig" (contract §2.3.2.2); an explicit key is honoured.
    let without_key = serde_json::json!({"name": "s", "namespace": "ns"});
    let sr: SecretRef = serde_json::from_value(without_key).unwrap();
    assert_eq!(
        sr.key, "kubeconfig",
        "absent key defaults to \"kubeconfig\""
    );
    assert_eq!(sr.name, "s");
    assert_eq!(sr.namespace, "ns");

    let with_key = serde_json::json!({"name": "s", "namespace": "ns", "key": "admin.conf"});
    let sr2: SecretRef = serde_json::from_value(with_key).unwrap();
    assert_eq!(sr2.key, "admin.conf", "explicit key is preserved");
}

#[test]
fn equalizer_config_status_serialises_camel_case_and_enums_kebab() {
    // T006: status fields serialise camelCase; ClusterState / FleetCondition
    // serialise kebab-case; lastError is absent when None.
    let status = EqualizerConfigStatus {
        clusters: vec![ClusterObservation {
            name: "home".to_string(),
            cpu_utilization_percent: 65.0,
            memory_utilization_percent: 55.0,
            total_allocatable_cpu_milli: 100_000,
            total_allocatable_memory_bytes: 200 * GIB,
            computed_cpu_budget_percent: 80,
            computed_memory_budget_percent: 80,
            state: ClusterState::Healthy,
            last_error: None,
            last_observed: "2026-08-06T00:00:00Z".to_string(),
        }],
        condition: FleetCondition::Healthy,
        last_reconciled: "2026-08-06T00:00:00Z".to_string(),
    };
    let json = serde_json::to_value(&status).unwrap();
    assert!(json.get("clusters").is_some_and(|v| v.is_array()));
    assert_eq!(
        json.get("condition").and_then(|v| v.as_str()),
        Some("healthy")
    );
    assert!(
        json.get("lastReconciled").is_some(),
        "lastReconciled camelCase"
    );

    let cluster = &json["clusters"][0];
    assert_eq!(
        cluster
            .get("cpuUtilizationPercent")
            .and_then(|v| v.as_f64()),
        Some(65.0)
    );
    assert_eq!(
        cluster
            .get("totalAllocatableCpuMilli")
            .and_then(|v| v.as_i64()),
        Some(100_000)
    );
    assert_eq!(
        cluster
            .get("computedMemoryBudgetPercent")
            .and_then(|v| v.as_i64()),
        Some(80)
    );
    assert_eq!(
        cluster.get("state").and_then(|v| v.as_str()),
        Some("healthy")
    );
    assert!(
        cluster.get("lastError").is_none(),
        "lastError absent when None (skip_serializing_if)"
    );

    // ClusterState serialises kebab-case for every variant and round-trips.
    assert_eq!(
        serde_json::to_string(&ClusterState::Over).unwrap(),
        r#""over""#
    );
    assert_eq!(
        serde_json::to_string(&ClusterState::Unreachable).unwrap(),
        r#""unreachable""#
    );
    assert_eq!(
        serde_json::to_string(&ClusterState::ConfigError).unwrap(),
        r#""config-error""#
    );
    let recovered: ClusterState = serde_json::from_str(r#""config-error""#).unwrap();
    assert_eq!(recovered, ClusterState::ConfigError);

    // FleetCondition serialises kebab-case for every variant.
    assert_eq!(
        serde_json::to_string(&FleetCondition::Compensating).unwrap(),
        r#""compensating""#
    );
    assert_eq!(
        serde_json::to_string(&FleetCondition::Degraded).unwrap(),
        r#""degraded""#
    );
}

#[test]
fn fleet_condition_aggregation_by_severity() {
    // T016: any Unreachable/ConfigError → Degraded (highest severity); else any
    // Over → Compensating; else Healthy.
    use ClusterState::*;
    assert_eq!(
        fleet_condition(&[]),
        FleetCondition::Healthy,
        "empty → Healthy"
    );
    assert_eq!(
        fleet_condition(&[Healthy, Healthy, Healthy]),
        FleetCondition::Healthy
    );
    assert_eq!(
        fleet_condition(&[Healthy, Over, Healthy]),
        FleetCondition::Compensating,
        "one Over → Compensating"
    );
    assert_eq!(
        fleet_condition(&[Healthy, Unreachable]),
        FleetCondition::Degraded,
        "one Unreachable → Degraded"
    );
    assert_eq!(
        fleet_condition(&[Over, Unreachable]),
        FleetCondition::Degraded,
        "Over + Unreachable → Degraded (highest severity)"
    );
    assert_eq!(
        fleet_condition(&[Healthy, ConfigError]),
        FleetCondition::Degraded,
        "ConfigError → Degraded"
    );
    // Degraded wins over Compensating regardless of ordering.
    assert_eq!(
        fleet_condition(&[Unreachable, Over]),
        FleetCondition::Degraded
    );
}

// ============================================================================
// T007–T015: the pure equalize() algorithm (data-model.md §2.3 truth table)
// ============================================================================

/// Build one cluster's observation for a single resource dimension.
fn obs(name: &str, utilization_percent: f64, total_allocatable: i64) -> ClusterResourceObservation {
    ClusterResourceObservation {
        name: name.to_string(),
        utilization_percent,
        total_allocatable,
    }
}

/// Look up a cluster's `(budget_percent, state)` from the equalize() result,
/// panicking if the cluster is absent (so a missing result is a loud failure).
fn budget_for(results: &[ComputedBudget], name: &str) -> (i32, BudgetState) {
    let b = results
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no computed budget for cluster `{name}`"));
    (b.budget_percent, b.state)
}

#[test]
fn equalize_all_under_target() {
    // Example 1 (US1 AC1): target 80, util 65/55/45, uniform 100_000m → 80/80/80.
    let observations = vec![
        obs("a", 65.0, 100_000),
        obs("b", 55.0, 100_000),
        obs("c", 45.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(res.len(), 3);
    for name in ["a", "b", "c"] {
        assert_eq!(budget_for(&res, name), (80, BudgetState::Good), "{name}");
    }
}

#[test]
fn equalize_one_over() {
    // Example 2 (US2 AC1): target 80, util 65/55/90 → 75/75/90. Over-cluster C
    // frozen at 90; good clusters A/B each absorb 5pp of C's overflow.
    let observations = vec![
        obs("a", 65.0, 100_000),
        obs("b", 55.0, 100_000),
        obs("c", 90.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(budget_for(&res, "a"), (75, BudgetState::Good));
    assert_eq!(budget_for(&res, "b"), (75, BudgetState::Good));
    assert_eq!(budget_for(&res, "c"), (90, BudgetState::Over));
}

#[test]
fn equalize_over_drops() {
    // Example 3 (US2 AC2): C drops 90→86. Overflow halves → good budgets 77.
    // (The spec's AC2 value "78" is a specify-phase arithmetic typo; 77 is the
    // algorithm-verified value — research R6.)
    let observations = vec![
        obs("a", 65.0, 100_000),
        obs("b", 55.0, 100_000),
        obs("c", 86.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(budget_for(&res, "a"), (77, BudgetState::Good));
    assert_eq!(budget_for(&res, "b"), (77, BudgetState::Good));
    assert_eq!(budget_for(&res, "c"), (86, BudgetState::Over));
}

#[test]
fn equalize_all_over() {
    // Example 4 (US2 AC3): all over → all frozen at floor(utilization); no good
    // clusters to compensate.
    let observations = vec![
        obs("a", 85.0, 100_000),
        obs("b", 85.0, 100_000),
        obs("c", 85.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(res.len(), 3);
    for name in ["a", "b", "c"] {
        assert_eq!(budget_for(&res, name), (85, BudgetState::Over), "{name}");
    }
}

#[test]
fn equalize_non_uniform_capacity() {
    // Example 5: different cluster sizes. C over at 95% (200_000m) overflows
    // 30_000m; split 2 ways = 15_000m each. Small A absorbs it as 15pp (→65),
    // large B as 7pp (→73). C frozen at 95. Absolute-units distribution.
    let observations = vec![
        obs("a", 60.0, 100_000),
        obs("b", 60.0, 200_000),
        obs("c", 95.0, 200_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(budget_for(&res, "a"), (65, BudgetState::Good));
    assert_eq!(budget_for(&res, "b"), (73, BudgetState::Good));
    assert_eq!(budget_for(&res, "c"), (95, BudgetState::Over));
}

#[test]
fn equalize_single_cluster() {
    // A single under-target cluster → budget = target (nothing to compensate).
    let under = equalize(&[obs("solo", 50.0, 100_000)], 80);
    assert_eq!(budget_for(&under, "solo"), (80, BudgetState::Good));
    // A single over-target cluster → frozen at floor(utilization), no good clusters.
    let over = equalize(&[obs("solo", 90.0, 100_000)], 80);
    assert_eq!(over.len(), 1, "only the frozen over-cluster is returned");
    assert_eq!(budget_for(&over, "solo"), (90, BudgetState::Over));
}

#[test]
fn equalize_zero_capacity_cluster() {
    // A zero-capacity cluster is benign: as an over-cluster it contributes no
    // absolute overflow; as a good cluster it gets budget = target (its zero
    // capacity never reaches the percentage-reduction divisor).
    let observations = vec![
        obs("over", 90.0, 100_000),
        obs("zero-over", 95.0, 0),
        obs("zero-good", 60.0, 0),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(budget_for(&res, "over"), (90, BudgetState::Over));
    assert_eq!(
        budget_for(&res, "zero-over"),
        (95, BudgetState::Over),
        "zero-cap over cluster frozen at floor(util)"
    );
    assert_eq!(
        budget_for(&res, "zero-good"),
        (80, BudgetState::Good),
        "zero-cap good cluster: budget = target, no div-by-zero"
    );
}

#[test]
fn equalize_multiple_over() {
    // Two over clusters (90/90) + two good (60/60), uniform 100_000m, target 80.
    // Combined overflow = 20_000m; split 2 ways = 10_000m each → 10pp → budget 70.
    let observations = vec![
        obs("o1", 90.0, 100_000),
        obs("o2", 90.0, 100_000),
        obs("g1", 60.0, 100_000),
        obs("g2", 60.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(budget_for(&res, "o1"), (90, BudgetState::Over));
    assert_eq!(budget_for(&res, "o2"), (90, BudgetState::Over));
    assert_eq!(budget_for(&res, "g1"), (70, BudgetState::Good));
    assert_eq!(budget_for(&res, "g2"), (70, BudgetState::Good));
}

#[test]
fn equalize_over_to_good_transition() {
    // A cluster that was Over at 90% last cycle drops to 70% (under target). This
    // cycle it is classified Good with budget = target — the algorithm has no
    // memory of the prior Over state.
    let observations = vec![
        obs("a", 65.0, 100_000),
        obs("b", 55.0, 100_000),
        obs("c", 70.0, 100_000),
    ];
    let res = equalize(&observations, 80);
    assert_eq!(
        budget_for(&res, "c"),
        (80, BudgetState::Good),
        "dropped under target → Good, budget = target"
    );
    for name in ["a", "b", "c"] {
        assert_eq!(budget_for(&res, name), (80, BudgetState::Good), "{name}");
    }
}

// ============================================================================
// T017: build_target_client — construct a kube::Client from kubeconfig bytes
// ============================================================================

/// A minimal, well-formed kubeconfig (plain HTTP, no TLS) used as the test
/// fixture. `Config::from_custom_kubeconfig` does not connect, so the server URL
/// need not resolve.
const TEST_KUBECONFIG: &[u8] = br#"
apiVersion: v1
kind: Config
clusters:
- name: test
  cluster:
    server: http://127.0.0.1:1
contexts:
- name: test
  context:
    cluster: test
    user: test
current-context: test
users:
- name: test
  user: {}
"#;

/// Install the rustls ring CryptoProvider idempotently. `Config`/`Client`
/// construction touches rustls even for plain-HTTP kubeconfigs, so the provider
/// must be set before [`build_target_client`] (the binary does this as its first
/// line; tests must too). `install_default` errors if a provider is already
/// installed, which we ignore.
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn build_target_client_constructs_from_valid_kubeconfig() {
    // T017: valid kubeconfig YAML → Ok(kube::Client). from_custom_kubeconfig does
    // not perform any network I/O, so construction succeeds offline.
    ensure_crypto_provider();
    let client = build_target_client(TEST_KUBECONFIG)
        .await
        .expect("a valid kubeconfig must build a client");
    // Sanity: the client is usable to construct an Api (still no connection).
    use capacity_admission_webhook::crd::Allocation;
    let _api = kube::Api::<Allocation>::all(client);
}

#[tokio::test]
async fn build_target_client_errors_on_invalid_input() {
    // Invalid UTF-8 fails at the first step (no TLS provider involved).
    let result = build_target_client(&[0xff, 0xfe, 0xfd]).await;
    assert!(result.is_err(), "invalid UTF-8 kubeconfig must error");
}
