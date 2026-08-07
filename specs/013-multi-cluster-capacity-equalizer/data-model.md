# Phase 1 Data Model — Multi-Cluster Capacity Equalizer (spec-013)

**Date**: 2026-08-06

This feature adds a new CRD (`EqualizerConfig`), a new binary
(`capacity-equalizer`), and a pure equalization algorithm. No changes to existing
CRDs (Allocation, ClusterCapacity) — the equalizer reads their status and writes
their spec overrides (spec-012 fields).

---

## 1. Entities

### 1.1 EqualizerConfig CRD (NEW)

Cluster-scoped, singleton `fleet-equalizer`, `emergency-ration.dev/v1`.

#### Spec

```rust
// src/equalizer/crd.rs

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Singleton instance name enforced by convention (one per cluster running the equalizer).
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
pub struct EqualizerConfigSpec {
    /// Cumulative CPU budget target (0–100). The fleet average CPU utilization
    /// converges to this value. Each resource is equalized independently (FR-014).
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
    /// Human-readable cluster name. Must be unique within targets[].
    pub name: String,

    /// Reference to the Secret containing this cluster's kubeconfig (FR-003).
    pub kubeconfig_secret_ref: SecretRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Secret name.
    pub name: String,

    /// Key within the Secret whose value is the kubeconfig YAML.
    /// Default: "kubeconfig".
    #[serde(default = "default_kubeconfig_key")]
    pub key: String,

    /// Namespace where the Secret lives.
    pub namespace: String,
}

fn default_kubeconfig_key() -> String {
    "kubeconfig".to_string()
}
```

Serialisation: camelCase. `cpuTargetBudgetPercent`, `memoryTargetBudgetPercent`,
`targets` are required. `targets[].kubeconfigSecretRef.key` defaults to
`"kubeconfig"`.

#### Status

```rust
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
    pub name: String,

    /// Observed CPU utilization (from Allocation.status.utilizationPercentCpu).
    pub cpu_utilization_percent: f64,

    /// Observed memory utilization.
    pub memory_utilization_percent: f64,

    /// Observed total allocatable CPU, milli (from ClusterCapacity.status).
    pub total_allocatable_cpu_milli: i64,

    /// Observed total allocatable memory, bytes.
    pub total_allocatable_memory_bytes: i64,

    /// Computed CPU budget the equalizer applied (or would apply).
    pub computed_cpu_budget_percent: i32,

    /// Computed memory budget.
    pub computed_memory_budget_percent: i32,

    /// Cluster state in the equalization.
    pub state: ClusterState,

    /// Last error (if Unreachable or ConfigError).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    /// Timestamp of last successful observation of this cluster.
    pub last_observed: String,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FleetCondition {
    /// All clusters at or below target.
    Healthy,
    /// At least one cluster over target; others compensating.
    Compensating,
    /// One or more clusters unreachable or in config error.
    Degraded,
}
```

### 1.2 Existing CRDs (consumed, unchanged)

- **Allocation** (`emergency-ration.dev/v1`): the equalizer reads
  `status.utilizationPercentCpu/Memory` and writes
  `spec.cpuBudgetPercent/memoryBudgetPercent` (spec-012 override fields). The
  equalizer does NOT touch `spec.budgetPercent` (FR-007).
- **ClusterCapacity** (`emergency-ration.dev/v1`): the equalizer reads
  `status.totalAllocatableCpuMilli/MemoryBytes`. Read-only.

---

## 2. The equalization algorithm (pure function)

### 2.1 Types

```rust
// src/equalizer/algorithm.rs

/// Input: one cluster's observed state for ONE resource dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterResourceObservation {
    pub name: String,
    pub utilization_percent: f64,
    pub total_allocatable: i64, // CPU milli or RAM bytes
}

/// The state of a cluster in the equalization for one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// At or below target; receiving a computed (possibly reduced) budget.
    Good,
    /// Over target; frozen at current utilization.
    Over,
}

/// Output: the computed budget for one cluster + one resource.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedBudget {
    pub name: String,
    pub budget_percent: i32,
    pub state: BudgetState,
}
```

### 2.2 Algorithm

```rust
/// The pure equalization algorithm for a SINGLE resource dimension (CPU or RAM).
///
/// Steps (data-model.md §2.2, FR-005, research R5):
/// 1. Partition: clusters with utilization > target are "over"; the rest are "good".
/// 2. Freeze over-clusters at floor(utilization_percent).
/// 3. Compute total absolute overflow = Σ (over: (util% − target) × allocatable / 100).
/// 4. If good_count == 0: return (all frozen, no compensation).
/// 5. Per good-cluster: budget = target − floor(overflow_abs / good_count
///    / good_cluster_allocatable × 100). Clamp to [0, 100].
///
/// Each resource dimension is equalized independently (FR-014).
pub fn equalize(
    observations: &[ClusterResourceObservation],
    target_budget_percent: i32,
) -> Vec<ComputedBudget> {
    let target = target_budget_percent as f64;

    let (over, good): (Vec<_>, Vec<_>) = observations
        .iter()
        .cloned()
        .partition(|o| o.utilization_percent > target);

    // Freeze over-clusters at their current utilization.
    let mut results: Vec<ComputedBudget> = over
        .iter()
        .map(|o| ComputedBudget {
            name: o.name.clone(),
            budget_percent: o.utilization_percent.floor().clamp(0.0, 100.0) as i32,
            state: BudgetState::Over,
        })
        .collect();

    if good.is_empty() {
        // No good clusters to compensate (US2 AC3 / edge "all over").
        return results;
    }

    // Total absolute overflow from over-clusters.
    let overflow_abs: i128 = over
        .iter()
        .map(|o| {
            let pct_overflow = (o.utilization_percent - target).max(0.0);
            (pct_overflow * o.total_allocatable as f64 / 100.0) as i128
        })
        .sum();

    let good_count = good.len() as i128;

    // Distribute overflow equally among good clusters (absolute units).
    for g in &good {
        let overflow_share_abs = overflow_abs / good_count; // integer division (floor)
        // Convert back to percentage points using THIS good cluster's capacity.
        let pct_reduction = if g.total_allocatable > 0 {
            (overflow_share_abs * 100 / g.total_allocatable as i128) as i32
        } else {
            0 // zero-capacity cluster: no reduction (edge case).
        };
        let budget = (target_budget_percent - pct_reduction).clamp(0, 100);
        results.push(ComputedBudget {
            name: g.name.clone(),
            budget_percent: budget,
            state: BudgetState::Good,
        });
    }

    results
}
```

### 2.3 Worked examples (truth table for unit tests)

**Example 1 — All under target (US1 AC1)**:
Target 80%, 3 clusters × 100_000m CPU, util 65/55/45.
- Over: none. Good: A, B, C.
- Overflow: 0. Per-good reduction: 0. Budgets: 80/80/80. ✅

**Example 2 — One over (US2 AC1)**:
Target 80%, 3 clusters × 100_000m, util 65/55/90.
- Over: C (90%). Overflow = (90−80)×100_000/100 = 10_000m.
- Good: A, B (count 2). Share = 10_000/2 = 5_000m each.
- A reduction = 5_000×100/100_000 = 5. Budget = 80−5 = 75.
- B reduction = same = 5. Budget = 75.
- C frozen at 90.
- Budgets: 75/75/90. Fleet avg = 80. ✅

**Example 3 — Over drops (US2 AC2)**:
Same setup, C drops to 86%.
- Overflow = (86−80)×100_000/100 = 6_000m.
- Share = 6_000/2 = 3_000m. Reduction = 3_000×100/100_000 = 3.
- Budgets: 77/77/86. Avg = 80. ✅
- (Note: spec AC2 says "78" — that's a specify-phase arithmetic typo. The correct
  value is 77. Tasks phase must encode 77, the algorithm-verified value.)

**Example 4 — All over (US2 AC3)**:
Target 80%, util 85/85/85.
- Over: all. Good: none. All frozen at 85. ✅

**Example 5 — Non-uniform capacity**:
Target 80%, A=100_000m util 60%, B=200_000m util 60%, C=200_000m util 95%.
- Over: C (95%). Overflow = (95−80)×200_000/100 = 30_000m.
- Good: A, B (count 2). Share = 30_000/2 = 15_000m.
- A reduction = 15_000×100/100_000 = 15. Budget = 80−15 = 65.
- B reduction = 15_000×100/200_000 = 7 (floor). Budget = 80−7 = 73.
- C frozen at 95.
- Budgets: 65/73/95.
- Verify: A limit = 65_000m, B limit = 146_000m, C limit = 190_000m.
  Total limit = 401_000m. Total alloc = 500_000m. Fleet budget = 80.2% ≈ 80% ✅
  (flooring makes it slightly under 80%, which is conservative — correct).

---

## 3. Reconcile loop (state machine)

```
Every 10s (configurable):

  1. READ EqualizerConfig spec (targets + targets + budget targets)
     │
  2. FOR EACH target cluster (concurrent via tokio::join_all):
     │  a. READ kubeconfig Secret from home cluster
     │  b. CONSTRUCT kube::Client from kubeconfig bytes
     │  c. GET Allocation singleton → read status.utilizationPercentCpu/Memory
     │  d. GET ClusterCapacity singleton → read status.totalAllocatable*
     │  e. RECORD ClusterObservation (or Unreachable/ConfigError on failure)
     │
  3. COMPUTE: equalize(cpu_observations, cpu_target) → cpu_budgets
              equalize(mem_observations, mem_target) → mem_budgets
     │
  4. FOR EACH reachable cluster (concurrent):
     │  PATCH Allocation.spec with computed cpuBudgetPercent + memoryBudgetPercent
     │  (strategic merge patch — only sets the override fields, leaves budgetPercent)
     │
  5. WRITE EqualizerConfig.status (per-cluster observations + fleet condition + timestamp)
```

**State transitions per cluster**:
```
                    ┌──────────────────────────────────┐
                    ▼                                  │
  ConfigError ──► Unreachable ──► Healthy ──► Over ──┘
  (Secret        (API timeout    (read OK,    (util >
   bad)           or error)       under tgt)   tgt)
                    ▲                                  │
                    └─── (API error / timeout) ───────┘
```

- `Healthy → Over`: utilization rises above target (next cycle detects).
- `Over → Healthy`: utilization drops to/below target (next cycle detects).
- Any state → `Unreachable`: API error.
- Any state → `ConfigError`: Secret missing/malformed.
- `Unreachable/ConfigError → Healthy/Over`: next successful read classifies.

---

## 4. Validation rules

| Rule | Enforced by | FR |
|------|-------------|----|
| `cpuTargetBudgetPercent` ∈ [0, 100] | `#[schemars(range)]` → CRD schema → apiserver | FR-002 |
| `memoryTargetBudgetPercent` ∈ [0, 100] | same | FR-002 |
| `targets` non-empty (≥1) | schema `minItems: 1` (or runtime validation) | FR-003 |
| `targets[].name` unique | runtime validation in reconcile (log + status error if dup) | FR-003 |
| Algorithm budgets clamped to [0, 100] | `clamp(0, 100)` in `equalize()` | FR-005 |
| Only override fields patched (not `budgetPercent`) | strategic merge patch with only `cpuBudgetPercent`/`memoryBudgetPercent` keys | FR-007 |
| Unreachable cluster skipped | reconcile loop checks `ClusterState` before patching | FR-009 |

---

## 5. No changes to existing CRDs

The Allocation and ClusterCapacity CRDs are UNCHANGED. The equalizer is a
consumer of their status and a writer of the spec-012 override fields. No CRD
migration, no schema version bump. The equalizer depends on spec-012 being
installed in every target cluster (Assumptions in the spec).
