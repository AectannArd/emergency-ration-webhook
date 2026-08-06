# Implementation Plan: Per-Resource Budget Tracking (spec-012)

**Branch**: `012-per-resource-budget` | **Date**: 2026-08-06 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/012-per-resource-budget/spec.md`

## Summary

The Allocation CRD currently carries a single required `budgetPercent` field
(0–100) that is applied identically to both CPU and RAM when the Allocation
Controller computes the per-resource ceilings (`ceiling_cpu_milli`,
`ceiling_memory_bytes`). This spec makes CPU and RAM budgets independently
configurable by adding two **optional** spec fields — `cpuBudgetPercent` and
`memoryBudgetPercent` — that override `budgetPercent` for their respective
resource when present, falling back to `budgetPercent` when absent.

The enforcement path is unchanged: `check_budget` in `src/webhook/admission.rs`
already evaluates CPU and RAM ceilings independently and already reports
per-resource violations. The only coupling removed is at the **budget-resolution
layer** — today one `budgetPercent` feeds both ceilings; after this spec each
ceiling is derived from its own effective budget. The controller computes and
the webhook reads the same status fields it reads today (`ceiling_cpu_milli` /
`ceiling_memory_bytes`); the webhook requires no behavioural change.

The change is strictly additive and backward-compatible (FR-005/006): a
singleton with no overrides produces byte-identical ceilings to the pre-feature
controller. The status gains two computed fields
(`effectiveCpuBudgetPercent` / `effectiveMemoryBudgetPercent`) and the structured
admission log gains two fields (`effective_cpu_budget_percent` /
`effective_memory_budget_percent`) so operators can inspect which budget
governed each resource without manual override-vs-fallback resolution. This
unblocks the downstream multi-cluster equalizer (spec-013, stashed), which needs
per-resource limits to equalize CPU and RAM independently.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024) — locked in `Cargo.toml`
(`rust-version = "1.89"`). No change.

**Primary Dependencies**: unchanged from the existing `Cargo.toml`. The feature
reuses the already-pinned `kube = "4.2.0"`, `k8s-openapi = "0.28.0"`,
`schemars = "1"`, `serde`/`serde_json`, `tracing`, `prometheus = "0.14"`. No new
crate, no version bump, no feature-flag change. (The CRD derive macro
`#[derive(CustomResource)]` and `#[schemars(range(min = 0, max = 100))]` are
already in use on `budgetPercent`; the new fields use the identical attribute
surface.)

**Storage**: N/A — state lives in the existing `Allocation` CRD
(`emergency-ration.dev/v1`, cluster-scoped singleton `cluster-allocation`). No
new CRD, no new singleton, no external store.

**Testing**: unchanged frameworks — `#[test]` unit tests for the resolution
function and CRD serialisation; `tower-test` mocked-apiserver integration tests
for the controller/webhook path; `cucumber-rs` BDD for the per-resource
admission scenarios; `erw-verify` (the on-demand real-cluster tool, spec-005)
gains one new enforcement scenario (S9) validating asymmetric per-resource
budgets against a live cluster. E2E on CI uses `kind` across the N-2 K8s matrix.

**Target Platform**: Linux container, Kubernetes workload — unchanged.

**Project Type**: in-tree additive feature on the existing
`capacity-admission-webhook` crate (library + webhook binary + `erw-verify`
binary). No new binary, no workspace split.

**Performance Goals**: unchanged — the resolution function is two `Option::unwrap_or`
calls per admission decision; the ceiling computation reuses the existing 128-bit
`ceiling()` arithmetic with no new intermediate magnitude. p99 admission decision
< 100 ms (constitution provisional target) is unaffected.

**Constraints**: backward compatibility is the hard constraint (FR-005/006,
US2). A singleton with no overrides MUST produce byte-identical ceilings to the
pre-feature controller for the same `budgetPercent` and cluster capacity. The
legacy `budgetPercent` field MUST remain required (it is the fallback for any
resource without an override, and the only budget when neither override is set).

**Scale/Scope**: small additive feature. Touched files: `src/crd/allocation.rs`
(spec + status struct + resolution function + tests), `src/controllers/allocation.rs`
(`build_allocation_status` signature + `recompute` GET + tests),
`src/webhook/handler.rs` (log fields, threaded through `DecisionSummary`),
`src/bin/erw-verify/scenarios/enforcement.rs` (one new scenario S9 + helper
generalisation), `deploy/crds.yaml` (regenerated from `Allocation::crd()`).
~6 source files, ~12–16 tasks (estimated in `/speckit-tasks`).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The fail-closed paths (missing allocation, missing status, stale data, quantity parse failure, timeout, panic) all return BEFORE budget resolution. This feature changes only what happens AFTER those guards — the effective budget number fed to `ceiling()`. No new fail-closed path is needed because no new failure mode is introduced: an absent override is not an error, it is the fallback. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS (strengthened) | The budget remains a deterministic, hard ceiling per resource — now independently tunable. An operator can protect RAM more tightly than CPU without weakening either. The canonical source of capacity truth (node `.status.allocatable` + pod requests) is unchanged. |
| III | Explicit Failure Mode Configuration | ✅ PASS | No new failure mode is introduced. Override-absent is a documented fallback (not an error); override-out-of-range is rejected by the CRD schema (`#[schemars(range(...))]`), the same mechanism that bounds `budgetPercent` today. The resolution function is total: every input maps to exactly one effective budget per resource. |
| IV | Observability Before Optimisation | ✅ PASS | FR-009 (status fields) + FR-010 (log fields) make the effective per-resource budget observable in two places. This is a first-class deliverable, not a polish task — the spec's US3 is dedicated to it. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | No new component. The 3-component architecture (Node Capacity Controller, Allocation Controller, Admission Webhook) is unchanged. The change is confined to the Allocation CRD types + the controller's ceiling computation + the webhook's log/status fields — exactly the seams the existing architecture exposes. No mutating webhook, no new CRD, no caching layer. |
| VI | Integration Test Coverage | ✅ PASS | FR-012 + quickstart.md require integration + BDD coverage for the asymmetric-budget path (CPU admits / memory denies) AND the backward-compat path (no overrides = legacy behaviour). `erw-verify` S9 covers the real-cluster path. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | No new Kubernetes API is used. The `Allocation` CRD stays at `v1` (additive optional fields are backward-compatible per Kubernetes CRD semantics across all supported versions). No conversion webhook. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | tasks.md (Phase 2) will order test-before-implementation per behaviour, per the constitution. The resolution function, the CRD serialisation, and the controller's per-resource ceiling computation are all pure-function testable first. |
| IX | Editor Configuration as Code | ✅ PASS | All new/edited files are existing file types (`.rs`, `.yaml`, `.md`, `.feature`) already covered by `.editorconfig`. No new file type. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | The two new spec fields, two new status fields, and two new log fields are user-facing operator surfaces and MUST be documented in README.md (task in tasks.md). The feature does not add CLI flags or env vars. |
| XI | CI-Green Completion Gate | ✅ PASS | The plan produces no CI changes beyond the CRD manifest regeneration; the existing CI matrix (Rust quality gate + E2E on `kind` + editorconfig) covers the change. Merge requires all green. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | No scratch files needed for this feature; any transient artifacts go to `.temp/`. |
| XIII | Separation of Usage and Contribution Documentation | ✅ PASS | The new spec/status fields are operator-facing → README.md. If `erw-verify` S9 adds a new invocation pattern it goes in CONTRIBUTING.md; if it reuses the existing invocation, no contributor-doc change. |

**Gate result**: PASS. No violations. No entries in the Complexity Tracking table.

## Project Structure

### Documentation (this feature)

```text
specs/012-per-resource-budget/
├── spec.md              # /speckit-specify output (committed)
├── checklists/
│   └── requirements.md  # quality checklist (committed)
├── plan.md              # This file (/speckit-plan output)
├── research.md          # Phase 0 output (/speckit-plan)
├── data-model.md        # Phase 1 output (/speckit-plan)
├── quickstart.md        # Phase 1 output (/speckit-plan)
├── contracts/
│   └── allocation-crd.md # Phase 1 output — extended Allocation CRD contract
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT this phase)
```

### Source Code (repository root)

```text
src/
├── crd/
│   ├── mod.rs                  # unchanged re-exports
│   ├── cluster_capacity.rs     # unchanged
│   └── allocation.rs           # EDITED — +2 spec fields, +2 status fields,
│                               #          +resolve_effective_budgets() fn, +tests
├── controllers/
│   ├── mod.rs                  # unchanged (status_merge_patch helper stays)
│   ├── node_capacity.rs        # unchanged
│   ├── node_filter.rs          # unchanged
│   ├── mock_api.rs             # unchanged
│   └── allocation.rs           # EDITED — build_allocation_status() takes
│                               #          per-resource budgets; recompute() reads
│                               #          overrides from the GET'd spec; +tests
├── webhook/
│   ├── mod.rs                  # unchanged
│   ├── admission.rs            # unchanged (check_budget + ceiling are reused as-is)
│   ├── error.rs                # unchanged
│   └── handler.rs              # EDITED — DecisionSummary gains 2 fields
│                               #          (effective_cpu/memory_budget_percent);
│                               #          decision() threads them; log emits them
├── bin/erw-verify/
│   └── scenarios/
│       └── enforcement.rs      # EDITED — +S9 (asymmetric per-resource budgets);
│                               #          apply_budget() generalised to patch
│                               #          per-resource overrides (+ helper)
├── config.rs                   # unchanged
├── lib.rs                      # unchanged
├── main.rs                     # unchanged
├── metrics.rs                  # unchanged (no new metric — the log + status fields
│                               #   cover US3; a metric is YAGNI per Principle V)
├── resources/mod.rs            # unchanged
├── resources/quantity.rs       # unchanged
└── time_util.rs                # unchanged

deploy/
└── crds.yaml                   # REGENERATED from Allocation::crd() (additive fields)

tests/
├── integration/
│   ├── budget_enforcement.rs   # EDITED — +asymmetric-budget test case (CPU admit / mem deny)
│   ├── capacity_awareness.rs   # EDITED — +effective-budget status assertion
│   ├── fail_safe.rs            # unchanged (fail-closed paths unaffected)
│   ├── dry_run.rs              # EDITED — +per-resource dry-run warning assertion
│   ├── exclusion.rs            # unchanged (exemption path unaffected)
│   ├── node_filter.rs          # unchanged
│   └── performance.rs          # unchanged
├── bdd/
│   ├── budget.feature          # EDITED — +Scenario: per-resource asymmetric budgets
│   └── steps/budget_steps.rs   # EDITED — step for patching per-resource overrides
└── verify/
    ├── report.rs               # unchanged
    └── args.rs                 # unchanged

README.md                       # EDITED — document new spec/status fields
CONTRIBUTING.md                 # EDITED only if erw-verify invocation changes
.editorconfig                   # unchanged (no new file type)
Cargo.toml                      # unchanged (no new dep, no new [[test]] — S9 reuses
                                #   the existing erw-verify binary)
```

**Structure Decision**: this is a minimal additive change to the existing
single-crate structure. No new module directory, no new binary, no workspace
split. The blast radius is 6 source files (4 edited, 1 regenerated manifest) +
4 test files (edited) + 2 docs. This matches SC-004 (single backward-compatible
change, no new components/CLI/RBAC). The choice to reuse `check_budget` and
`ceiling()` unchanged is deliberate (Principle V: minimal surface) — the
per-resource split happens above those functions, not inside them.

## Complexity Tracking

> No constitution violations to justify. Table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none)    | —          | —                                   |

---

## Constitution Check (Post-Design)

*Re-evaluated against the Phase 1 artifacts: `research.md`, `data-model.md`,
`contracts/allocation-crd.md`, `quickstart.md`.*

| # | Principle | Status | Evidence from the artifacts |
|---|-----------|--------|-----------------------------|
| I | Fail-Closed (NON-NEGOTIABLE) | ✅ PASS | `data-model.md` §4.3 + `contracts/allocation-crd.md` §4.2: the fail-closed paths set effective budgets to `-1` and return BEFORE budget resolution. No new failure mode introduced (override-absent = fallback, not error). `quickstart.md` V3.3 tests this. |
| II | Capacity as a Hard Budget | ✅ PASS | `data-model.md` §3.1: ceilings remain deterministic `floor(total * budget / 100)` per resource, now independently tunable. The hard-budget invariant is strengthened, not weakened. |
| III | Explicit Failure Modes | ✅ PASS | `data-model.md` §5: resolution is a total function; out-of-range overrides are schema-rejected. No "undefined" category introduced. |
| IV | Observability | ✅ PASS | `contracts/allocation-crd.md` §2.2/§2.3 (status fields) + §4.2 (log fields) + `quickstart.md` V3.1–V3.3. No metric added (research R9 — YAGNI, derivable from status). |
| V | Separated Concerns (NON-NEGOTIABLE) | ✅ PASS | No new component. `check_budget` and `ceiling()` are reused (ceiling refactored to delegate, preserving arithmetic — `data-model.md` §3.1). The per-resource split is confined to budget resolution + ceiling computation. |
| VI | Integration Test Coverage | ✅ PASS | `quickstart.md` covers US1 (V1.3 integration, V1.4 BDD, V1.5 real-cluster S9), US2 (V2.4 full suite), US3 (V3.2 integration). |
| VII | K8s N-2 | ✅ PASS | CRD stays at `v1`, additive optional fields (contract §5). No new K8s API. |
| VIII | Test-First (NON-NEGOTIABLE) | ✅ PASS | `quickstart.md` enumerates the tests first; tasks.md (Phase 2) orders test-before-implementation. The resolution fn + ceiling helper are pure-function TDD-able. |
| IX | EditorConfig | ✅ PASS | All new/edited files are `.rs`/`.yaml`/`.md`/`.feature` — already covered. |
| X | README Documentation | ✅ PASS | Project Structure lists README.md as edited (new spec/status fields documented). |
| XI | CI-Green Gate | ✅ PASS | No CI workflow change; the existing matrix covers the additive change. |
| XII | Scratch Space | ✅ PASS | No scratch files needed for this feature. |
| XIII | Usage/Contribution Doc Separation | ✅ PASS | New spec/status fields → README; erw-verify S9 invocation → CONTRIBUTING.md (if it changes). |

**Post-design gate result**: PASS. No design artifact introduces a constitution
violation. The design is self-consistent across `data-model.md`,
`contracts/allocation-crd.md`, and `quickstart.md` (cross-doc consistency pass
performed: resolution rule, serialisation-absence-when-None, status field names,
ceiling helper backward-compat equivalence, fail-closed -1 sentinel, and S9
parameters all agree across the three documents).
