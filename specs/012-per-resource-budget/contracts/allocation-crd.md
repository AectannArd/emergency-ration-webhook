# Contract: Allocation CRD — Per-Resource Budget Extension (spec-012)

**Status**: AUTHORITATIVE — the implementing agent MUST satisfy this contract.
Where `data-model.md` or `quickstart.md` appear to disagree with this file, THIS
file wins (and the disagreeing doc is a planning defect to fix, not behaviour to
implement).

**Scope**: the diff the spec-012 feature introduces on the existing `Allocation`
CRD (`emergency-ration.dev/v1`, kind `Allocation`, cluster-scoped singleton
`cluster-allocation`). All fields not listed here are unchanged.

---

## 1. Spec fields (operator-configurable)

### 1.1 `budgetPercent` (UNCHANGED)

- **Type**: `integer`, required.
- **Range**: `minimum: 0, maximum: 100`.
- **Role**: the fallback budget for any resource whose override is absent
  (FR-002, FR-006). When NEITHER override is set, this is the budget for both
  CPU and RAM (legacy behaviour, byte-identical — FR-005).
- **Serialisation**: `budgetPercent` (camelCase, unchanged).

### 1.2 `cpuBudgetPercent` (NEW — spec-012)

- **Type**: `integer`, **optional** (absent allowed).
- **Range**: `minimum: 0, maximum: 100` (enforced by CRD schema; apiserver
  rejects out-of-range).
- **Role**: when present, the CPU ceiling is derived from THIS value instead of
  `budgetPercent` (FR-001, FR-002). When absent, CPU falls back to
  `budgetPercent`.
- **Serialisation**: `cpuBudgetPercent` (camelCase). MUST serialise as
  `cpuBudgetPercent: <int>` when `Some`, and MUST be **absent from the JSON**
  (not `null`) when `None` — matching `Option<i32>` serde semantics and the
  existing `enforcementMode`/`excludedNamespaces` pattern (US2 AC2).

### 1.3 `memoryBudgetPercent` (NEW — spec-012)

- **Type**: `integer`, **optional**.
- **Range**: `minimum: 0, maximum: 100`.
- **Role**: symmetric to `cpuBudgetPercent` for RAM.
- **Serialisation**: `memoryBudgetPercent` (camelCase), absent-when-None (same
  rule as 1.2).

### 1.4 Resolution rule (FR-002) — the core contract

For each resource, the **effective budget** is:

```
effective_cpu    = cpuBudgetPercent    if cpuBudgetPercent    is present
                   else budgetPercent

effective_memory = memoryBudgetPercent if memoryBudgetPercent is present
                   else budgetPercent
```

Each resource resolves **independently**. A singleton may have CPU overridden
and memory falling back (or vice versa) — this is a valid partial configuration
(US1 AC3), not an error. Resolution MUST be a total function (no panic on any
spec that passes schema validation).

---

## 2. Status fields (controller-computed)

### 2.1 Existing ceilings (UNCHANGED shape, NEW derivation)

- `ceiling_cpu_milli` (`integer`) — now derived from `effective_cpu`, not
  necessarily `budgetPercent`.
- `ceiling_memory_bytes` (`integer`) — now derived from `effective_memory`.
- Arithmetic: `floor(total_allocatable * effective_budget / 100)` per resource,
  with 128-bit intermediates saturating to i64 (the existing overflow guard).
- **Backward-compat guarantee**: when both overrides are absent,
  `ceiling_cpu_milli` and `ceiling_memory_bytes` are byte-identical to the
  pre-spec-012 values for the same `budgetPercent` and supply (FR-005).

### 2.2 `effectiveCpuBudgetPercent` (NEW — spec-012, FR-009)

- **Type**: `integer`.
- **Value**: the effective CPU budget the controller used to compute
  `ceiling_cpu_milli` (i.e. the resolved value from §1.4).
- **Purpose**: observability — operators read this via
  `kubectl get allocations cluster-allocation -o yaml` to see the applied CPU
  budget without manually resolving override-vs-fallback (US3 AC2).
- **Serialisation**: `effectiveCpuBudgetPercent` (camelCase).

### 2.3 `effectiveMemoryBudgetPercent` (NEW — spec-012, FR-009)

- Symmetric to §2.2 for memory. Serialises as `effectiveMemoryBudgetPercent`.

### 2.4 Other status fields (UNCHANGED)

`allocated_cpu_milli`, `allocated_memory_bytes`, `utilization_percent_cpu`,
`utilization_percent_memory`, `last_updated` — unchanged.

---

## 3. Controller behaviour

### 3.1 Singleton auto-creation (`ensure_singleton` / `default_allocation_singleton`)

- Seeds `budget_percent = 80` (the existing `DEFAULT_BUDGET_PERCENT`) — UNCHANGED.
- Seeds `cpu_budget_percent: None`, `memory_budget_percent: None` — NEW (FR-008).
  The auto-created singleton has no overrides, so a fresh cluster boots in legacy
  mode (both resources at 80%).
- The controller NEVER modifies `budget_percent` based on the overrides, and
  NEVER infers the overrides (FR-007). Overrides are operator-set only.

### 3.2 Reconcile (`recompute`)

- `GET` the Allocation singleton (as today).
- Resolve per-resource budgets via `resolve_effective_budgets(&spec)` →
  `(effective_cpu, effective_memory)`.
- Compute ceilings via `ceiling_per_resource(supply, (effective_cpu, effective_memory))`.
- Write the full status (including the two new effective-budget fields) via the
  existing merge-patch-status path.
- If the singleton is missing (404), recreate it (as today) and return — next
  tick reads the fresh spec.

---

## 4. Webhook behaviour

### 4.1 Enforcement (NO behavioural change to the decision logic)

- The webhook reads `ceiling_cpu_milli` / `ceiling_memory_bytes` from the
  Allocation status (as today). It does NOT re-resolve budgets, does NOT read
  the override fields, does NOT recompute ceilings.
- `check_budget(allocated, pod_request, ceilings)` is called UNCHANGED — it
  already evaluates CPU and RAM independently and reports per-resource
  violations (FR-004, FR-011).
- Dry-run mode (spec-004): a memory-only violation produces a memory-only
  warning, symmetric to the enforce-mode per-resource violation (edge case).
- Exemptions (spec-008): the exemption path runs BEFORE budget resolution and is
  unaffected.

### 4.2 Observability (FR-010)

- On every budget-resolved decision (admit, deny, dry-run-deny), the structured
  log carries:
  - `effective_cpu_budget_percent` (sourced from `status.effectiveCpuBudgetPercent`)
  - `effective_memory_budget_percent` (sourced from `status.effectiveMemoryBudgetPercent`)
- The legacy `budget_percent` field is also emitted (from `spec.budgetPercent`),
  for backward compat in existing log consumers.
- Fail-closed paths that return before budget resolution (missing allocation,
  stale, quantity-parse failure, timeout, panic, exemption) set both effective
  fields to `-1` (the existing "no budget context" sentinel). FR-010 exempts
  these paths from carrying budget figures.

---

## 5. CRD manifest (`deploy/crds.yaml`)

Regenerated from `Allocation::crd()`. The diff vs the current manifest:

- Under `...spec.properties`: add `cpuBudgetPercent` and `memoryBudgetPercent`,
  each `{type: integer, minimum: 0, maximum: 100}`. NOT added to
  `...spec.required`.
- Under `...status.properties`: add `effectiveCpuBudgetPercent` and
  `effectiveMemoryBudgetPercent`, each `{type: integer}`.
- `apiVersion` of the CRD object: unchanged (`apiextensions.k8s.io/v1`).
- The served version stays `v1` — no new storage version, no conversion webhook.

---

## 6. erw-verify scenario S9 (FR-012)

A new scenario in `src/bin/erw-verify/scenarios/enforcement.rs`, added to the
`run()` vector as `timed("S9", "per-resource asymmetric budgets", s9(client))`:

1. Patch `cluster-allocation` spec: `cpuBudgetPercent: 95`,
   `memoryBudgetPercent: 30` (leave `budgetPercent: 80`).
2. Wait for the controller to recompute: poll until
   `status.ceiling_memory_bytes == floor(supply.mem * 30 / 100)` (reuse the
   `apply_budget` wait-loop pattern, lines 398–412).
3. Create a pod with a high CPU request (fits under 95% CPU ceiling) and a high
   memory request (exceeds 30% memory ceiling). Expect HTTP 403 denial.
4. Assert the denial message names **memory** as the violated resource (use the
   existing `denial_message` helper; the message contains "memory", not "cpu").
5. Cleanup: patch `cpuBudgetPercent: null, memoryBudgetPercent: null` (JSON merge
   patch with `null` removes the optional fields) and restore `budgetPercent: 80`;
   wait for ceiling to settle (reuse `restore_budget`).

The `apply_budget` helper is generalised (or a sibling
`apply_per_resource_budgets` added) to patch the override fields. The existing
S3–S6 scenarios continue to use the single-budget helper unchanged.

---

## 7. Backward compatibility (release gate — US2 / FR-005 / FR-006)

These are hard requirements, not best-effort:

1. `budgetPercent` remains **required** on the spec (FR-006). A singleton with
   no overrides and no `budgetPercent` is invalid (the resolution would have no
   fallback).
2. A singleton with `budgetPercent: 80` and no overrides produces ceilings
   byte-identical to the pre-spec-012 controller (FR-005). This MUST be asserted
   by a dedicated unit test (research R10).
3. The override fields serialise as **absent** (not `null`) when `None` (US2 AC2)
   — matching `Option<i32>` serde semantics.
4. The auto-created singleton seeds both overrides as `None` (FR-008).
5. The existing integration/BDD/E2E suites pass unchanged on a no-override
   singleton (US2 AC3, SC-002).
