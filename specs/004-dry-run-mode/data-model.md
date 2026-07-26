# Phase 1: Data Model — Dry-Run Enforcement Mode

**Feature**: spec-004 (dry-run mode) | **Date**: 2026-07-27

This document defines the data model changes for the dry-run enforcement mode.
It references the existing data model from spec-001 and describes only the
deltas.

---

## 1. EnforcementMode enum

A new type representing the two valid enforcement modes.

```rust
/// The enforcement mode of the capacity admission webhook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnforcementMode {
    /// Reject pods that exceed the budget (the default, fail-closed behaviour).
    Enforce,
    /// Admit pods that exceed the budget, surfacing the would-be rejection
    /// as an admission warning. Fail-closed paths still reject.
    DryRun,
}
```

Serialisation: `"enforce"` and `"dry-run"` (kebab-case, matching the CRD field
values operators type).

### Default resolution

Because the field is optional on the CRD spec, a helper resolves any
unrecognised or absent value to `Enforce`:

```rust
/// Resolve the effective enforcement mode, defaulting to Enforce for
/// None or any unrecognised value (FR-003).
pub fn resolve_enforcement_mode(mode: Option<EnforcementMode>) -> EnforcementMode {
    mode.unwrap_or(EnforcementMode::Enforce)
}
```

---

## 2. AllocationSpec change

The `AllocationSpec` struct gains one optional field:

```rust
pub struct AllocationSpec {
    /// Maximum allowed allocation as a percentage of total allocatable capacity
    /// (0–100). Applied to both CPU and RAM independently.
    #[schemars(range(min = 0, max = 100))]
    pub budget_percent: i32,

    /// Enforcement mode: `enforce` (default) or `dry-run`.
    /// When absent, treated as `enforce` (FR-003).
    pub enforcement_mode: Option<EnforcementMode>,  // NEW
}
```

### CRD YAML schema fragment (generated)

The OpenAPI schema for the `spec` properties now includes:

```yaml
properties:
  budgetPercent:
    type: integer
    minimum: 0
    maximum: 100
  enforcementMode:                    # NEW
    type: string
    enum:
      - enforce
      - dry-run
    description: >-
      Enforcement mode. When 'dry-run', over-budget pods are admitted with
      a warning instead of rejected. Defaults to 'enforce' when absent.
```

The field is NOT in `required` — it is optional, so existing instances without
it remain valid.

---

## 3. Decision state machine (dry-run path)

The admission decision pipeline is unchanged up to the budget check. The
branching at `check_budget` result is the only insertion point:

```text
                    ┌──────────────────┐
                    │  AdmissionRequest │
                    │   arrives        │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │  Deserialise     │──── error ──▶ REJECT (fail-closed)
                    │  AdmissionReview │    (400)      [regardless of mode]
                    └────────┬─────────┘
                             │ ok
                    ┌────────▼─────────┐
                    │  Read Allocation  │──── missing ──▶ REJECT (fail-closed)
                    │  from cache       │    (500)      [regardless of mode]
                    └────────┬─────────┘
                             │ present
                    ┌────────▼─────────┐
                    │  Check freshness  │──── stale ──▶ REJECT (fail-closed)
                    │  of status        │    (500)      [regardless of mode]
                    └────────┬─────────┘
                             │ fresh
                    ┌────────▼─────────┐
                    │  Read ClusterCap  │──── missing ──▶ REJECT (fail-closed)
                    │  from cache       │    (500)      [regardless of mode]
                    └────────┬─────────┘
                             │ present
                    ┌────────▼─────────┐
                    │  Extract pod      │──── parse err ──▶ REJECT (fail-closed)
                    │  requests         │    (400)        [regardless of mode]
                    └────────┬─────────┘
                             │ ok
                    ┌────────▼─────────┐
                    │  check_budget()   │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
         Admit          Deny           (no other outcome)
              │              │
              ▼              ▼
     ┌────────────────┐  ┌─────────────────────────────┐
     │ ALLOW          │  │ Read enforcement_mode        │
     │ (normal)       │  │  from allocation spec        │
     │                │  └──────────┬──────────────────┘
     │                │             │
     │                │    ┌────────▼────────┐
     │                │    │  Enforce?       │
     │                │    └──┬───────────┬──┘
     │                │    YES│           │NO (DryRun)
     │                │       │           │
     │                │       ▼           ▼
     │                │  ┌─────────┐ ┌──────────────────────┐
     │                │  │ DENY    │ │ ALLOW + warnings      │
     │                │  │ (reject)│ │ (dry-run would-deny)  │
     │                │  │ (403)   │ │ summary: DryRunDeny   │
     │                │  └─────────┘ └──────────────────────┘
     └────────────────┘
```

**Key invariant**: every fail-closed path (left column) returns BEFORE
`check_budget` is reached. The enforcement-mode branch is ONLY reachable when
the budget check itself produces a `Deny`. Therefore, dry-run mode can only
convert budget denials — never error rejections. This is the structural
guarantee for FR-006 / Principle I.

---

## 4. DecisionSummary change

The `DecisionSummary` struct gains one field:

```rust
pub struct DecisionSummary {
    // ... existing fields ...
    pub enforcement_mode: String,  // NEW: "enforce" or "dry_run"
}
```

Populated from `resolve_enforcement_mode(...)` at decision time. Threaded
through `emit_log` to appear in every structured log entry (FR-009).

---

## 5. DecisionVerdict change

```rust
pub enum DecisionVerdict {
    Allow,
    Deny,
    DryRunDeny,  // NEW
    Error,
}
```

---

## 6. Metrics VerdictLabel change

```rust
pub enum VerdictLabel {
    Allow,
    Deny,
    DryRunDeny,  // NEW — serialised as "dry_run_deny"
    Error,
}
```

The `Metrics::new()` pre-creation loop adds `DryRunDeny` to the verdict
iteration, so the `{resource=*, verdict="dry_run_deny"}` series appear at zero
from startup.

---

## 7. AdmissionResponse warnings construction

When dry-run mode converts a deny to an admit, the warning text is the same
message a real rejection would carry — built from the violations:

```text
Budget violations (dry-run): <violation_line_1>\n<violation_line_2>
```

Each `<violation_line_N>` is the output of `BudgetViolation::message_line()` —
identical to the rejection message format. A prefix `"Budget violations
(dry-run): "` distinguishes the warning from a real rejection when viewed in
cluster events or log aggregators.

The `response.warnings` field is set to `Some(vec![warning_text])`.
`response.allowed` remains `true`. `response.status` is not set (the pod is
admitted, not rejected).

---

## 8. Validation rules

| Rule | Source | Enforcement |
|------|--------|-------------|
| `enforcementMode` absent → `enforce` | FR-003 | `resolve_enforcement_mode` helper |
| `enforcementMode` invalid → `enforce` | FR-003, edge case | serde deserialisation failure → reflector does not cache the object → webhook sees stale/missing allocation → fail-closed. Additionally, `resolve_enforcement_mode(None)` defaults to Enforce as a runtime safety net. |
| `enforcementMode: enforce` → existing behaviour unchanged | FR-003 | no code path alteration |
| `enforcementMode: dry-run` → over-budget pods admitted with warning | FR-004, FR-005 | deny-branch conversion in `evaluate` |
| Fail-closed paths reject in both modes | FR-006 | architectural: error paths return before `check_budget` |
| Auto-created singleton defaults to `enforce` | FR-010 | `default_allocation_singleton()` seeds `Some(Enforce)` |
