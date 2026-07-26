# Implementation Plan: Dry-Run Enforcement Mode

**Branch**: `spec/dry-run-mode` | **Date**: 2026-07-27 | **Spec**:
[spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-dry-run-mode/spec.md`

## Summary

The dry-run enforcement mode lets operators install the capacity admission
webhook in an audit/shadow configuration: the webhook evaluates every admission
request normally but admits over-budget pods with a warning instead of rejecting
them. This is toggled via a new optional field `enforcementMode` on the
Allocation CRD spec (`enforce` | `dry-run`, default `enforce`), runtime-adjustable
via `kubectl patch` without restart. Fail-closed paths (capacity data
missing/stale, timeout, panic, malformed request) reject regardless of mode —
dry-run converts only over-budget denials. The feature is fully additive: one
new CRD field, one new enum variant in the decision pipeline, one new metrics
verdict label, and one new log field. No existing behaviour changes in enforce
mode.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024), MSRV recorded in `Cargo.toml`.

**Primary Dependencies**: No new dependencies. The implementation uses existing
crates only:
- `kube` 4.2.0 — `AdmissionResponse.warnings` field (confirmed present in
  `kube-core/src/admission.rs:314`), CRD derive macro.
- `k8s-openapi` 0.28.0 — no new types needed.
- `serde` / `schemars` 1.0 — enum serialisation + CRD OpenAPI schema generation.
- `prometheus` 0.14 — new `VerdictLabel` variant.
- `tracing` 0.1 — new log field.

**Storage**: N/A (state lives in CRDs, as before). The new field is on the
Allocation CRD spec; no new storage.

**Testing**: Same three-tier strategy as spec-001:
- Unit tests: in-module `#[cfg(test)]` for the new enum, resolution helper,
  `evaluate` dry-run branch, log/metrics variants.
- Integration tests: new `tests/integration/dry_run.rs` (tower-test mocked
  apiserver).
- BDD: new `tests/bdd/features/dry_run.feature` + step definitions.

**Target Platform**: Linux container, Kubernetes Deployment (unchanged).

**Project Type**: Kubernetes operator / admission webhook (unchanged).

**Performance Goals**: Unchanged. The dry-run path performs the same budget
check as enforce; the only addition is setting the `warnings` field on the
response struct (in-memory, no I/O). Provisional targets (p99 < 100 ms, p50 <
50 ms) apply unchanged (SC-005).

**Constraints**: Unchanged. Fail-closed paths reject in both modes (FR-006 /
Principle I). Validating-only (no mutation). N-2 Kubernetes support window
(1.34–1.36).

**Scale/Scope**: Small additive feature. Estimated delta: ~1 new enum, ~1 new
struct field on `AllocationSpec`, ~1 new variant on `DecisionVerdict` and
`VerdictLabel`, ~1 new field on `DecisionSummary`, ~30 lines of logic change in
`evaluate()`, ~1 new integration test file, ~1 new BDD feature file, README
update.

## Constitution Check (Pre-Design)

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | Dry-run mode converts only over-budget denials. All fail-closed paths (missing/stale data, timeout, panic, malformed request) return BEFORE `check_budget` and are structurally unaffected by `enforcementMode`. FR-006 + US2 enforce this. The `enforcementMode` field defaults to `enforce` when absent/invalid (FR-003), so the safe default is always fail-closed budget enforcement. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | The budget calculation is unchanged. Dry-run mode does not alter the budget arithmetic, the ceiling, or the allocation tracking — it only changes the *verdict* for over-budget pods from deny to admit-with-warning. The budget is still computed identically; the operator sees exactly what would be blocked. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The enforcement mode is an explicitly-declared, tested, and documented configuration field on the Allocation CRD spec. Every decision path maps to a known outcome: enforce-deny, dry-run-deny-admit, or fail-closed-reject. No new undefined category is introduced. |
| IV | Observability Before Optimisation | ✅ PASS | Dry-run decisions emit structured logs (WARN, `decision=dry_run_deny`, `enforcement_mode` field) and metrics (`verdict=dry_run_deny` label) — first-class observability from the first decision. The mode is included in every log entry (FR-009). This is not deferred to a polish phase. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | The feature adds one optional field to one existing CRD — no new component, no new webhook type, no new data store. The enforcement mode is read from the same cached Allocation singleton the webhook already reads for `budgetPercent`. The Allocation Controller creates the default and then never touches the field (it is a webhook concern). No new dependencies. |
| VI | Integration Test Coverage of Main and Exceptional Workflows | ✅ PASS | New integration tests (`tests/integration/dry_run.rs`) cover: dry-run admit of over-budget pod, dry-run fail-closed on stale data, enforce-mode unchanged, mode switch. New BDD feature (`dry_run.feature`) covers the same scenarios in Gherkin. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | The admission `warnings` field was introduced in K8s 1.19 (GA). All versions in the support window (1.34–1.36) are far above this floor. No new Kubernetes API is used — the field is on the existing AdmissionReview type. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | Tests will be written first: unit tests for the enum/resolution helper, then the `evaluate` dry-run branch, then integration tests, then BDD. The implementation plan mandates Red-Green-Refactor. |
| IX | Editor Configuration as Code | ✅ PASS | All new files (`.rs`, `.feature`, `.md`) will comply with `.editorconfig`. No new file types are introduced. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | The README will be updated in the same PR: new "Enforcement Modes" section, Allocation CRD spec table update, Failure Modes update, Metrics update, Logging update. R12 in research.md itemises every README change. |
| XI | CI-Green Completion Gate | ✅ PASS | All CI jobs (fmt, clippy, test, E2E, editorconfig) must pass before merge. The feature does not introduce any CI-incompatible change. |
| XII | .temp/ Scratch Space | ✅ PASS | Agent intercommunication files (if any during implementation) go in `.temp/`, which is git-ignored. No tracked scratch files. |

**Gate result**: ✅ ALL PASS. No violations. No Complexity Tracking entries needed.

## Project Structure

### Documentation (this feature)

```text
specs/004-dry-run-mode/
├── plan.md                              # This file
├── research.md                          # Phase 0: 12 research decisions
├── data-model.md                        # Phase 1: CRD schema, state machine, validation rules
├── quickstart.md                        # Phase 1: validation scenarios (7 scenarios)
├── contracts/
│   └── admission-webhook-dry-run.md     # Phase 1: dry-run response/contract amendment
├── checklists/
│   └── requirements.md                  # Specify-phase quality checklist
└── spec.md                              # The feature specification
```

### Source Code (repository root — deltas only)

```text
src/
├── crd/
│   ├── allocation.rs        # +enforcement_mode: Option<EnforcementMode> on AllocationSpec
│   └── mod.rs               # +pub use EnforcementMode
├── webhook/
│   ├── handler.rs           # +DryRunDeny variant, enforcement_mode on DecisionSummary,
│   │                        #  dry-run branch in evaluate(), enforcement_mode in emit_log()
│   └── mod.rs               # re-export EnforcementMode
├── metrics.rs               # +VerdictLabel::DryRunDeny, pre-create new series
└── controllers/
    └── allocation.rs        # default_allocation_singleton() seeds enforcement_mode: Some(Enforce)

deploy/
└── crds.yaml                # regenerated with enforcementMode in the OpenAPI schema

tests/
├── integration/
│   └── dry_run.rs           # NEW: dry-run integration tests (mocked apiserver)
└── bdd/
      ├── features/
      │   └── dry_run.feature  # NEW: Gherkin scenarios for dry-run
      └── steps/
          └── dry_run_steps.rs # NEW: cucumber-rs step definitions

Cargo.toml                   # +[[test]] entries for dry_run + dry_run_bdd
README.md                    # updated: enforcement modes section + table updates
```

**Structure Decision**: The feature is fully additive within the existing
3-component architecture. No new modules, no new components, no new binaries.
The `EnforcementMode` enum lives in `crd/` alongside the Allocation CRD it
modifies (it is a CRD field type). The decision-pipeline change is in
`webhook/handler.rs` (the `evaluate` function), which is where the budget check
already lives. The metrics change is in `metrics.rs` (one new enum variant).
The controller change is minimal: the auto-created singleton seeds the default.
New test files mirror the existing `tests/integration/` and `tests/bdd/`
patterns.

## Complexity Tracking

No violations to justify. The feature is additive within the existing
architecture and does not introduce complexity beyond the 3-component split.

## Constitution Check (Post-Design)

*Re-check after Phase 1 design artifacts.*

| # | Principle | Status | Post-Design Evidence |
|---|-----------|--------|----------------------|
| I | Fail-Closed by Default | ✅ PASS | The decision state machine (data-model.md §3) confirms: all fail-closed paths return BEFORE `check_budget`. The dry-run conversion is at the `Deny` branch of `check_budget` only. It is structurally impossible for an error path to be converted to an admit. The `resolve_enforcement_mode` helper defaults to `Enforce` for `None`/invalid (data-model.md §1). |
| II | Capacity as a Hard Budget | ✅ PASS | The budget arithmetic (`check_budget`, `ceiling`) is untouched. Dry-run does not alter how capacity is computed — it changes only the verdict for over-budget pods. The operator sees the exact same figures in warnings/logs/metrics as a real rejection would carry. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The Error Path Matrix (contracts/admission-webhook-dry-run.md) enumerates every condition × mode → outcome combination. There is no undefined path. Unknown error types still reject via the catch-all. |
| IV | Observability Before Optimisation | ✅ PASS | The `DryRunDeny` verdict variant and `dry_run_deny` metrics label are first-class from the first decision. The `enforcement_mode` log field is on every entry. No observability is deferred. |
| V | Separated Concerns, Minimal Surface | ✅ PASS | One optional field on one existing CRD. No new component, webhook, or data store. The mode is read from the same cached object as `budgetPercent`. The controller does not use the mode. No new dependencies. |
| VI | Integration Test Coverage | ✅ PASS | Integration tests (quickstart.md Scenario 3/6) cover dry-run admit, dry-run fail-closed, enforce unchanged, and mode switch. BDD feature covers the same. |
| VII | K8s Version Support Window | ✅ PASS | The `warnings` field is GA since K8s 1.19; the support window is 1.34–1.36. No version-compatibility risk. |
| VIII | Test-First Development | ✅ PASS | The implementation will follow Red-Green-Refactor: unit tests for the enum and `evaluate` branch first, watched to fail, then implemented. |
| IX | Editor Configuration as Code | ✅ PASS | New files are `.rs`, `.feature`, `.md` — all covered by existing `.editorconfig` sections. |
| X | README Documentation | ✅ PASS | research.md R12 itemises every README change. The README update is part of the same PR. |
| XI | CI-Green Completion Gate | ✅ PASS | No CI-incompatible change. All jobs must pass before merge. |
| XII | .temp/ Scratch Space | ✅ PASS | No tracked scratch files. |

**Post-design gate result**: ✅ ALL PASS. The design artifacts confirm every
principle is satisfied. No amendments to the constitution are needed.
