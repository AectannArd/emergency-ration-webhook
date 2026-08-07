//! `EqualizerConfig` CRD — the fleet-wide configuration the multi-cluster
//! capacity equalizer reads (spec-013).
//!
//! Cluster-scoped, singleton instance `fleet-equalizer` (`emergency-ration.dev/v1`).
//! The spec carries the per-resource budget targets + the list of target clusters
//! (each identified by a kubeconfig `Secret` reference); the status carries the
//! per-cluster observations + fleet condition the reconcile loop writes. Where
//! this file disagrees with `data-model.md`, `contracts/equalizer-config-crd.md`
//! wins (it is the authoritative CRD contract).

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Singleton instance name enforced by convention (one per cluster running the
/// equalizer). Mirrors the existing `cluster-allocation` / `cluster-capacity`
/// singleton convention.
pub const FLEET_EQUALIZER_NAME: &str = "fleet-equalizer";

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "emergency-ration.dev",
    version = "v1",
    kind = "EqualizerConfig",
    status = "EqualizerConfigStatus",
    shortname = "eqconf"
)]
// Cluster-scoped: the `namespaced` flag is intentionally omitted (its absence
// means `scope: Cluster`), matching contract §1 and research R2.
/// Spec of the EqualizerConfig CRD. The operator-configurable budget targets +
/// target cluster list.
pub struct EqualizerConfigSpec {
    /// Cumulative CPU budget target (0–100). The fleet average CPU utilization
    /// converges to this value. Each resource is equalized independently
    /// (FR-014).
    #[schemars(range(min = 0, max = 100))]
    pub cpu_target_budget_percent: i32,

    /// Cumulative memory budget target (0–100). Independent from CPU.
    #[schemars(range(min = 0, max = 100))]
    pub memory_target_budget_percent: i32,

    /// Target cluster definitions. Every cluster — including the one the
    /// equalizer runs in — is specified here (FR-003). Minimum 1 entry.
    pub targets: Vec<TargetCluster>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetCluster {
    /// Human-readable cluster name. Must be unique within `targets[]`.
    pub name: String,

    /// Reference to the Secret containing this cluster's kubeconfig (FR-003).
    pub kubeconfig_secret_ref: SecretRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Secret name.
    pub name: String,

    /// Key within the Secret whose value is the kubeconfig YAML. Defaults to
    /// `"kubeconfig"` (contract §2.3.2.2).
    #[serde(default = "default_kubeconfig_key")]
    pub key: String,

    /// Namespace where the Secret lives (typically the equalizer's namespace).
    pub namespace: String,
}

/// Default value for [`SecretRef::key`] when the field is absent.
fn default_kubeconfig_key() -> String {
    "kubeconfig".to_string()
}

/// Status of the EqualizerConfig CRD, populated by the reconcile loop.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EqualizerConfigStatus {
    /// Per-cluster observations from the last reconcile cycle (FR-010).
    pub clusters: Vec<ClusterObservation>,

    /// Overall fleet condition (FR-011).
    pub condition: FleetCondition,

    /// Timestamp of the last successful reconcile cycle (RFC 3339).
    pub last_reconciled: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterObservation {
    /// Cluster name (matches `spec.targets[].name`).
    pub name: String,

    /// Observed CPU utilization (from `Allocation.status.utilizationPercentCpu`).
    pub cpu_utilization_percent: f64,

    /// Observed memory utilization.
    pub memory_utilization_percent: f64,

    /// Observed total allocatable CPU, milli (from `ClusterCapacity.status`).
    pub total_allocatable_cpu_milli: i64,

    /// Observed total allocatable memory, bytes.
    pub total_allocatable_memory_bytes: i64,

    /// Computed CPU budget the equalizer applied (or would apply if reachable).
    pub computed_cpu_budget_percent: i32,

    /// Computed memory budget.
    pub computed_memory_budget_percent: i32,

    /// Cluster state in the equalization.
    pub state: ClusterState,

    /// Last error message (present iff `state` is `Unreachable` or `ConfigError`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    /// Timestamp (RFC 3339) of the last successful observation of this cluster.
    pub last_observed: String,
}

/// A cluster's state within one reconcile cycle. Serialised kebab-case for clean
/// `kubectl` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClusterState {
    /// At or below its computed budget (good-state).
    Healthy,
    /// Over the target; frozen at current utilization.
    Over,
    /// API server unreachable; budget left at last-known value.
    Unreachable,
    /// Kubeconfig Secret missing or malformed.
    ConfigError,
}

/// Overall fleet condition, the highest-severity state across all clusters
/// (FR-011). Serialised kebab-case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FleetCondition {
    /// All clusters at or below their computed budgets; no over-cluster
    /// compensation active.
    #[default]
    Healthy,
    /// At least one cluster is over target; others are compensating.
    Compensating,
    /// One or more clusters are unreachable or in config error.
    Degraded,
}

/// Aggregate per-cluster states into the overall fleet condition (FR-011).
///
/// Severity ordering: any `Unreachable`/`ConfigError` → [`FleetCondition::Degraded`]
/// (a fleet with an unreachable cluster cannot be considered healthy, even if the
/// reachable clusters are all within budget); else any `Over` →
/// [`FleetCondition::Compensating`]; else [`FleetCondition::Healthy`]. Pure — no
/// I/O — so it is unit-tested in isolation and reused by the reconcile loop.
pub fn fleet_condition(states: &[ClusterState]) -> FleetCondition {
    if states
        .iter()
        .any(|s| matches!(s, ClusterState::Unreachable | ClusterState::ConfigError))
    {
        FleetCondition::Degraded
    } else if states.iter().any(|s| matches!(s, ClusterState::Over)) {
        FleetCondition::Compensating
    } else {
        FleetCondition::Healthy
    }
}
