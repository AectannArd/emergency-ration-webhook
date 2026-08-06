# Tasks: Per-Resource Budget Tracking (spec-012)

**Input**: Design documents from `/specs/012-per-resource-budget/`
(plan.md, spec.md, research.md, data-model.md, contracts/allocation-crd.md,
quickstart.md)

**Prerequisites**: plan.md, spec.md (required); research.md, data-model.md,
contracts/, quickstart.md (all present).

**Tests**: TDD is NON-NEGOTIABLE (Constitution Principle VIII). Every
implementation task is preceded by its test task (RED → GREEN → REFACTOR). Tests
are written first, watched to fail for the right reason, then implemented.

**Organization**: Tasks are grouped by user story. The contract
(`contracts/allocation-crd.md`) is the authoritative source the implementation
must satisfy; where data-model.md or quickstart.md describe behavior, they agree
with the contract.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: Which user story this task belongs to (US1/US2/US3)
- All paths are repository-relative (single-crate project; `src/` at root)

## Path Conventions

Single project: `src/`, `tests/`, `deploy/` at repository root. Binary targets
live under `src/bin/erw-verify/`. See `plan.md` §Project Structure for the full
concrete tree.

---

## Phase 1: Setup

**Purpose**: No new project structure is created by this feature (it is additive
to the existing crate). The only "setup" is confirming the working tree is on the
implementation branch and the foundation compiles before changes begin.

- [ ] T001 Create implementation branch `spec/012-per-resource-budget` off `main` and confirm `cargo build` + `cargo test` pass on the unmodified tree (baseline green — Constitution Principle XI). Commit nothing yet.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The CRD type additions and pure helper functions that ALL user
stories depend on. These are pure (no I/O, no async) and fully unit-testable in
isolation (Constitution Principle VIII). No user-story work can begin until this
phase is complete and green.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Tests for Foundational (write FIRST, watch RED)

- [ ] T002 [P] Unit test: `resolve_effective_budgets` resolution truth table in `src/crd/allocation.rs` (tests module). Assert every row of `data-model.md` §2 truth table: `(80,None,None)→(80,80)`, `(80,Some(90),None)→(90,80)`, `(80,None,Some(60))→(80,60)`, `(80,Some(90),Some(60))→(90,60)`, `(80,Some(80),Some(80))→(80,80)`, `(70,Some(90),None)→(90,70)`, `(0,None,None)→(0,0)`, `(100,Some(0),Some(100))→(0,100)`. Watch fail (function absent).
- [ ] T003 [P] Unit test: `ceiling_per_resource` computes each figure with its own budget in `src/webhook/admission.rs` (tests module). Assert `ceiling_per_resource((100_000, 200*Gi), (90, 60)) == (90_000, floor(200Gi*60/100))`. Watch fail (function absent).
- [ ] T004 [P] Unit test: backward-compat equivalence `ceiling_per_resource((t,t),(p,p)) == ceiling((t,t),p)` for several `(t, p)` in `src/webhook/admission.rs` (tests module). Watch fail (`ceiling_per_resource` absent).
- [ ] T005 [P] Unit test: new spec fields serialise camelCase `cpuBudgetPercent`/`memoryBudgetPercent`, round-trip, and are absent from JSON when `None` (US2 AC2) in `src/crd/allocation.rs` (tests module). Watch fail (fields absent → won't serialise).
- [ ] T006 [P] Unit test: CRD schema declares new fields optional (NOT in `spec.required`) with `minimum:0, maximum:100`, and `budgetPercent` stays required (FR-006) in `src/crd/allocation.rs` (tests module). Watch fail (fields absent from schema).
- [ ] T007 [P] Unit test: new status fields `effectiveCpuBudgetPercent`/`effectiveMemoryBudgetPercent` serialise camelCase and round-trip in `src/crd/allocation.rs` (tests module). Watch fail (fields absent).

### Implementation for Foundational (GREEN)

- [ ] T008 [US1] Add `cpu_budget_percent: Option<i32>` and `memory_budget_percent: Option<i32>` fields to `AllocationSpec` in `src/crd/allocation.rs`, each with `#[schemars(range(min = 0, max = 100))]`. Mirrors the existing `budget_percent` field attribute (data-model.md §1.1, contract §1.2/§1.3). Make T005, T006 pass.
- [ ] T009 [P] [US1] Add `effective_cpu_budget_percent: i32` and `effective_memory_budget_percent: i32` to `AllocationStatus` in `src/crd/allocation.rs` (data-model.md §1.1, contract §2.2/§2.3). Make T007 pass.
- [ ] T010 [US1] Implement `pub fn resolve_effective_budgets(spec: &AllocationSpec) -> (i32, i32)` in `src/crd/allocation.rs` per data-model.md §2 (each resource: override if `Some`, else `budget_percent`). Make T002 pass.
- [ ] T011 [US1] Implement `pub fn ceiling_single(total: i64, budget_percent: i32) -> i64` and `pub fn ceiling_per_resource(total: Figures, budgets: (i32, i32)) -> Figures` in `src/webhook/admission.rs`, extracting the 128-bit-guarded arithmetic from the current `ceiling()` body (data-model.md §3.1). Refactor the existing `ceiling()` to delegate: `ceiling_per_resource(total, (p, p))`. Verify existing `ceiling` callers compile unchanged. Make T003, T004 pass.
- [ ] T012 [US1] Update `default_allocation_singleton()` in `src/controllers/allocation.rs` to seed `cpu_budget_percent: None, memory_budget_percent: None` (FR-008 — fresh cluster boots in legacy mode). Update the existing `AllocationSpec` struct literals in the test helpers (`fn sample_spec` etc.) to include the two new `None` fields so the crate compiles.

**Checkpoint**: Foundation ready — `cargo test --lib` green; `resolve_effective_budgets` and the ceiling helpers proven pure. The controller recompute loop and webhook do NOT yet use these; user-story work can now begin.

---

## Phase 3: User Story 1 — Asymmetric Budgets Per Resource (Priority: P1) 🎯 MVP

**Goal**: The controller computes per-resource ceilings from the resolved
budgets; the webhook enforces them unchanged. An operator can set
`cpuBudgetPercent: 90` + `memoryBudgetPercent: 60` and get independent
enforcement (SC-001).

**Independent Test**: set asymmetric overrides, submit a pod that fits on CPU
but exceeds memory → denied on memory only (US1 AC1). Swap overrides, symmetric
pod → denied on CPU only (US1 AC2).

**Note**: the webhook's `check_budget` and the enforcement decision path require
**NO change** — they already evaluate CPU/RAM ceilings independently against the
status ceilings (contract §4.1). The only wiring is in the controller.

### Tests for User Story 1 (write FIRST, watch RED)

- [ ] T013 [US1] Unit test: `build_allocation_status` with per-resource budgets `(90, 60)` and supply `(100_000, 200*Gi)` produces `ceiling_cpu_milli == 90_000`, `ceiling_memory_bytes == floor(200Gi*60/100)`, AND `effective_cpu_budget_percent == 90`, `effective_memory_budget_percent == 60` (FR-003, FR-009) in `src/controllers/allocation.rs` (tests module). Watch fail (signature still takes single `budget_percent`).
- [ ] T014 [P] [US1] Integration test: asymmetric budgets — CPU admits, memory denies — in `tests/integration/budget_enforcement.rs`. Allocation singleton with `cpuBudgetPercent: 95, memoryBudgetPercent: 30`; a pod with CPU request under the 95% ceiling + memory request over the 30% ceiling asserts `Deny` with exactly one violation, `resource: Memory` (FR-011, US1 AC1). Swapped overrides + symmetric pod → `Deny` with `resource: Cpu` only (US1 AC2). Watch fail (controller still computes a single ceiling).
- [ ] T015 [P] [US1] BDD scenario in `tests/bdd/budget.feature`: "Per-resource asymmetric budgets — CPU admits, memory denies" per quickstart.md V1.4. Add the matching step that patches per-resource overrides in `tests/bdd/steps/budget_steps.rs`. Watch fail.

### Implementation for User Story 1 (GREEN)

- [ ] T016 [US1] Change `build_allocation_status` signature in `src/controllers/allocation.rs` from `budget_percent: i32` to `budgets: (i32, i32)`; compute ceilings via `ceiling_per_resource(total_supply, budgets)`; populate `effective_cpu_budget_percent: budgets.0` and `effective_memory_budget_percent: budgets.1` in the returned status (data-model.md §3.2, contract §3.2). Make T013 pass.
- [ ] T017 [US1] Update `recompute()` in `src/controllers/allocation.rs`: replace `let budget = allocation.spec.budget_percent` with `let budgets = resolve_effective_budgets(&allocation.spec)` (import `resolve_effective_budgets` from `crate::crd`); pass `budgets` to `build_allocation_status` (data-model.md §3.3). Make T014, T015 pass.
- [ ] T018 [US1] REFACTOR: remove the now-unused `use crate::webhook::admission::ceiling;` import in `src/controllers/allocation.rs` if no other reference remains; run `cargo clippy -- -D warnings` and `cargo fmt --check` (Constitution quality gate).

**Checkpoint**: US1 fully functional and independently testable. `cargo test --test budget_enforcement` + `cargo test --test budget_bdd` green. An operator can now set asymmetric per-resource budgets.

---

## Phase 4: User Story 2 — Backward Compatibility (Priority: P2)

**Goal**: An existing cluster upgraded with no spec change exhibits zero
behavioural drift (SC-002). The release gate — no regression on single-budget
deployments.

**Independent Test**: `cargo test` (full existing suite) passes unchanged on a
no-override singleton (US2 AC3); ceilings byte-identical without overrides
(US2 AC1).

**Note**: most of US2 is already verified by T004 (ceiling backward-compat
equivalence) and the full existing suite. This phase adds the explicit
FR-005 proof and confirms no-override serialisation.

### Tests for User Story 2 (write FIRST, watch RED)

- [ ] T019 [P] [US2] Unit test: no-override ceilings byte-identical to legacy (FR-005, research R10) in `src/controllers/allocation.rs` (tests module). For `(budget_percent: 80, cpu: None, memory: None)` and supply `(100_000, 200*Gi)`, assert `build_allocation_status(..., (80, 80))` yields `ceiling_cpu_milli`/`ceiling_memory_bytes` equal to `floor(supply*80/100)` — the exact pre-spec-012 values. Repeat for `budget_percent` ∈ {0, 50, 80, 100}. Watch fail (would already pass after T016, but write it FIRST per TDD; if it passes immediately, the function is correct — still required as the explicit gate).
- [ ] T020 [P] [US2] Unit test: `default_allocation_singleton()` seeds both overrides as `None` (FR-008) in `src/controllers/allocation.rs` (tests module). Assert the auto-created singleton's spec has `cpu_budget_percent.is_none()` and `memory_budget_percent.is_none()`. Watch fail (would pass after T012; still required as the explicit gate).

### Implementation for User Story 2 (VERIFICATION-ONLY — likely no new code)

- [ ] T021 [US2] Run `cargo test` (full suite: unit + integration + BDD). Confirm every pre-existing test passes unchanged against a no-override singleton (US2 AC3, SC-002). This is verification — if any pre-existing test fails, the foundational or US1 change introduced a regression and MUST be fixed (not the test). Commit baseline-green evidence.

**Checkpoint**: US2 certified. Backward compatibility is proven by T004, T019,
T020, and the full existing suite (T021). A pre-feature singleton behaves
identically.

---

## Phase 5: User Story 3 — Observability of Effective Per-Resource Budget (Priority: P3)

**Goal**: The effective per-resource budget is observable in the Allocation
status (already written by the controller via T009/T016) AND in the structured
admission log (FR-010) (SC-003).

**Independent Test**: a decision's structured log carries
`effective_cpu_budget_percent` and `effective_memory_budget_percent` matching the
status values (US3 AC1); `kubectl get allocations -o yaml` shows them (US3 AC2).

**Note**: the status fields are already populated (Phase 2/3). This phase adds
the webhook log fields (FR-010) — the only new code in US3.

### Tests for User Story 3 (write FIRST, watch RED)

- [ ] T022 [P] [US3] Unit test: `build_allocation_status` exposes effective budgets in status (FR-009) — covers `(90, 60)→(90,60)` and legacy `(80,80)→(80,80)` — in `src/controllers/allocation.rs` (tests module). Watch fail (may already pass after T016; write first per TDD).
- [ ] T023 [P] [US3] Integration test: structured log / `DecisionSummary` carries `effective_cpu_budget_percent` + `effective_memory_budget_percent` matching status on admit/deny/dry-run-deny (FR-010) in `tests/integration/budget_enforcement.rs`. Use the existing summary-capture harness. Watch fail (fields absent from `DecisionSummary`).
- [ ] T024 [P] [US3] Integration test: fail-closed paths (missing allocation, stale data) set both effective fields to `-1` in the summary (FR-010 exempt paths) in `tests/integration/fail_safe.rs`. Watch fail (fields absent from `DecisionSummary`).

### Implementation for User Story 3 (GREEN)

- [ ] T025 [US3] Add `pub effective_cpu_budget_percent: i64` and `pub effective_memory_budget_percent: i64` to `DecisionSummary` in `src/webhook/handler.rs` (data-model.md §4.1, contract §4.2). Update the `decision()` builder to accept the per-resource pair and populate them from the Allocation **status** (`status.effective_cpu_budget_percent`, `status.effective_memory_budget_percent`) — do NOT re-resolve in the webhook (research R5). Update `reject_outcome` and `exempt` to set both to `-1` (the existing no-budget sentinel, handler.rs:635). Make T023, T024 pass.
- [ ] T026 [US3] Thread the effective per-resource budgets from `allocation.status` through the `decide()` function in `src/webhook/handler.rs`: read them alongside the existing `budget_percent` and pass to `DecisionSummary::decision(...)`. Emit them in the structured log (`tracing::info!`/`debug!` at the decision point). Make T023 pass end-to-end.
- [ ] T027 [US3] Update all existing `DecisionSummary::decision(...)` and summary-builder call sites in `src/webhook/handler.rs` tests to pass the new pair (mechanical: existing tests pass `(budget_percent, budget_percent)` or read from a status fixture). Confirm `cargo test --lib` green.

**Checkpoint**: US3 complete. Effective per-resource budgets observable in
status (controller) and structured log (webhook). `cargo test` fully green.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Real-cluster verification scenario, manifest regeneration,
operator documentation, and the final quality gate.

- [ ] T028 [US1] Write test for `erw-verify` S9 first: add `async fn s9(client)` to `src/bin/erw-verify/scenarios/enforcement.rs` (per contract §6: patch `cpuBudgetPercent:95, memoryBudgetPercent:30`, wait for recompute, create a pod denied on memory, assert "memory" in the denial message, restore overrides to `null`). Register it in the `run()` vector as `timed("S9", "per-resource asymmetric budgets", s9(client))`. Generalise `apply_budget` into `apply_per_resource_budgets` (or add a sibling) that patches the override fields and waits for the memory ceiling to settle. Note: S9 runs only against a real cluster (`#[ignore]`-equivalent — `erw-verify` is opt-in); it is NOT part of `cargo test`.
- [ ] T029 Regenerate `deploy/crds.yaml` from `Allocation::crd()` so the manifest carries the new optional spec fields (`cpuBudgetPercent`/`memoryBudgetPercent`, `minimum:0, maximum:100`, NOT in `required`) and new status fields (`effectiveCpuBudgetPercent`/`effectiveMemoryBudgetPercent`). Verify with `kubectl apply --dry-run=server` semantics (or a `kind` apply in CI) that the updated CRD is accepted. Contract §5.
- [ ] T030 [P] Document the new spec fields (`cpuBudgetPercent`/`memoryBudgetPercent`), new status fields (`effectiveCpuBudgetPercent`/`effectiveMemoryBudgetPercent`), and new structured-log fields (`effective_cpu_budget_percent`/`effective_memory_budget_percent`) in `README.md` (Constitution Principle X — user-facing operator surface). Include a `kubectl patch` example for setting asymmetric overrides.
- [ ] T031 [P] Update `CONTRIBUTING.md` ONLY if the `erw-verify` invocation changed (Constitution Principle XIII). If S9 reuses the existing invocation (`erw-verify --kubeconfig <path>`), no change is needed — note this in the commit message.
- [ ] T032 Add per-resource dry-run warning test: in `tests/integration/dry_run.rs`, assert that an asymmetric-budget memory-only violation in dry-run mode produces a memory-only warning (edge case). Watch fail first if the warning construction depends on the new fields; otherwise verification-only.
- [ ] T033 Run the full quality gate (Constitution Principles VIII, IX, XI): `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (unit + integration + BDD). ALL must be green. Fix any failure at its root, not by weakening a test.
- [ ] T034 Run `quickstart.md` validation: execute the V1.1–V3.3 commands from `specs/012-per-resource-budget/quickstart.md` and confirm all pass. This is the final feature-readiness check (SC-001 through SC-004).

**Checkpoint**: Feature complete and documented. Ready for PR.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies — branch off `main`, confirm baseline green.
- **Foundational (Phase 2)**: depends on Setup — BLOCKS all user stories.
- **US1 (Phase 3)**: depends on Foundational. The controller wiring is the core.
- **US2 (Phase 4)**: depends on Foundational + US1 (the controller change in T016 is what US2 certifies backward-compatible). Mostly verification.
- **US3 (Phase 5)**: depends on Foundational + US1 (reads the status fields US1's controller change writes). Adds the webhook log fields.
- **Polish (Phase 6)**: depends on all user stories. S9, manifest, docs, final gate.

### User Story Dependencies

- **US1 (P1)**: depends on Foundational only. Independent MVP slice.
- **US2 (P2)**: depends on Foundational + US1 (certifies US1 didn't regress legacy). Independently testable via the no-override path.
- **US3 (P3)**: depends on Foundational + US1 (consumes the status fields US1 writes). Independently testable via the log/status assertions.

### Within Each User Story

- Tests written FIRST and watched RED (Constitution Principle VIII).
- Types/helpers before wiring; wiring before integration tests pass.
- `cargo clippy` + `cargo fmt --check` after each story (T018 pattern).

### Parallel Opportunities

- T002–T007 (foundational tests, all different concerns, `[P]`) can be written together.
- T003/T004 (ceiling helper tests) and T002 (resolution test) are independent.
- T014/T015 (US1 integration + BDD) are `[P]` (different files).
- T019/T020 (US2 tests) are `[P]`.
- T022/T023/T024 (US3 tests) are `[P]` (different files/modules).
- T030/T031 (docs) are `[P]`.

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1).
2. **STOP and VALIDATE**: asymmetric per-resource budgets enforced; `cargo test --test budget_enforcement` + `cargo test --test budget_bdd` green.
3. This is a deployable MVP — operators can set `cpuBudgetPercent`/`memoryBudgetPercent`.

### Incremental Delivery

1. Foundational types + helpers (pure, unit-tested).
2. US1 → controller computes per-resource ceilings → MVP.
3. US2 → certify backward compat → safe to upgrade existing clusters.
4. US3 → effective budgets observable in log + status → debuggable.
5. Polish → real-cluster S9, manifest, docs, final gate → PR-ready.

---

## Notes

- The webhook's `check_budget` and the enforcement decision logic are UNCHANGED
  (contract §4.1). Do not modify `src/webhook/admission.rs::check_budget` —
  tasks T011 (ceiling helpers) and T016 (controller) are the only enforcement-
  adjacent changes; `check_budget` already evaluates CPU/RAM independently.
- `budgetPercent` MUST remain required (FR-006) — never make it `Option` or add
  `#[serde(default)]`. It is the fallback for any resource without an override.
- Overrides serialise ABSENT (not `null`) when `None` (US2 AC2) — rely on
  `Option<i32>` serde semantics; do not add `skip_serializing_if` hacks.
- The agent reads `spec.budget_percent` in `recompute`; after T017 it reads the
  whole spec and calls `resolve_effective_budgets`. Do not add a third cache.
- No new Prometheus metric (research R9 — YAGNI). Status + log fields cover US3.
- The `.dockerignore` at the repo root is an untracked leftover from a prior
  real-cluster test session; do NOT commit it as part of this spec's work.
