# Phase 0 Research — Per-Resource Budget Tracking (spec-012)

**Date**: 2026-08-06

This feature is a small additive change on a codebase the planning agent has
already mapped (specs 001–011 delivered). There are no `NEEDS CLARIFICATION`
tokens in the Technical Context. The research items below resolve the design
decisions that the plan must lock before Phase 1, grounded in the actual source.

---

## R1 — New spec fields: type, range attribute, naming

**Decision**: Add two `Option<i32>` fields to `AllocationSpec`, each with
`#[schemars(range(min = 0, max = 100))]`, serialising camelCase as
`cpuBudgetPercent` / `memoryBudgetPercent`.

**Rationale**: mirrors the existing `budget_percent: i32` field (which carries
the same `#[schemars(range(min = 0, max = 100))]` attribute at
`src/crd/allocation.rs:64`). `Option<>` because the fields are optional
overrides (FR-001); the range matches `budgetPercent`'s 0–100 contract. Field
names are the camelCase of the existing `budgetPercent` with the resource prefix
— operators write `cpuBudgetPercent` / `memoryBudgetPercent` in
`kubectl patch`, symmetric with `budgetPercent`.

**Alternatives considered**:
- A single nested struct `perResourceBudget: { cpu?: i32, memory?: i32 }` —
  rejected: it groups the overrides under a new sub-object, which is a heavier
  schema change and a less familiar `kubectl patch` path
  (`spec.perResourceBudget.cpu` vs `spec.cpuBudgetPercent`). The flat fields are
  consistent with how `budgetPercent`, `enforcementMode`, `excludedNamespaces`
  already live directly under `spec`.
- A map `budgetOverrides: { cpu: i32, memory: i32 }` — rejected for the same
  reason, plus a map implies arbitrary resource keys the webhook does not
  support (only CPU and memory are tracked).

---

## R2 — Resolution function: signature, purity, placement

**Decision**: Add a pure function in `src/crd/allocation.rs`:

```rust
/// Effective per-resource budgets after override-or-fallback resolution (FR-002).
/// Each resource resolves independently: its override if `Some`, else `budget_percent`.
pub fn resolve_effective_budgets(spec: &AllocationSpec) -> (i32, i32) {
    let cpu = spec.cpu_budget_percent.unwrap_or(spec.budget_percent);
    let memory = spec.memory_budget_percent.unwrap_or(spec.budget_percent);
    (cpu, memory)
}
```

Returns `(effective_cpu_budget, effective_memory_budget)` as `i32` (the same type
`ceiling()` takes today).

**Rationale**: the resolution is a pure function of the spec — no I/O, no async,
no cluster state. Placing it next to `AllocationSpec` (in `crd/allocation.rs`)
keeps it unit-testable in isolation (Principle VIII) and reusable by both the
controller (ceiling computation) and any future caller (e.g. the equalizer,
spec-013). The tuple return matches the existing `Figures = (i64, i64)` convention
used throughout the admission path.

**Alternatives considered**:
- Inline the `unwrap_or` at each call site — rejected: two call sites
  (controller recompute, webhook log) would duplicate the resolution logic, and
  the equalizer (spec-013) will need it too. A named function is the DRY,
  testable choice.
- Return a struct `EffectiveBudgets { cpu: i32, memory: i32 }` — viable but
  heavier than a tuple for two values; the codebase already uses `(i64, i64)`
  tuples for CPU/memory figures everywhere. Keep the convention.

---

## R3 — Controller: `build_allocation_status` signature change

**Decision**: Change `build_allocation_status` from taking a single
`budget_percent: i32` to taking per-resource budgets. Two viable shapes:

```rust
// Option A — take the spec, resolve internally:
pub fn build_allocation_status(
    allocated: (i64, i64),
    total_supply: (i64, i64),
    spec: &AllocationSpec,
) -> AllocationStatus

// Option B — take the resolved tuple, caller resolves:
pub fn build_allocation_status(
    allocated: (i64, i64),
    total_supply: (i64, i64),
    budgets: (i32, i32),  // (cpu, memory) already resolved
) -> AllocationStatus
```

**Recommendation: Option B.** The caller (`recompute`) resolves once via
`resolve_effective_budgets(&spec)` and passes the tuple; `build_allocation_status`
stays a pure arithmetic function (it already is — it does not touch the spec
today). This keeps the function's responsibility narrow (figures → status) and
makes the resolution an explicit, testable step at the call site.

The function then calls `ceiling(total_supply, cpu_budget)` and
`ceiling(total_supply, mem_budget)` — but note `ceiling()` today takes a single
`budget_percent` and applies it to BOTH figures. So either:
- (B1) call `ceiling()` twice with the same supply but different budgets, taking
  the `.0` (cpu) from the first call and `.1` (memory) from the second — wasteful
  but correct; or
- (B2) add a per-resource variant `ceiling_per_resource(total, (cpu_pct, mem_pct))`
  in `src/webhook/admission.rs` that computes each figure with its own budget.

**Recommendation: B2** — a small per-resource ceiling helper is clearer than
calling the pair-returning `ceiling()` twice and discarding half each time. The
existing `ceiling(total, percent) -> Figures` stays for backward compat / other
callers; the new helper composes it:

```rust
/// Per-resource ceiling (spec-012): each figure gets its own budget percent.
pub fn ceiling_per_resource(total: Figures, budgets: (i32, i32)) -> Figures {
    (ceiling_single(total.0, budgets.0), ceiling_single(total.1, budgets.1))
}
```

where `ceiling_single` is the existing inner `apply` closure extracted to a
function (pure, 128-bit-guarded). This is a minimal refactor of the current
`ceiling()` body, not new arithmetic.

**Rationale**: keeps the overflow guard in one place; the existing `ceiling()`
can delegate to `ceiling_per_resource((t,t), (p,p))` to preserve its exact
behaviour for any existing caller. Backward compatibility of the computed
ceilings is then provable: `ceiling_per_resource((t,t),(p,p)) == ceiling((t,t),p)`.

**Alternatives considered**:
- Option A (pass the spec) — couples a pure arithmetic function to the CRD type;
  rejected on Principle V (minimal surface).
- Call `ceiling()` twice and discard — works but is obscure; a reviewer would
  ask why. B2 is self-documenting.

---

## R4 — Status fields: effective per-resource budgets

**Decision**: Add two computed fields to `AllocationStatus`:

```rust
/// Effective CPU budget percent after override resolution (spec-012, FR-009).
/// Equals `spec.cpuBudgetPercent` if set, else `spec.budgetPercent`.
pub effective_cpu_budget_percent: i32,
/// Effective memory budget percent after override resolution (spec-012, FR-009).
pub effective_memory_budget_percent: i32,
```

Serialising camelCase as `effectiveCpuBudgetPercent` /
`effectiveMemoryBudgetPercent`. Populated by the controller in
`build_allocation_status` from the resolved tuple.

**Rationale**: FR-009 requires the applied budgets to be inspectable via
`kubectl get allocations -o yaml` without manual resolution. These are computed
(values the controller actually used), distinct from the raw spec fields. They
make US3 AC2 directly verifiable. They are `i32` (not `Option`) because by the
time status is written, resolution has already produced a concrete number.

**Alternatives considered**:
- A single nested `effectiveBudgets: {cpu, memory}` — rejected for the same flat-
  field consistency as R1.
- Source tracking (`effectiveCpuBudgetSource: "override"|"fallback"`) — considered
  but deferred: the source is derivable (`spec.cpuBudgetPercent == status.effectiveCpuBudgetPercent`
  ⟹ override-or-equal; otherwise fallback). Adding a source enum is YAGNI until
  operators ask for it. US3 AC1/AC3 are satisfied by the value fields + the log
  fields (R5).

---

## R5 — Log fields: effective per-resource budgets

**Decision**: Add two fields to `DecisionSummary` (`src/webhook/handler.rs`):

```rust
pub effective_cpu_budget_percent: i64,
pub effective_memory_budget_percent: i64,
```

Threaded through `DecisionSummary::decision()` (replacing the single
`budget_percent` parameter with the per-resource pair, or keeping `budget_percent`
for the summary's existing usages and adding the pair alongside). Emitted in the
structured log on every budget-resolved decision (admit/deny/dry-run-deny). The
fail-closed early-return paths (missing allocation, stale, exemption) keep
`budget_percent = -1` as today and set the effective fields to `-1` too (no
budget context, consistent with the existing convention at handler.rs:635).

**Rationale**: FR-010. The webhook reads the Allocation status (which now carries
`effectiveCpuBudgetPercent` / `effectiveMemoryBudgetPercent`), so the log fields
come directly from the status — no re-resolution in the webhook. This keeps the
webhook's budget-resolution logic unchanged (it reads ceilings from status; the
effective budgets ride along in the same status object).

**Alternatives considered**:
- Re-resolve in the webhook from `allocation.spec` — rejected: the webhook
  already trusts the controller's status for ceilings; re-resolving would be a
  second source of truth and could drift if the resolution function changed.
  Reading from status is the single-source principle.
- Emit only the legacy single `budget_percent` — rejected: FR-010 explicitly
  requires per-resource fields; with asymmetric budgets a single number is
  ambiguous.

---

## R6 — `budgetPercent` stays required (backward compat keystone)

**Decision**: The existing `budget_percent: i32` field on `AllocationSpec`
remains required (no `Option`, no `#[serde(default)]`). FR-006.

**Rationale**: if `budgetPercent` were optional, a singleton with no overrides
and no `budgetPercent` would have no budget at all — the resolution function
would have nothing to fall back to, and `ceiling()` would receive an undefined
value. Keeping it required guarantees every valid singleton has at least the
fallback budget. This is the keystone of backward compatibility (US2): a
pre-feature singleton (`budgetPercent: 80`, no overrides) remains valid and
produces identical ceilings.

The auto-created singleton (`default_allocation_singleton()` in
`src/controllers/allocation.rs:146`) adds `cpu_budget_percent: None,
memory_budget_percent: None` to the struct literal (FR-008) — preserving the
legacy default behaviour on first boot.

**Alternatives considered**:
- Make `budgetPercent` optional and require at least one of the three — rejected:
  complicates the schema (oneof validation), breaks backward compat for any
  client that assumes `budgetPercent` is present, and solves no real problem
  (operators who want per-resource budgets still set `budgetPercent` as the
  fallback).

---

## R7 — CRD manifest regeneration (deploy/crds.yaml)

**Decision**: `deploy/crds.yaml` is regenerated from `Allocation::crd()` (the
kube-rs derive produces the OpenAPI schema including the new optional fields
with their `minimum: 0, maximum: 100` constraints). The new fields appear under
`spec.versions[0].schema.openAPIV3Schema.properties.spec.properties` as
`cpuBudgetPercent` / `memoryBudgetPercent` (`type: integer, minimum: 0, maximum:
100`), NOT in the `required` array. Two new status fields appear under
`...properties.status.properties` as `effectiveCpuBudgetPercent` /
`effectiveMemoryBudgetPercent`.

**Rationale**: the manifest is generated from the Rust types (constitution:
"the Rust struct IS the schema source of truth"). No hand-edit. The CRD
apiVersion stays `v1` — additive optional fields are backward-compatible per
Kubernetes CRD semantics, no conversion webhook, no version bump.

**Alternatives considered**: none. This is the only correct path.

---

## R8 — `erw-verify` S9: asymmetric per-resource budget scenario

**Decision**: Add scenario S9 to
`src/bin/erw-verify/scenarios/enforcement.rs`:

- Patch the Allocation singleton with `cpuBudgetPercent: 95`,
  `memoryBudgetPercent: 30` (keep `budgetPercent` at the default 80 as fallback).
- Wait for the controller to recompute (poll `ceiling_memory_bytes` until it
  reflects the 30% memory budget, reusing the `apply_budget` wait pattern).
- Create a pod with high CPU request (fits under 95% CPU ceiling) and high
  memory request (exceeds 30% memory ceiling). Assert HTTP 403 denial.
- Assert the denial message names **memory** as the violated resource (not CPU),
  via the existing `denial_message` helper.
- Restore: clear the overrides (patch `cpuBudgetPercent: null,
  memoryBudgetPercent: null` via JSON merge patch — setting to `null` removes an
  optional field) and restore `budgetPercent: 80`. Wait for ceiling to settle.

Generalise `apply_budget` into `apply_budgets(client, cpu: Option<i32>,
memory: Option<i32>, fallback: i32)` — or add a sibling helper
`apply_per_resource_budgets` — so S9 can patch the override fields. The existing
S3–S6 scenarios continue to use the single-budget helper unchanged (they patch
`budgetPercent` only, which remains the fallback).

**Rationale**: FR-012. This reuses the existing enforcement-scenario harness
(timed, sequential, restore-on-exit) and the existing denial-detection helpers.
No new binary, no new CLI flag — S9 is just another scenario in the `run()`
vector. The wait-for-recompute pattern is already established in `apply_budget`
(lines 398–412).

**Alternatives considered**:
- A separate `erw-verify --scenario per-resource` mode — rejected: YAGNI; the
  scenario fits naturally in the existing enforcement group.
- Skip the real-cluster scenario and rely only on mocked integration tests —
  rejected: the mocked integration test cannot validate the controller→status→webhook
  round-trip against a real apiserver the way `erw-verify` does (Principle VI).

---

## R9 — Metrics: no new metric (YAGNI)

**Decision**: Do NOT add a new Prometheus metric for the effective per-resource
budget. The structured log fields (R5) and the status fields (R4) satisfy US3.
Metrics remain as-is (`capacity_admission_verdicts_total`, latency histogram,
capacity utilisation).

**Rationale**: a gauge of "current effective CPU budget %" would be a constant
between operator patches — low cardinality but also low signal. Operators
debugging a specific denial read the log line (which carries the budgets for
THAT decision) or `kubectl get allocations -o yaml` (which shows the current
state). A metric adds a third place to check without adding information.
Principle V (minimal surface) and Principle IV (observe what matters — verdicts
and capacity, not config) both point to skipping it. If operators later ask for
a budget gauge, it's a trivial additive follow-up.

**Alternatives considered**:
- `capacity_admission_effective_budget_percent{resource="cpu|memory"}` gauge —
  rejected as above. Document the deferral in README so the decision is
  discoverable.

---

## R10 — Backward-compatibility proof obligation

**Decision**: The tasks.md MUST include a dedicated test that proves FR-005
(byte-identical ceilings when no overrides): construct an `AllocationSpec` with
`budget_percent: 80, cpu_budget_percent: None, memory_budget_percent: None`,
call `build_allocation_status` with fixed supply, and assert the resulting
`ceiling_cpu_milli` / `ceiling_memory_bytes` equal the values produced by the
pre-feature code path (`ceiling(supply, 80)`). This is the US2 AC1 gate.

**Rationale**: backward compatibility is the release blocker (US2). A dedicated
test makes it a first-class assertion, not an emergent property. The test is
pure (no I/O) and lives in the controller's unit tests.

**Alternatives considered**: relying on the existing integration tests to "not
break" — insufficient, because the existing tests do not assert on the exact
ceiling values for the no-override case; they assert on admission verdicts, which
could coincidentally pass even if the ceiling drifted. An explicit ceiling-value
assertion is required.

---

## Summary

All 10 research items resolve to concrete, low-risk decisions. No item requires
external research (no new crate, no API lookup, no version matrix check) — the
feature is fully grounded in the existing codebase, which the planning agent has
already mapped across specs 001–011. Phase 1 design artifacts (data-model,
contract, quickstart) encode these decisions.
