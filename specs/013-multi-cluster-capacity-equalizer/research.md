# Phase 0 Research — Multi-Cluster Capacity Equalizer (spec-013)

**Date**: 2026-08-06

This feature introduces a new binary, new CRD, and a multi-cluster control loop
on a codebase the planning agent has fully mapped (specs 001–012 delivered). The
research items below resolve the architectural and algorithmic decisions the plan
must lock, grounded in the actual source.

---

## R1 — Multi-cluster kube::Client construction from Secret-mounted kubeconfigs

**Decision**: The equalizer reads each target cluster's kubeconfig bytes from the
referenced Kubernetes Secret (in the equalizer's own cluster), parses them via
`kube::config::Kubeconfig::read_from_yaml()` (or `read_from` on the bytes), and
constructs a `kube::Client` via
`Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default())` — the exact
pattern already proven in `src/bin/erw-verify/client.rs:19-35`.

```rust
// src/equalizer/cluster_client.rs (sketch)
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};

pub async fn build_target_client(kubeconfig_bytes: &[u8]) -> Result<Client> {
    let kc = Kubeconfig::read_from_yaml(std::str::from_utf8(kubeconfig_bytes)?)?;
    let config = Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default()).await?;
    Ok(Client::try_from(config)?)
}
```

**Rationale**: kube-rs already handles all the kubeconfig complexity (TLS, auth
provider plugins, cluster URL, CA cert). The erw-verify binary uses this exact
API to build a client from a file path; the equalizer does the same but from
Secret bytes instead of a file. No new crate, no new TLS handling. The
CryptoProvider (rustls ring) MUST be installed before any client construction —
the binary's `main.rs` calls `rustls::crypto::ring::default_provider().install_default()`
as its first line (same fix as the webhook binary, documented in the CI failure
catalog Layer 2).

**Alternatives considered**:
- Store the kubeconfig as a file on disk + use `Kubeconfig::read_from(path)` —
  rejected: requires a volume mount per target, and the Secret bytes are already
  in the API server; reading them programmatically avoids the mount indirection.
- Use in-cluster SA for the home cluster + Secrets for remotes — rejected: user
  clarification C5 mandates all clusters (including home) via kubeconfig Secret,
  for uniformity.

---

## R2 — EqualizerConfig CRD: singleton + scope

**Decision**: Cluster-scoped CRD (`emergency-ration.dev/v1`, kind
`EqualizerConfig`), singleton instance `fleet-equalizer` (one per cluster that
runs the equalizer). The singleton convention mirrors the existing
`cluster-allocation` and `cluster-capacity` singletons.

**Rationale**: the equalizer manages fleet-wide state — a single config object is
the natural model. Cluster-scoped (not namespaced) because the equalizer reaches
across clusters; a namespaced singleton would imply per-namespace isolation that
does not exist in the feature's semantics. `kube::CustomResource` derive with no
`namespaced` flag gives cluster scope (same as the existing CRDs).

**Alternatives considered**:
- Multiple EqualizerConfig instances (one per "fleet group") — rejected: YAGNI
  for v1. One equalizer manages one fleet. Can be revisited if operators ask for
  partitioned fleets.
- Namespaced — rejected: the kubeconfig Secrets live in a namespace, but the
  config object itself is cluster-scoped (it references Secrets by name +
  namespace, so it can reach across namespaces).

---

## R3 — EqualizerConfig spec schema

**Decision**:

```rust
pub struct EqualizerConfigSpec {
    /// Cumulative CPU budget target (0–100). The fleet average utilization
    /// converges to this value.
    #[schemars(range(min = 0, max = 100))]
    pub cpu_target_budget_percent: i32,

    /// Cumulative memory budget target (0–100). Independent from CPU.
    #[schemars(range(min = 0, max = 100))]
    pub memory_target_budget_percent: i32,

    /// Target cluster definitions. Each cluster — including the one the
    /// equalizer runs in — is identified by a kubeconfig Secret reference.
    pub targets: Vec<TargetCluster>,
}

pub struct TargetCluster {
    /// Human-readable cluster name (unique within targets[]).
    pub name: String,

    /// Reference to the Secret containing this cluster's kubeconfig.
    pub kubeconfig_secret_ref: SecretRef,
}

pub struct SecretRef {
    /// Secret name.
    pub name: String,

    /// Key within the Secret whose value is the kubeconfig YAML (default: "kubeconfig").
    pub key: String,

    /// Namespace where the Secret lives (typically the equalizer's namespace).
    pub namespace: String,
}
```

Serialisation: camelCase (`cpuTargetBudgetPercent`, `memoryTargetBudgetPercent`,
`targets`, `kubeconfigSecretRef`). All required (no `Option`) except the default
on `key`.

**Rationale**: per-resource targets for consistency with spec-012 (the equalizer
writes per-resource overrides downstream). `targets` is a list (not a map) to
preserve operator-specified ordering (though the algorithm is order-independent).
The SecretRef includes `namespace` because the kubeconfig Secrets may live in a
dedicated namespace (e.g., `fleet-equalizer-system`).

**Alternatives considered**:
- A single `targetBudgetPercent` applied to both CPU and RAM — rejected: C2
  resolved per-resource independent equalization; a single target would force CPU
  and RAM to the same cumulative budget, which defeats the purpose of spec-012.
- Embed the kubeconfig directly in the CRD spec — rejected: kubeconfigs are
  secrets; they belong in Secret objects, not in CRD specs that operators can
  read via `kubectl get`.

---

## R4 — EqualizerConfig status schema

**Decision**:

```rust
pub struct EqualizerConfigStatus {
    /// Per-cluster observations from the last reconcile cycle.
    pub clusters: Vec<ClusterObservation>,

    /// Overall fleet condition (Healthy / Compensating / Degraded).
    pub condition: FleetCondition,

    /// Timestamp of the last successful reconcile cycle (RFC 3339).
    pub last_reconciled: String,
}

pub struct ClusterObservation {
    /// Cluster name (matches spec.targets[].name).
    pub name: String,

    /// Observed CPU utilization percentage (from Allocation.status).
    pub cpu_utilization_percent: f64,

    /// Observed memory utilization percentage.
    pub memory_utilization_percent: f64,

    /// Observed total allocatable CPU (milli, from ClusterCapacity.status).
    pub total_allocatable_cpu_milli: i64,

    /// Observed total allocatable memory (bytes).
    pub total_allocatable_memory_bytes: i64,

    /// Computed CPU budget the equalizer applied (or would apply if reachable).
    pub computed_cpu_budget_percent: i32,

    /// Computed memory budget.
    pub computed_memory_budget_percent: i32,

    /// Cluster state in the equalization.
    pub state: ClusterState,

    /// Last error message (if state is Unreachable or ConfigError).
    pub last_error: Option<String>,

    /// Timestamp of the last successful observation of this cluster.
    pub last_observed: String,
}

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

#[serde(rename_all = "kebab-case")]
pub enum FleetCondition {
    /// All clusters at or below target; no compensation active.
    Healthy,
    /// At least one cluster over target; others compensating.
    Compensating,
    /// One or more clusters unreachable or in config error.
    Degraded,
}
```

**Rationale**: FR-010/011 require rich per-cluster + fleet status. The status is
the operator's primary observability surface (US3 AC4). `ClusterObservation` is a
list (not a map) to match `targets[]` ordering. `ClusterState` and
`FleetCondition` are enums serialised kebab-case for clean `kubectl` output.

---

## R5 — The equalization algorithm: pure function signature and precision

**Decision**: the algorithm is a pure function that takes the observed fleet
state + targets and returns the computed per-cluster per-resource budgets. It is
the most heavily unit-tested component in this feature.

```rust
// src/equalizer/algorithm.rs

/// Input: one cluster's observed state for one resource dimension.
pub struct ClusterResourceObservation {
    pub name: String,
    pub utilization_percent: f64,
    pub total_allocatable: i64,  // CPU milli or RAM bytes
}

/// Output: the computed budget for one cluster + resource.
pub struct ComputedBudget {
    pub name: String,
    pub budget_percent: i32,
    pub state: BudgetState,  // Good / Over
}

/// The pure equalization algorithm (per resource dimension).
///
/// 1. Identify over-clusters: utilization_percent > target.
/// 2. Compute total absolute overflow = sum over over-clusters of
///    (utilization - target) * total_allocatable / 100.
/// 3. For each over-cluster: budget = floor(utilization) (frozen).
/// 4. For each good-cluster: budget = target - floor(total_overflow /
///    good_count / good_cluster_capacity * 100).
///    - Simplified: the per-good-cluster reduction in PERCENTAGE POINTS is
///      floor(total_overflow_abs / good_count / good_cluster_allocatable * 100).
///    - Each good cluster may have different capacity, so the reduction is
///      computed per-good-cluster using THAT cluster's capacity.
///    - Edge: if good_count == 0, all are frozen (no compensation).
/// 5. Clamp all budgets to [0, 100].
pub fn equalize(
    observations: &[ClusterResourceObservation],
    target_budget_percent: i32,
) -> Vec<ComputedBudget>
```

**Precision note**: the user's worked example (3 clusters × 100 CPU, target 80%,
util 65/55/90) has uniform capacity, so the math simplifies to
`good_budget = target - overflow_total_pct / good_count`. But the general case
has non-uniform cluster capacities. The algorithm compensates in ABSOLUTE units
(CPU milli / RAM bytes), then converts back to per-good-cluster percentage using
each good cluster's own capacity:

```
overflow_abs_total = Σ (over_cluster: (util% − target) × allocatable / 100)
overflow_per_good_cluster_abs = overflow_abs_total / good_count
good_cluster_budget% = target − floor(overflow_per_good_cluster_abs / good_cluster_allocatable × 100)
```

This ensures the fleet-wide ABSOLUTE overflow is compensated, regardless of
individual cluster sizes. A small cluster compensates with a larger percentage
reduction (it has less capacity to absorb the same absolute amount); a large
cluster compensates with a smaller percentage reduction. The fleet average in
absolute terms converges to the target.

**Rationale**: this is the user's algorithm (C1 clarification) generalized to
non-uniform capacities. The `floor` on percentage points is conservative (slightly
more restrictive on good clusters), matching the spec's rounding edge case.

**Alternatives considered**:
- Distribute overflow by capacity-weight (larger clusters absorb more) — rejected:
  the user said "divided by the number of good-state clusters" (equal split), not
  capacity-weighted. Equal split in absolute units is the user's stated model.
- Use floating-point budgets — rejected: the Allocation CRD fields are `i32`
  (integer percentages). Floor to integer.

---

## R6 — Worked example verification (algorithm correctness)

Target 80%, 3 clusters × 100 CPU (100_000 milli), util 65%/55%/90%:

1. Over-clusters: cluster C (90% > 80%). Over-clusters count = 1.
2. Overflow_abs = (90 − 80) × 100_000 / 100 = 10_000 milli CPU.
3. Good-clusters: A (65%), B (55%). Good count = 2.
4. Overflow per good = 10_000 / 2 = 5_000 milli CPU each.
5. Good A budget% = 80 − floor(5_000 / 100_000 × 100) = 80 − 5 = 75.
6. Good B budget% = 80 − floor(5_000 / 100_000 × 100) = 80 − 5 = 75.
7. Over C budget% = floor(90) = 90.
8. Fleet avg = (90 + 75 + 75) / 3 = 80. ✅

Over-cluster drops to 86%: overflow = (86−80)×100_000/100 = 6_000. Per good =
3_000. Good budget = 80 − floor(3000/100000×100) = 80 − 3 = 77... 

Wait — the user's example says good clusters get 78% (80 − 4/2 = 78). Let me
recheck: overflow at 86% = (86−80) = 6 percentage points × 100_000/100 = 6_000
milli. Per good = 6_000/2 = 3_000 milli. Good budget = 80 − floor(3_000/100_000 ×
100) = 80 − 3 = 77.

But the user said `80 − 4/2 = 78`. The user's mental model divides the PERCENTAGE
overflow (4 percentage points) by good count (2) = 2 percentage points per good
cluster → 80 − 2 = 78. That is the uniform-capacity simplification. My absolute-
units formula gives 77 (3 percentage points), which differs from the user's 78.

**Resolution**: the user's example uses uniform capacities where absolute and
percentage are equivalent (100 CPU total). In the uniform case:
- overflow_pct = (util% − target) = (86 − 80) = 6 percentage points. But the
  user wrote "4/2" — they computed 86−80 = 6, then... no, the user wrote
  "compensate the 6% of amount" but the example says "80 − 4/2 = 78". Let me
  re-read: "to 86% for example ... compensate the 6% of amount". 6/2 = 3. 80−3 =
  77. The user wrote "4/2" in the first example (90% → 10 overflow, 10/2 = 5,
  80−5 = 75) and "6% of amount" for the second (86% → 6 overflow, 6/2 = 3, 80−3
  = 77). The "4/2 = 78" in the spec is a typo from my specify phase (I wrote
  80−4/2=78 but should have written 80−6/2=77). 

The algorithm is correct: overflow_pct = (over_util − target); per-good reduction
= overflow_pct / good_count; good_budget = target − floor(reduction). This works
for uniform capacity (where 1% of capacity = total/100, same for all clusters).

For **non-uniform capacity**, the absolute-units conversion is needed. The
algorithm handles both via the absolute-units formula. The spec's AC2 value
should be 77 (not 78) — I'll note this as a specify-phase typo to correct in
the tasks phase, but the algorithm itself is correct.

---

## R7 — Hybrid poll + watch architecture

**Decision**: the reconcile loop runs on a fixed interval (default 10s,
configurable via a flag). Within each cycle:

1. **Poll phase**: for each target cluster, read the kubeconfig Secret, construct
   (or reuse) a `kube::Client`, GET the `Allocation` + `ClusterCapacity` status.
   This is the discovery + state-read step.
2. **Compute phase**: run the pure `equalize()` algorithm on the observed state.
3. **Patch phase**: for each reachable cluster, patch `Allocation.spec` with the
   computed per-resource budgets.

**Watch layer** (optional optimization, layered on top of polling): for each
cluster confirmed reachable in the poll phase, open a `kube::runtime::watcher`
on the `Allocation` CRD. When a watch event fires (utilization changed), trigger
an immediate reconcile cycle for the fleet (not just that cluster — the
algorithm is fleet-wide). Watch streams run in background tokio tasks; on
stream error, the watcher is dropped and that cluster falls back to
polling-only until the next cycle re-establishes the watch.

**Rationale**: the user chose hybrid (C4). Polling is the reliable baseline
(every 10s, guaranteed); watches provide sub-second reactivity for clusters
that support them. The watch layer is an optimization, not a correctness
requirement — if all watches fail, the polling loop still equalizes correctly
(at 10s latency). This is the safest multi-cluster observation strategy.

**v1 scoping**: start with polling-only (simpler, proven kube-rs pattern). Add
the watch layer as a second iteration within the same spec if time permits, or
defer to a follow-up spec. The spec's FR-008 describes the full hybrid model;
the plan recommends polling-first, watch-as-enhancement.

---

## R8 — Dockerfile.equalizer: separate image

**Decision**: a second Dockerfile (`Dockerfile.equalizer`) following the same
multi-stage build pattern as the existing `Dockerfile`, but targeting the
`capacity-equalizer` binary:

```dockerfile
FROM rust:1.89-bookworm AS builder
WORKDIR /usr/src/capacity-admission-webhook
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin/equalizer && echo "fn main() {}" > src/bin/equalizer/main.rs \
    && echo "" > src/lib.rs && cargo build --release && rm -rf src
COPY . .
RUN touch src/lib.rs && cargo build --release --bin capacity-equalizer
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /usr/src/capacity-admission-webhook/target/release/capacity-equalizer /usr/local/bin/capacity-equalizer
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/capacity-equalizer"]
```

**Rationale**: separate image = independent deployment lifecycle (the equalizer
can be upgraded without redeploying the webhook). The dummy-deps caching layer
must create stubs for ALL `[[bin]]` paths (the CI failure catalog Layer 9
lesson). The runtime image is distroless (same as the webhook). No HTTPS port
needed (the equalizer has no admission endpoint) — only a metrics port (9090)
if metrics are exposed.

**Alternatives considered**:
- Single image with both binaries, entrypoint selects — rejected: couples
  deployment lifecycles; a webhook redeploy would restart the equalizer and
  vice versa.
- Multi-arch build via the existing publish workflow — viable follow-up; v1
  builds the equalizer image alongside the webhook image in CI.

---

## R9 — RBAC for the equalizer

**Decision**: the equalizer's ServiceAccount needs:

In the **home cluster** (where the equalizer runs):
- `get`/`list`/`watch` on `Secrets` (read kubeconfig Secrets for target clusters).
- Full CRUD on `equalizerconfigs.emergency-ration.dev` (read spec, write status).

In **each target cluster** (via the kubeconfig's identity):
- `get` on `allocations.emergency-ration.dev` (read status: utilization).
- `get` on `clustercapacities.emergency-ration.dev` (read status: allocatable).
- `patch` on `allocations.emergency-ration.dev` (write spec: cpuBudgetPercent /
  memoryBudgetPercent).

**Rationale**: least-privilege. The equalizer reads Secrets only in its home
cluster; in target clusters it only reads CRD status + patches Allocation spec.
The target-cluster RBAC is the operator's responsibility (they create the
ServiceAccount + Role in each target cluster and embed the kubeconfig in the
Secret). The deploy manifest includes an example target-cluster RBAC for
reference.

---

## R10 — Testing strategy

**Decision**:

1. **Algorithm unit tests** (`tests/equalizer/algorithm.rs`): the pure
   `equalize()` function tested with truth-table cases — the worked examples
   from the spec (US1 AC1, US2 AC1-AC5) plus edge cases (all-over, single
   cluster, zero capacity, non-uniform capacities). This is the most critical
   test surface.

2. **Reconcile integration tests** (`tests/equalizer/reconcile.rs`): mock N
   target-cluster apiservers via `tower-test` (one mock Service per cluster),
   feed scripted Allocation/ClusterCapacity responses, and assert the correct
   budget patches are issued. Tests the read → compute → patch loop end-to-end
   without real clusters.

3. **BDD** (`tests/bdd/features/equalizer.feature`): Gherkin scenarios for the
   equalization user stories (all-under, over-compensation, unreachable).

4. **E2E** (CI): two `kind` clusters, the webhook installed in both, the
   equalizer in one, EqualizerConfig targeting both. Verify budgets propagate.
   This is the heaviest test — may be `#[ignore]` by default with a CI-specific
   runner.

5. **erw-verify** (FR-015): a new scenario module that orchestrates a
   multi-cluster fixture and validates the equalizer against it.

**Rationale**: the algorithm's purity makes it the highest-leverage test target.
The reconcile integration tests validate the multi-cluster wiring (the new
complexity). E2E proves it works against real apiservers. The testing approach
mirrors the existing webhook's strategy (unit → tower-test mock → BDD → kind
E2E), adapted for the multi-cluster dimension.

---

## R11 — Library crate sharing: what the equalizer reuses

**Decision**: the equalizer binary depends on the `capacity_admission_webhook`
library crate for:
- `Allocation`, `AllocationSpec`, `AllocationStatus` (read target status, write
  target spec).
- `ClusterCapacity`, `ClusterCapacityStatus` (read target capacity).
- `CLUSTER_ALLOCATION_NAME`, `CLUSTER_CAPACITY_NAME` constants (singleton names).

The equalizer does NOT reuse: the webhook handler, admission logic, ceiling
computation, metrics, or controllers (those are per-cluster concerns, not fleet-
level). The EqualizerConfig CRD is defined in the equalizer's own module
(`src/equalizer/crd.rs`), not in the shared `src/crd/` directory (it's an
equalizer-specific type, not a per-cluster type).

**Rationale**: clean separation. The library exports the shared types; the
equalizer imports them. This mirrors how `erw-verify` reuses the library for CRD
types without pulling in the webhook handler.

---

## Summary

11 research items, all resolved to concrete decisions. No external research
needed (no new crate, no unfamiliar API) — the feature reuses kube-rs patterns
already proven in the codebase (`erw-verify/client.rs` for multi-cluster client
construction, `kube::CustomResource` derive for the new CRD, `tower-test` for
mocked multi-cluster integration tests). The algorithm is the novel element;
R5/R6 lock its design and verify the math. Phase 1 design artifacts encode these
decisions.
