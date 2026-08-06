# Quickstart — Per-Resource Budget Tracking (spec-012)

**Date**: 2026-08-06

A validation guide mapping each spec user story to runnable test scenarios. This
is NOT an implementation tutorial — it lists the commands and assertions that
prove the feature works, referencing the contract (`contracts/allocation-crd.md`)
and data model (`data-model.md`) for the precise field semantics.

---

## Prerequisites

- A working dev environment for the `capacity-admission-webhook` crate (Rust 1.89,
  edition 2024). See `CONTRIBUTING.md` for setup.
- For mocked integration/BDD tests: no cluster required — `tower-test` mocks the
  apiserver.
- For `erw-verify` S9 (real-cluster): a throwaway Kubernetes cluster reachable
  via `KUBECONFIG` (same prerequisite as the existing erw-verify scenarios; see
  `CONTRIBUTING.md` § "Running erw-verify").

---

## US1 — Asymmetric Budgets Per Resource (P1)

**Validates**: FR-001, FR-002, FR-003, FR-004, FR-011, FR-012. The core
capability — independent CPU/RAM ceilings.

### V1.1 — Unit test: resolution function (pure)

```bash
cargo test --lib crd::allocation::tests::resolve_effective_budgets
```

**Asserts** (from `data-model.md` §2 truth table):
- `(80, None, None) → (80, 80)` (legacy)
- `(80, Some(90), None) → (90, 80)` (CPU override, memory fallback)
- `(80, None, Some(60)) → (80, 60)` (memory override, CPU fallback)
- `(80, Some(90), Some(60)) → (90, 60)` (both overridden)
- `(80, Some(80), Some(80)) → (80, 80)` (override equals fallback)
- `(100, Some(0), Some(100)) → (0, 100)` (boundary)

### V1.2 — Unit test: per-resource ceiling helper

```bash
cargo test --lib webhook::admission::tests::ceiling_per_resource
```

**Asserts**:
- `ceiling_per_resource((100_000, 200*Gi), (90, 60))` ==
  `(90_000, floor(200Gi * 60/100))` — each figure uses its own budget.
- `ceiling_per_resource((t, t), (p, p))` == `ceiling((t, t), p)` for several
  `(t, p)` — backward-compat equivalence (research R3).

### V1.3 — Integration test: CPU admits, memory denies (asymmetric)

```bash
cargo test --test budget_enforcement per_resource_asymmetric
```

**Scenario**: Allocation singleton with `cpuBudgetPercent: 95`,
`memoryBudgetPercent: 30`, `budgetPercent: 80`. A pod with CPU request fitting
under the 95% CPU ceiling but memory request exceeding the 30% memory ceiling.

**Asserts**:
- Verdict = Deny.
- Violations contains exactly ONE entry, `resource: Memory` (CPU is NOT reported
  as violated — FR-011).
- Swapping overrides (`cpuBudgetPercent: 30`, `memoryBudgetPercent: 95`) and the
  symmetric pod (CPU-heavy) → Deny with `resource: Cpu` only (US1 AC2).

### V1.4 — BDD: per-resource asymmetric budgets

```bash
cargo test --test budget_bdd
```

**Feature** (`tests/bdd/budget.feature`, new scenario):
```gherkin
Scenario: Per-resource asymmetric budgets — CPU admits, memory denies
  Given the cluster has 100 CPU and 200Gi memory allocatable
  And the Allocation singleton has budgetPercent 80
  And the Allocation singleton has cpuBudgetPercent 95
  And the Allocation singleton has memoryBudgetPercent 30
  When a pod requesting 90 CPU and 150Gi is submitted
  Then the pod is denied
  And the denial reason names memory as the exceeded resource
```

### V1.5 — Real-cluster (erw-verify S9, FR-012)

```bash
erw-verify --kubeconfig <path>   # runs all scenarios including S9
```

**S9 detail** in `contracts/allocation-crd.md` §6. **Asserts** against a live
apiserver: patching `cpuBudgetPercent: 95, memoryBudgetPercent: 30`, waiting for
the controller to recompute, then creating a pod that is denied on memory
(with "memory" in the denial message). Restores overrides afterwards.

---

## US2 — Backward Compatibility (P2)

**Validates**: FR-005, FR-006, FR-008. The release gate — no behavioural drift
on existing single-budget deployments.

### V2.1 — Unit test: byte-identical ceilings without overrides (FR-005)

```bash
cargo test --lib controllers::allocation::tests::no_override_ceilings_match_legacy
```

**Asserts**: for `(budget_percent: 80, cpu: None, memory: None)` and supply
`(100_000, 200Gi)`, `build_allocation_status` produces `ceiling_cpu_milli` and
`ceiling_memory_bytes` equal to `floor(supply * 80 / 100)` — the exact values
the pre-spec-012 `ceiling()` would produce. Repeated for several `budget_percent`
values (0, 50, 80, 100) and supplies.

### V2.2 — Unit test: overrides serialise absent when None (US2 AC2)

```bash
cargo test --lib crd::allocation::tests::overrides_absent_when_none
```

**Asserts**: serialising an `AllocationSpec` with `cpu_budget_percent: None,
memory_budget_percent: None` to JSON yields an object WITHOUT `cpuBudgetPercent`
or `memoryBudgetPercent` keys (not `null`).

### V2.3 — Unit test: budgetPercent remains required (FR-006)

```bash
cargo test --lib crd::allocation::tests::budget_percent_still_required
```

**Asserts**: the generated CRD schema lists `budgetPercent` in `spec.required`,
and does NOT list `cpuBudgetPercent` / `memoryBudgetPercent`.

### V2.4 — Full existing suite unchanged (US2 AC3, SC-002)

```bash
cargo test
```

**Asserts**: the full existing test suite (budget enforcement, capacity
awareness, dry-run, fail-safe, exclusion, node-filter, performance, BDD) passes
unchanged when run against an Allocation singleton with no overrides. This is
the regression gate.

---

## US3 — Observability of Effective Per-Resource Budget (P3)

**Validates**: FR-009, FR-010.

### V3.1 — Unit test: status exposes effective budgets (FR-009)

```bash
cargo test --lib controllers::allocation::tests::status_exposes_effective_budgets
```

**Asserts**: `build_allocation_status(..., (90, 60))` produces a status with
`effective_cpu_budget_percent: 90` and `effective_memory_budget_percent: 60`.
And for the legacy case `(..., (80, 80))` both effective fields equal 80.

### V3.2 — Integration test: log carries effective per-resource budgets (FR-010)

```bash
cargo test --test budget_enforcement log_carries_effective_budgets
```

**Asserts**: the `DecisionSummary` (captured by the test harness) carries
`effective_cpu_budget_percent` and `effective_memory_budget_percent` matching
the Allocation status values, on admit/deny/dry-run-deny decisions.

### V3.3 — Integration test: fail-closed paths carry -1 (FR-010 exempt paths)

```bash
cargo test --test fail_safe effective_budgets_minus_one_on_error
```

**Asserts**: on a missing-allocation / stale-data rejection, the summary's
effective fields are `-1` (no budget context), matching the existing
`budget_percent: -1` convention.

---

## Edge case coverage (mapped to tests)

| Edge case (from spec) | Test |
|-----------------------|------|
| Both overrides absent (legacy) | V2.1, V2.4 |
| Both overrides present | V1.1, V1.3 |
| Only one override present (CPU then memory) | V1.1 (truth table), V1.3 (swap) |
| Override equals budgetPercent | V1.1 (row 5) |
| Override at 0 | V1.1 (row 7–8), plus existing `budgetPercent: 0` tests unchanged |
| Override at 100 | V1.1 (boundary) |
| All three consistent (80/80/80) | V1.1 (row 5), V2.1 |
| Negative override | CRD schema rejects (V2.3 proves schema validity) — no runtime test needed |
| Exemption interaction | Unchanged exemption tests (V2.4 regression) |
| Dry-run per-resource warning | `cargo test --test dry_run per_resource_warning` (new case) |

---

## Full validation command (run everything)

```bash
# All unit + integration + BDD (mocked apiserver):
cargo test

# Clippy + fmt gate (constitution quality gate):
cargo clippy -- -D warnings
cargo fmt --check

# Real-cluster (optional, requires throwaway cluster + KUBECONFIG):
erw-verify --kubeconfig ~/.kube/config
```

**Expected**: all green. The feature adds new tests; it does not modify the
behaviour the existing tests assert, so the existing suite passes unchanged.
