# Feature Specification: Cumulative Multi-Cluster Capacity Equalizer

**Feature Branch**: `013-multi-cluster-capacity-equalizer`

**Created**: 2026-08-06

**Status**: Draft

**Input**: User description: a controller deployed in one cluster that watches a
fleet of N Kubernetes clusters, reads each cluster's current capacity allocation,
and dynamically adjusts each cluster's budget limit to bring the fleet's
cumulative allocation to a configured target — compensating for over-limit
clusters by lowering the budgets of healthy clusters in proportion.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Equalization: All Clusters Within Target (Priority: P1)

As a fleet operator, I want the equalizer to set every cluster's budget to the
cumulative target when all clusters are below it, so that each cluster gets
maximum headroom while the fleet stays at the configured cumulative budget.

**Why this priority**: this is the baseline case and the smallest independently
testable slice. It exercises the core read → compute → patch loop against
multiple clusters, the EqualizerConfig CRD, and the per-resource budget patches
landing on each target's Allocation singleton. Without this, the equalizer does
nothing.

**Independent Test**: configure an EqualizerConfig with `cpuTargetBudgetPercent:
80`, `memoryTargetBudgetPercent: 80`, and 3 target clusters each at 65/55/45%
CPU utilization (all below target). The equalizer patches each cluster's
`Allocation.spec` with `cpuBudgetPercent: 80, memoryBudgetPercent: 80` and sets
its own status to healthy.

**Acceptance Scenarios**:

1. **Given** an EqualizerConfig with `cpuTargetBudgetPercent: 80` and 3 target
   clusters all reporting CPU utilization below 80% (65%, 55%, 45%), **When** the
   equalizer reconciles, **Then** each target cluster's `Allocation.spec.cpuBudgetPercent`
   is patched to 80 (its ceiling rises to the target).
2. **Given** the same configuration, **When** the equalizer reconciles, **Then**
   its own `EqualizerConfig.status` reports each cluster's observed utilization,
   the computed budget, and an overall `Healthy` condition.
3. **Given** a cluster whose Allocation singleton does not yet have
   `cpuBudgetPercent` set (it only has the legacy `budgetPercent`), **When** the
   equalizer patches it, **Then** the override is applied and the cluster's
   webhook enforces the new per-resource ceiling (per spec-012's resolution
   logic).

---

### User Story 2 - Over-Limit Compensation (Priority: P2)

As a fleet operator, when one cluster exceeds the cumulative target, I want the
equalizer to freeze that cluster at its current utilization (stopping further
growth) and lower the budgets of the other clusters to compensate — distributing
the absolute overflow equally among the good-state clusters — so that the fleet
average stays at the target. When the over-limit cluster's utilization drops, I
want all budgets recalculated immediately.

**Why this priority**: this is the core equalization algorithm and the reason
the feature exists. P2 (not P1) because it depends on US1's read→compute→patch
loop being functional first; it adds the overflow-distribution math and the
dynamic recalculation on top.

**Independent Test**: configure `cpuTargetBudgetPercent: 80` with 3 clusters
(each 100 CPU total) at utilizations 65%, 55%, 90%. The 90%-cluster is frozen at
90% (overflow = 10 CPU). The two good clusters each get `80 − 10/2 = 75%`. Fleet
average = (90+75+75)/3 = exactly 80%. When the over-cluster drops to 86%, its
budget is lowered to 86% and the good clusters rise to `80 − 4/2 = 78%`.

**Acceptance Scenarios**:

1. **Given** `cpuTargetBudgetPercent: 80` and 3 clusters (100 CPU each) at
   utilization 65%, 55%, 90%, **When** the equalizer reconciles, **Then** the
   over-cluster's `cpuBudgetPercent` is frozen at 90, and each good cluster
   receives `80 − 10/2 = 75` (overflow 10 CPU / 2 good clusters).
2. **Given** the same state after reconciliation, **When** the over-cluster's
   utilization drops from 90% to 86% (overflow reduced from 10 to 6 CPU), **Then**
   the over-cluster's budget is lowered to 86, and the good clusters' budgets rise
   to `80 − 6/2 = 77`. (Overflow 6% × 100 CPU / 100 = 6_000m; per good cluster
   3_000m; reduction = 3_000m × 100 / 100_000m = 3%; budget = 80 − 3 = 77.)
3. **Given** `cpuTargetBudgetPercent: 80` and 3 clusters where ALL three are over
   80% (85%, 85%, 85%), **When** the equalizer reconciles, **Then** each cluster
   is frozen at 85 (no good clusters to compensate; the fleet is uniformly over —
   the equalizer cannot help but prevents each from growing further).
4. **Given** the over-cluster from AC1 at 90% and one of the good clusters also
   goes over its computed 75% budget (reaches 76%), **When** the equalizer
   reconciles, **Then** the 76%-cluster is now also an over-cluster (frozen at
   76%), and the remaining good cluster's budget is recomputed to compensate both
   over-clusters' combined overflow.
5. **Given** CPU and RAM utilization that disagree (CPU all under target, RAM has
   one cluster over), **When** the equalizer reconciles, **Then** CPU and RAM
   budgets are computed independently — CPU gets target for all, RAM gets the
   overflow-compensated distribution (per spec-012's per-resource model).

---

### User Story 3 - Target Reachability and Status Reporting (Priority: P3)

As a fleet operator, when a target cluster's API server is unreachable or its
kubeconfig Secret is missing/malformed, I want the equalizer to skip that
cluster's budget patch (leaving its last-known budget untouched), report the
failure in its status, and continue managing the remaining reachable clusters —
so that one unreachable cluster does not block the fleet's equalization.

**Why this priority**: this is the operational resilience layer. Without it, a
single target outage freezes the entire equalizer. P3 because US1/US2 are the
core value; US3 makes the system production-grade.

**Independent Test**: configure 3 target clusters where one cluster's API server
is unreachable. The equalizer's status reports that cluster as `Unreachable`
with the last error, patches the 2 reachable clusters normally, and continues
reconciling without crashing. When the unreachable cluster comes back, it is
re-incorporated into the equalization.

**Acceptance Scenarios**:

1. **Given** 3 target clusters where cluster C's API server is unreachable,
   **When** the equalizer reconciles, **Then** clusters A and B receive their
   computed budgets normally, cluster C's budget is left untouched (last-known
   value preserved), and the status reports C as `Unreachable` with an error
   message and timestamp.
2. **Given** a target cluster whose kubeconfig Secret is missing or malformed,
   **When** the equalizer reconciles, **Then** that cluster is reported as
   `ConfigError` in the status, the remaining clusters are managed normally, and
   the equalizer does not crash or enter a restart loop.
3. **Given** cluster C was unreachable but its API server comes back online,
   **When** the next reconcile cycle runs, **Then** C is re-incorporated: its
   utilization is read, its budget is recomputed, and its status transitions to
   `Healthy`.
4. **Given** any reconcile cycle, **When** the equalizer computes budgets, **Then**
   the status records per-cluster: observed CPU/RAM utilization, observed total
   allocatable CPU/RAM, computed budget (CPU/RAM), cluster state (Healthy /
   Over / Unreachable / ConfigError), and the last-updated timestamp — sufficient
   for an operator to understand the fleet state from `kubectl get equalizerconfig -o yaml`.

---

### Edge Cases

- **All clusters over target** → each frozen at its current utilization; no
  compensation possible (US2 AC3). Status reports the fleet as `AllOverTarget`.
- **Single-cluster fleet (N=1)** → the equalizer patches the one cluster's budget
  to the target if it's under, or freezes it at current utilization if over.
  Degenerate but valid.
- **Cluster with zero allocatable capacity** (e.g., all nodes drained) →
  utilization is undefined (0/0). The equalizer treats this cluster as having 0
  capacity; its absolute overflow is 0, so it does not contribute to the overflow
  pool. Its budget is set to the target (it has nothing to protect).
- **Cluster joins mid-operation** (operator adds a target to the EqualizerConfig)
  → the next reconcile cycle discovers it, reads its state, and incorporates it
  into the equalization from that point.
- **Cluster removed mid-operation** → its budget is left at its last-known value
  (the equalizer does NOT reset it); the remaining clusters' budgets are
  recomputed for the reduced fleet.
- **Kubeconfig Secret updated at runtime** → the equalizer picks up the new
  kubeconfig on the next reconcile (Secrets are read fresh each cycle, not cached
  indefinitely).
- **Multiple over-clusters** → total overflow = sum of each over-cluster's
  absolute overflow; distributed equally among ALL good-state clusters (US2 AC4).
- **Over-cluster's utilization drops below target** → it transitions from
  Over-state to Good-state; its budget is set to the target, and the remaining
  compensation is recomputed.
- **Rounding**: absolute overflow divided by good-cluster count may produce a
  fractional percentage. The per-cluster budget is floored to an integer (0–100),
  consistent with the Allocation CRD's integer `budgetPercent` fields. Flooring
  is conservative (slightly more restrictive on the good clusters).
- **Equalizer pod restart** → the equalizer is stateless (all state lives in the
  target clusters' Allocation CRDs + the EqualizerConfig status). A restart
  re-reads everything from scratch; no local state to lose.
- **EqualizerConfig updated at runtime** (target budget changed, cluster added)
  → picked up on the next reconcile cycle; no restart needed.
- **Per-cluster webhook in the over-cluster still enforces** — the over-cluster's
  webhook enforces its frozen budget (e.g., 90%); it does NOT go dry-run. The
  equalizer only adjusts the BUDGET number; enforcement stays with the local
  webhook (per-cluster sovereignty).
- **Home cluster identity** — the cluster the equalizer runs in is managed through
  a kubeconfig Secret just like every other cluster. No in-cluster ServiceAccount
  special-casing. The equalizer is location-independent.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST introduce a new controller binary (`capacity-equalizer`)
  separate from the existing `capacity-admission-webhook` binary, packaged as its
  own Docker image (the existing Dockerfile builds the webhook image; a second
  Dockerfile or multi-target build produces the equalizer image).

- **FR-002**: The system MUST introduce a new cluster-scoped CRD
  `EqualizerConfig` (`emergency-ration.dev/v1`, singleton instance
  `fleet-equalizer`) whose spec carries the cumulative target budget (per-resource:
  `cpuTargetBudgetPercent` and `memoryTargetBudgetPercent`, each 0–100) and a list
  of target cluster definitions.

- **FR-003**: Each target cluster definition in `EqualizerConfig.spec.targets[]`
  MUST carry: a human-readable `name`, a reference to a Kubernetes Secret
  containing a kubeconfig (`kubeconfigSecretRef` with `name` and `key`), and the
  namespace of that Secret. Every cluster — including the one the equalizer runs
  in — is specified this way (no in-cluster SA shortcut).

- **FR-004**: The equalizer MUST read each target cluster's
  `Allocation.status.utilizationPercentCpu/Memory` (current demand) AND
  `ClusterCapacity.status.totalAllocatableCpuMilli/MemoryBytes` (cluster capacity)
  to compute the absolute overflow in real units (CPU milli / RAM bytes), per the
  resolved equalization algorithm.

- **FR-005**: The equalization algorithm MUST, per resource independently:
  (a) identify clusters whose utilization exceeds the target budget as
  "over-clusters" and freeze their budget at their current utilization
  percentage; (b) compute the total absolute overflow as the sum of each
  over-cluster's `(utilization − target) × totalAllocatable / 100`; (c) distribute
  the total overflow equally among the "good-state" clusters (those at or below
  target) by lowering each good cluster's budget by `floor(totalOverflow /
  goodClusterCount / goodClusterCapacity × 100)` percentage points below the
  target; (d) when there are no good clusters, freeze all at current utilization.

- **FR-006**: The equalizer MUST write the computed per-resource budgets to each
  target cluster's `Allocation.spec.cpuBudgetPercent` and
  `Allocation.spec.memoryBudgetPercent` (the override fields introduced in
  spec-012) via a strategic-merge patch.

- **FR-007**: The equalizer MUST NOT modify the target cluster's legacy
  `budgetPercent` field — it writes only the per-resource overrides (spec-012
  fields), leaving `budgetPercent` as the operator's fallback.

- **FR-008**: The equalizer MUST discover target clusters via a polling interval
  (configurable, default 10 seconds) and, for each cluster confirmed reachable,
  open a live WATCH stream on that cluster's `Allocation` and `ClusterCapacity`
  CRDs for sub-second reactivity. When a watch stream fails, the equalizer falls
  back to polling that cluster until the stream is re-established (hybrid
  discovery + watch model).

- **FR-009**: When a target cluster's API server is unreachable or its kubeconfig
  Secret is missing/malformed, the equalizer MUST skip that cluster's budget
  patch (preserving its last-known budget), report the failure in its status, and
  continue managing the remaining clusters.

- **FR-010**: The `EqualizerConfig.status` MUST report, per target cluster: the
  observed CPU and RAM utilization percentages, observed total allocatable CPU
  and RAM, the computed budget percentages (CPU and RAM), the cluster state
  (`Healthy`, `Over`, `Unreachable`, `ConfigError`), the last error message (if
  any), and the timestamp of the last successful observation.

- **FR-011**: The `EqualizerConfig.status` MUST report an overall fleet condition
  (`Healthy` when all clusters are at or below their computed budgets,
  `Compensating` when at least one cluster is over and others are compensating,
  `Degraded` when one or more clusters are unreachable).

- **FR-012**: The equalizer MUST emit structured logs (`tracing`) for every
  reconcile cycle: the per-cluster observed/utilization/computed-budget figures,
  the total overflow, the compensation distribution, and any errors.

- **FR-013**: The equalizer MUST be stateless — all persistent state lives in the
  target clusters' Allocation CRDs (the budgets it writes) and the
  EqualizerConfig status (the observations it records). A pod restart re-reads
  everything; no local cache or database.

- **FR-014**: The equalizer MUST reconcile CPU and RAM independently — each
  resource has its own target, its own overflow pool, its own set of over/good
  clusters, and its own computed budgets. A cluster can be an over-cluster for
  CPU but a good-cluster for RAM simultaneously.

- **FR-015**: The on-demand verification tool (`erw-verify`) MUST gain a scenario
  (or a separate verification mode) that validates the equalizer against a
  multi-cluster test fixture (e.g., two `kind` clusters) — patching the
  EqualizerConfig, observing the budget patches land on each target's Allocation,
  and asserting the equalization math.

### Key Entities *(include if feature involves data)*

- **EqualizerConfig CRD** (NEW): cluster-scoped, singleton `fleet-equalizer`,
  `emergency-ration.dev/v1`. Spec carries per-resource cumulative targets + target
  cluster list (name → kubeconfig Secret ref). Status carries per-cluster
  observations, computed budgets, cluster states, and the overall fleet condition.

- **Allocation CRD** (existing, consumed downstream): the equalizer reads
  `status` (utilization) and writes `spec.cpuBudgetPercent` /
  `spec.memoryBudgetPercent` on each target cluster's singleton. No change to the
  CRD itself (spec-012 added the override fields; spec-013 uses them).

- **ClusterCapacity CRD** (existing, consumed read-only): the equalizer reads
  `status.totalAllocatableCpuMilli/MemoryBytes` from each target to convert
  utilization percentages to absolute units for overflow computation.

- **Kubeconfig Secrets**: standard Kubernetes Secrets (type `Opaque`) containing
  a kubeconfig file under a named key. Referenced by `EqualizerConfig.spec.targets[].kubeconfigSecretRef`.
  The equalizer reads these to construct a `kube::Client` per target cluster.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A fleet of 3 clusters with a target of 80% CPU, where one cluster
  is at 90% utilization, has its budgets equalized to 90% / 75% / 75% within one
  reconcile cycle — and the fleet average converges to exactly 80%.

- **SC-002**: When the over-cluster's utilization drops (90% → 86%), the budgets
  are recalculated to 86% / 78% / 78% within one reconcile cycle (sub-second
  reaction via live watch, or ≤10s via polling fallback).

- **SC-003**: An unreachable target cluster does not block the equalization of
  the remaining clusters, and its failure is visible in `kubectl get
  equalizerconfig -o yaml` with a clear error and timestamp.

- **SC-004**: CPU and RAM are equalized independently — a cluster over on CPU
  but under on RAM receives a frozen CPU budget and a target RAM budget in the
  same reconcile cycle.

- **SC-005**: The equalizer is delivered as a separate binary + Docker image,
  deployable independently from the webhook, with its own RBAC (read Secrets,
  read/patch Allocation + ClusterCapacity, manage EqualizerConfig).

## Assumptions

- Each target cluster has the emergency-ration-webhook installed and running
  (Node Capacity Controller + Allocation Controller + Admission Webhook), so the
  Allocation and ClusterCapacity CRDs exist with populated status. The equalizer
  depends on these CRDs being present and healthy; it does not install or manage
  them.
- Each target cluster's Allocation singleton is named `cluster-allocation` and
  the ClusterCapacity singleton is `cluster-capacity` (the existing convention).
- The kubeconfig in each Secret grants sufficient RBAC to read Allocation +
  ClusterCapacity CRDs and patch Allocation.spec (cluster-admin or a scoped role
  the operator creates). The equalizer does not create RBAC in target clusters.
- The equalization algorithm uses `floor` for integer percentage rounding
  (conservative — slightly more restrictive on good clusters). This matches the
  Allocation CRD's integer fields.
- The feature does NOT amend the constitution: it adds a new component (the
  equalizer) that operates alongside the existing 3-component per-cluster
  architecture (Principle V). The equalizer does not add a new failure mode to
  the admission path (Principle I) — if the equalizer is down, each cluster's
  webhook continues enforcing its last-known budget independently. The equalizer
  is a fleet-level optimizer, not a per-cluster safety mechanism.
- The separate binary/image decision honors Principle V (separated concerns): the
  equalizer is a fleet-control-plane component, not an admission-critical-path
  component. Coupling them in one binary would conflate different risk profiles
  and deployment lifecycles.
- The `capacity-equalizer` binary reuses the library crate
  (`capacity_admission_webhook`) for CRD types (`Allocation`, `ClusterCapacity`,
  status structs) — the types are shared, the binaries are separate.
