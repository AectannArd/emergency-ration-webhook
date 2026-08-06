# Phase 1 Data Model — Per-Resource Budget Tracking (spec-012)

**Date**: 2026-08-06

This feature adds two optional spec fields and two computed status fields to the
existing `Allocation` CRD, plus one pure resolution function and one per-resource
ceiling helper. No new entity, no state machine, no new CRD.

---

## 1. Entities

### 1.1 Allocation CRD (extended)

The existing cluster-scoped singleton (`cluster-allocation`,
`emergency-ration.dev/v1`, kind `Allocation`). Only the diff is shown; all other
fields are unchanged.

#### Spec additions (`AllocationSpec`)

Two new optional fields, mirroring the existing `budget_percent`:

```rust
// src/crd/allocation.rs — additions to AllocationSpec

pub struct AllocationSpec {
    // ... existing fields unchanged ...
    pub budget_percent: i32,              // unchanged (FR-006: stays required)
    pub enforcement_mode: Option<EnforcementMode>,        // unchanged
    pub excluded_namespaces: Option<Vec<String>>,         // unchanged
    pub excluded_priority_classes: Option<Vec<String>>,   // unchanged

    /// spec-012 (FR-001): optional CPU budget override (0–100). When present,
    /// the CPU ceiling is derived from this value instead of `budget_percent`.
    /// When absent, CPU falls back to `budget_percent` (FR-002).
    #[schemars(range(min = 0, max = 100))]
    pub cpu_budget_percent: Option<i32>,

    /// spec-012 (FR-001): optional memory budget override (0–100). Symmetric to
    /// `cpu_budget_percent` for RAM.
    #[schemars(range(min = 0, max = 100))]
    pub memory_budget_percent: Option<i32>,
}
```

Serialisation (camelCase, via the existing `#[serde(rename_all = "camelCase")]`
on the struct): `cpuBudgetPercent`, `memoryBudgetPercent`. Both absent from the
schema's `required` array (optional). Both carry `minimum: 0, maximum: 100` in
the generated OpenAPI schema.

#### Status additions (`AllocationStatus`)

Two new computed fields:

```rust
// src/crd/allocation.rs — additions to AllocationStatus

pub struct AllocationStatus {
    // ... existing fields unchanged ...
    pub allocated_cpu_milli: i64,
    pub allocated_memory_bytes: i64,
    pub ceiling_cpu_milli: i64,
    pub ceiling_memory_bytes: i64,
    pub utilization_percent_cpu: f64,
    pub utilization_percent_memory: f64,
    pub last_updated: String,

    /// spec-012 (FR-009): the effective CPU budget percent the controller used
    /// to compute `ceiling_cpu_milli`. Equals `spec.cpuBudgetPercent` if set,
    /// else `spec.budgetPercent`. Exposed for observability (US3 AC2).
    pub effective_cpu_budget_percent: i32,

    /// spec-012 (FR-009): the effective memory budget percent. Symmetric.
    pub effective_memory_budget_percent: i32,
}
```

Serialisation: `effectiveCpuBudgetPercent`, `effectiveMemoryBudgetPercent`.

### 1.2 No new entities

No new CRD, no new singleton, no new component. The `ClusterCapacity` CRD is
unaffected.

---

## 2. Resolution function (pure)

```rust
// src/crd/allocation.rs

/// Effective per-resource budgets after override-or-fallback resolution (FR-002).
///
/// Each resource resolves independently: its override (`cpu_budget_percent` /
/// `memory_budget_percent`) if `Some`, else `budget_percent` as fallback.
/// Returns `(effective_cpu_budget, effective_memory_budget)`, each in 0–100
/// (clamped by the CRD schema; this function does not re-clamp).
pub fn resolve_effective_budgets(spec: &AllocationSpec) -> (i32, i32) {
    let cpu = spec.cpu_budget_percent.unwrap_or(spec.budget_percent);
    let memory = spec.memory_budget_percent.unwrap_or(spec.budget_percent);
    (cpu, memory)
}
```

**Truth table** (all values are percentages):

| `budget_percent` | `cpu_budget_percent` | `memory_budget_percent` | effective CPU | effective memory |
|------------------|----------------------|-------------------------|---------------|------------------|
| 80               | None                 | None                    | 80            | 80               |
| 80               | Some(90)             | None                    | 90            | 80               |
| 80               | None                 | Some(60)                | 80            | 60               |
| 80               | Some(90)             | Some(60)                | 90            | 60               |
| 80               | Some(80)             | Some(80)                | 80            | 80               |
| 70               | Some(90)             | None                    | 90            | 70               |
| 0                | None                 | None                    | 0             | 0                |
| 100              | Some(0)              | Some(100)               | 0             | 100              |

Row 1 = legacy/backward-compat (US2). Rows 2–4 = partial/full override (US1).
Row 5 = override-equals-fallback (edge case). Rows 7–8 = boundary (0%/100%).

---

## 3. Ceiling computation (controller)

### 3.1 Per-resource ceiling helper

```rust
// src/webhook/admission.rs

/// Compute the budget ceiling for a single resource (spec-012).
/// `floor(total * budget_percent / 100)` with 128-bit intermediates,
/// saturating to i64. Same arithmetic as `ceiling()`, extracted per-resource.
pub fn ceiling_single(total: i64, budget_percent: i32) -> i64 {
    let budget = budget_percent.clamp(0, 100) as i128;
    let product = total as i128 * budget;
    ((product / 100).min(i64::MAX as i128)) as i64
}

/// Per-resource ceiling pair (spec-012). Each figure gets its own budget percent.
pub fn ceiling_per_resource(total: Figures, budgets: (i32, i32)) -> Figures {
    (
        ceiling_single(total.0, budgets.0),
        ceiling_single(total.1, budgets.1),
    )
}
```

The existing `ceiling(total: Figures, budget_percent: i32) -> Figures` is refactored
to delegate:

```rust
pub fn ceiling(total_allocatable: Figures, budget_percent: i32) -> Figures {
    ceiling_per_resource(total_allocatable, (budget_percent, budget_percent))
}
```

**Backward-compatibility proof**: `ceiling((t_cpu, t_mem), p)` now equals
`ceiling_per_resource((t_cpu, t_mem), (p, p))` = `(ceiling_single(t_cpu, p), ceiling_single(t_mem, p))`.
The old body computed `(apply(t_cpu), apply(t_mem))` with the same `apply` —
identical arithmetic. So any existing caller of `ceiling()` gets byte-identical
results (FR-005).

### 3.2 Controller `build_allocation_status` (edited)

```rust
// src/controllers/allocation.rs

pub fn build_allocation_status(
    allocated: (i64, i64),
    total_supply: (i64, i64),
    budgets: (i32, i32),   // spec-012: was `budget_percent: i32`, now resolved per-resource
) -> AllocationStatus {
    let ceilings = ceiling_per_resource(total_supply, budgets);
    AllocationStatus {
        allocated_cpu_milli: allocated.0,
        allocated_memory_bytes: allocated.1,
        ceiling_cpu_milli: ceilings.0,
        ceiling_memory_bytes: ceilings.1,
        utilization_percent_cpu: ratio(allocated.0, ceilings.0),
        utilization_percent_memory: ratio(allocated.1, ceilings.1),
        effective_cpu_budget_percent: budgets.0,      // spec-012 FR-009
        effective_memory_budget_percent: budgets.1,   // spec-012 FR-009
        last_updated: now_rfc3339(),
    }
}
```

### 3.3 Controller `recompute` (edited)

The `recompute` function today reads `allocation.spec.budget_percent` (single
value). After spec-012 it reads the whole spec and resolves:

```rust
// src/controllers/allocation.rs — inside recompute()
let budgets = match allocation_api.get(CLUSTER_ALLOCATION_NAME).await {
    Ok(allocation) => resolve_effective_budgets(&allocation.spec),
    Err(err) if is_not_found(&err) => { /* recreate singleton, return */ }
    Err(err) => { /* skip recompute */ return; }
};
// ... rest unchanged, passes `budgets` to build_allocation_status ...
```

---

## 4. Webhook observability (handler)

### 4.1 DecisionSummary additions

```rust
// src/webhook/handler.rs — DecisionSummary gains two fields

pub struct DecisionSummary {
    // ... existing fields ...
    pub budget_percent: i64,   // unchanged (legacy fallback, for back-comat in log consumers)
    // spec-012 FR-010:
    pub effective_cpu_budget_percent: i64,
    pub effective_memory_budget_percent: i64,
    // ...
}
```

### 4.2 Threading

`DecisionSummary::decision()` currently takes `budget_percent: i32`. After
spec-012 it takes the per-resource pair (or the resolved tuple), sourced from the
Allocation **status** (not re-resolved in the webhook — R5):

```rust
// in the decide() function, after fetching allocation.status:
let effective_cpu = status.effective_cpu_budget_percent;
let effective_mem = status.effective_memory_budget_percent;
// pass to DecisionSummary::decision(..., (effective_cpu, effective_mem), ...)
```

The existing `budget_percent` field on the summary is populated from
`allocation.spec.budget_percent` as today (kept for backward compat in any log
consumer that reads it). The two new fields are emitted alongside in the
structured log.

### 4.3 Fail-closed paths

The early-return paths (`reject_outcome`, `exempt`) set
`effective_cpu_budget_percent = -1`, `effective_memory_budget_percent = -1`,
matching the existing `budget_percent = -1` sentinel for "no budget context"
(handler.rs:635). FR-010 explicitly exempts these paths.

---

## 5. Validation rules

| Rule | Enforced by | FR |
|------|-------------|----|
| `cpuBudgetPercent` ∈ [0, 100] if present | `#[schemars(range(min=0,max=100))]` on the field → CRD OpenAPI schema → apiserver validation | FR-001 |
| `memoryBudgetPercent` ∈ [0, 100] if present | same | FR-001 |
| `budgetPercent` required (not optional) | field type `i32` (not `Option`), in schema `required` array | FR-006 |
| Overrides optional (absent allowed) | `Option<i32>`, NOT in schema `required` array | FR-001 |
| Resolution is total (no panic on any valid spec) | `unwrap_or` on `Option<i32>` — always falls back | FR-002 |
| Ceilings byte-identical when no overrides | `ceiling()` delegates to `ceiling_per_resource((t,t),(p,p))`; proven in §3.1 | FR-005 |

---

## 6. State transitions

No new state machine. The Allocation CRD's lifecycle (absent → created by
controller → status-patched by controller → spec-patched by operator) is
unchanged. The only new transition is operator-set overrides on the spec, which
the controller picks up on the next recompute tick (same as `budgetPercent`
changes today — runtime-adjustable, no restart, US1 AC4).

---

## 7. Algorithm: per-resource budget resolution → ceiling → enforcement

```
Operator sets Allocation.spec:
  budgetPercent = 80                  (required, fallback)
  cpuBudgetPercent = 90   (optional)
  memoryBudgetPercent = 60 (optional)

Allocation Controller recompute tick:
  spec = GET cluster-allocation
  (cpu_budget, mem_budget) = resolve_effective_budgets(spec)
      → cpu_budget = 90 (override), mem_budget = 60 (override)
  supply = ClusterCapacity.status.total_allocatable (cpu_milli, mem_bytes)
  ceilings = ceiling_per_resource(supply, (90, 60))
      → ceiling_cpu_milli = floor(supply.cpu * 90 / 100)
      → ceiling_memory_bytes = floor(supply.mem * 60 / 100)
  PATCH Allocation.status = {
      ceiling_cpu_milli, ceiling_memory_bytes,
      effective_cpu_budget_percent: 90,
      effective_memory_budget_percent: 60,
      ...allocated, utilization, last_updated...
  }

Admission Webhook decide(pod):
  status = Allocation.status (from reflector cache)
  ceilings = (status.ceiling_cpu_milli, status.ceiling_memory_bytes)
  effective = (status.effective_cpu_budget_percent, status.effective_memory_budget_percent)
  verdict = check_budget(allocated, pod_request, ceilings)  // UNCHANGED function
  log: effective_cpu_budget_percent=90, effective_memory_budget_percent=60, ...
```

The enforcement function (`check_budget`) is completely unchanged — it already
evaluates CPU and RAM independently against the ceilings it is given. The
per-resource budgets flow in via the ceilings, which the controller now computes
independently.
