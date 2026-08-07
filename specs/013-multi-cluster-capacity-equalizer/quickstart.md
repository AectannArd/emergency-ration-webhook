# Quickstart — Multi-Cluster Capacity Equalizer (spec-013)

**Date**: 2026-08-06

A validation guide mapping each spec user story to runnable test scenarios. This
is NOT an implementation tutorial — it lists the commands and assertions that
prove the feature works.

---

## Prerequisites

- Dev environment for the `capacity-admission-webhook` crate (Rust 1.89).
- For mocked integration/BDD tests: no cluster required — `tower-test` mocks the
  apiserver (one mock per target cluster).
- For E2E (kind multi-cluster): two `kind` clusters with the webhook installed
  in both. See `CONTRIBUTING.md` for setup.
- For `erw-verify` equalizer scenario (FR-015): same multi-cluster fixture.

---

## US1 — Equalization: All Clusters Within Target (P1)

**Validates**: FR-002, FR-003, FR-004, FR-006, FR-010, FR-011. The baseline
read → compute → patch loop.

### V1.1 — Unit test: the pure equalization algorithm (all-under case)

```bash
cargo test --test algorithm equalize_all_under_target
```

**Asserts**: target 80%, 3 clusters × 100_000m util 65/55/45 → budgets 80/80/80,
all state=Good. (data-model.md §2.3 Example 1.)

### V1.2 — Unit test: CRD serialisation round-trip

```bash
cargo test --test algorithm equalizer_config_crd_serialises
```

**Asserts**: `EqualizerConfig::crd()` has the expected name
(`equalizerconfigs.emergency-ration.dev`), scope `Cluster`, short name `eqconf`,
status subresource, and the spec fields with range constraints.

### V1.3 — Integration test: multi-cluster read → compute → patch (mocked)

```bash
cargo test --test reconcile equalize_all_under_target_mocked
```

**Scenario**: 3 mocked target apiservers, each returning an Allocation at 65/55/45%
utilization + a ClusterCapacity at 100_000m. The equalizer's reconcile loop
reads all three, computes 80/80/80, and issues 3 PATCH calls — assert each mock
received the correct `cpuBudgetPercent: 80, memoryBudgetPercent: 80` patch.

### V1.4 — BDD: all clusters within target

```bash
cargo test --test equalizer_bdd
```

**Feature** (`tests/bdd/features/equalizer.feature`):
```gherkin
Scenario: All clusters within target — budgets set to target
  Given 3 target clusters with CPU utilization 65%, 55%, 45%
  And the EqualizerConfig has cpuTargetBudgetPercent 80
  When the equalizer reconciles
  Then each cluster receives cpuBudgetPercent 80
  And the fleet condition is Healthy
```

---

## US2 — Over-Limit Compensation (P2)

**Validates**: FR-005, FR-014. The core equalization algorithm.

### V2.1 — Unit test: one over-cluster compensation

```bash
cargo test --test algorithm equalize_one_over
```

**Asserts**: target 80%, util 65/55/90, all × 100_000m → budgets 75/75/90.
(Example 2.) Over-cluster C frozen at 90, good clusters A/B at 75.

### V2.2 — Unit test: over-cluster drops, recalculation

```bash
cargo test --test algorithm equalize_over_drops
```

**Asserts**: C drops from 90% to 86% → budgets 77/77/86. (Example 3. **Note:
the spec's AC2 says "78" — that is a specify-phase arithmetic typo; the correct
value is 77. The algorithm and tests use 77.**)

### V2.3 — Unit test: all over (no compensation possible)

```bash
cargo test --test algorithm equalize_all_over
```

**Asserts**: util 85/85/85, target 80 → all frozen at 85. (Example 4.)

### V2.4 — Unit test: non-uniform cluster capacities

```bash
cargo test --test algorithm equalize_non_uniform_capacity
```

**Asserts**: A=100_000m@60%, B=200_000m@60%, C=200_000m@95%, target 80 →
budgets 65/73/95. (Example 5 — verifies the absolute-units distribution with
different cluster sizes.)

### V2.5 — Unit test: CPU and RAM independent

```bash
cargo test --test algorithm equalize_cpu_ram_independent
```

**Asserts**: CPU all-under (all get target), RAM one-over (RAM gets compensated).
Two separate `equalize()` calls produce independent budgets. The reconcile loop
calls `equalize()` twice (once per resource) and patches both override fields.

### V2.6 — Integration test: over-compensation patched to mocked targets

```bash
cargo test --test reconcile equalize_over_compensation_mocked
```

**Scenario**: 3 mocks, util 65/55/90. Assert C receives `cpuBudgetPercent: 90`
(frozen) and A/B receive `cpuBudgetPercent: 75` (compensated).

---

## US3 — Target Reachability and Status Reporting (P3)

**Validates**: FR-009, FR-010, FR-011.

### V3.1 — Integration test: unreachable cluster skipped

```bash
cargo test --test reconcile unreachable_cluster_skipped
```

**Scenario**: 3 mocks, but cluster C's mock returns an error. Assert A and B
receive their computed budgets, C's budget is NOT patched (no PATCH call to C's
mock), and the status reports C as `Unreachable` with an error message.

### V3.2 — Integration test: config error (missing Secret)

```bash
cargo test --test reconcile config_error_missing_secret
```

**Scenario**: the Secret for cluster C does not exist in the home-cluster mock.
Assert C is reported as `ConfigError`, A/B managed normally.

### V3.3 — Integration test: unreachable cluster recovers

```bash
cargo test --test reconcile unreachable_recovers
```

**Scenario**: C is unreachable on cycle 1, then reachable on cycle 2. Assert C's
status transitions `Unreachable → Healthy`, and its budget is patched on cycle 2.

### V3.4 — Unit test: fleet condition aggregation

```bash
cargo test --test algorithm fleet_condition
```

**Asserts**: all healthy → `Healthy`. One over → `Compensating`. One unreachable
→ `Degraded`. Multiple states → `Degraded` wins (highest severity).

### V3.5 — Unit test: status serialisation (kubectl-readable)

```bash
cargo test --test algorithm status_serialises_camel_case
```

**Asserts**: `EqualizerConfigStatus` serialises with `clusters`, `condition`,
`lastReconciled` in camelCase; `ClusterState` serialises kebab-case
(`healthy`/`over`/`unreachable`/`config-error`); `FleetCondition` serialises
kebab-case.

---

## Edge case coverage (mapped to tests)

| Edge case | Test |
|-----------|------|
| All over target | V2.3 |
| Single-cluster fleet | `cargo test --test algorithm single_cluster` |
| Zero-capacity cluster | `cargo test --test algorithm zero_capacity` |
| Non-uniform capacities | V2.4 |
| Multiple over-clusters | `cargo test --test algorithm multiple_over` |
| Over→good transition | `cargo test --test algorithm over_to_good` |
| Rounding (floor) | V2.1 (75 = 80−5, floored) |
| ConfigError (missing Secret) | V3.2 |
| Unreachable (API error) | V3.1 |
| Recovery | V3.3 |

---

## Full validation command

```bash
# All unit + integration + BDD:
cargo test

# Clippy + fmt gate:
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Equalizer-specific:
cargo test --test algorithm
cargo test --test reconcile
cargo test --test equalizer_bdd

# E2E (multi-cluster kind, opt-in):
# See CONTRIBUTING.md — requires two kind clusters with webhook installed.
```

**Expected**: all green. The equalizer adds new tests; it does not modify the
webhook's existing behavior, so the existing suite passes unchanged.
