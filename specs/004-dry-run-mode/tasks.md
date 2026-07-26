---

description: "Task list for dry-run enforcement mode implementation"
---

# Tasks: Dry-Run Enforcement Mode

**Input**: Design documents from `/specs/004-dry-run-mode/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md,
contracts/admission-webhook-dry-run.md

**Tests**: The constitution (Principle VIII) mandates TDD — tests are written
FIRST, watched to fail, then implemented. Every task below follows
Red-Green_Refactor.

**Organization**: Tasks are grouped by user story to enable independent
implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- Single project: `src/`, `tests/` at repository root (existing layout)
- Deploy manifests: `deploy/`

---

## Phase 1: Foundational (Blocking Prerequisites)

**Purpose**: The `EnforcementMode` type, CRD schema change, and safe-default
resolution helper. Everything else depends on this.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Tests

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation.**

- [ ] T001 [P] Test `EnforcementMode` enum serialises as `"enforce"` and
  `"dry-run"` (kebab-case) and deserialises both values — add to `src/crd/allocation.rs`
  `#[cfg(test)]` module
- [ ] T002 [P] Test `resolve_enforcement_mode(None)` returns `Enforce` and
  `resolve_enforcement_mode(Some(DryRun))` returns `DryRun` — add to
  `src/crd/allocation.rs` `#[cfg(test)]` module
- [ ] T003 [P] Test the Allocation CRD OpenAPI schema includes `enforcementMode`
  as an optional string enum field (not in `required`) — add to
  `src/crd/allocation.rs` `#[cfg(test)]` module

### Implementation

- [ ] T004 Define `EnforcementMode` enum (`Enforce`, `DryRun`) with
  `serde(rename_all = "kebab-case")`, `JsonSchema` derive in
  `src/crd/allocation.rs`
- [ ] T005 Add `pub enforcement_mode: Option<EnforcementMode>` field to
  `AllocationSpec` in `src/crd/allocation.rs`
- [ ] T006 [P] Define `resolve_enforcement_mode(mode: Option<EnforcementMode>)
  -> EnforcementMode` helper in `src/crd/allocation.rs` (defaults `None` to
  `Enforce`)
- [ ] T007 [P] Re-export `EnforcementMode` and `resolve_enforcement_mode` from
  `src/crd/mod.rs` and `src/lib.rs`
- [ ] T008 Update `default_allocation_singleton()` in
  `src/controllers/allocation.rs` to seed `enforcement_mode:
  Some(EnforcementMode::Enforce)`
- [ ] T009 [P] Regenerate `deploy/crds.yaml` to include the `enforcementMode`
  field in the Allocation CRD OpenAPI schema (run the CRD generation and
  diff/reconcile the output)

**Checkpoint**: The `EnforcementMode` type exists, serialises correctly, the CRD
schema includes the new optional field, and the auto-created singleton defaults
to `enforce`. The webhook does not yet read or use the field.

---

## Phase 2: User Story 1 — Shadow Evaluation (Priority: P1) 🎯 MVP

**Goal**: In dry-run mode, over-budget pods are admitted with a warning instead
of rejected. In enforce mode, behaviour is unchanged.

**Independent Test**: Submit an over-budget pod in dry-run mode and observe it
admitted with a warning; submit the same pod in enforce mode and observe it
rejected.

### Tests

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation.**

- [ ] T010 [P] [US1] Unit test: `evaluate()` in dry-run mode converts an
  over-budget deny to an admit with `response.allowed == true` and
  `response.warnings` populated with the budget-violation message — add to
  `src/webhook/handler.rs` `#[cfg(test)]` module
- [ ] T011 [P] [US1] Unit test: `evaluate()` in enforce mode rejects an
  over-budget pod (unchanged behaviour) — add to `src/webhook/handler.rs`
  `#[cfg(test)]` module
- [ ] T012 [P] [US1] Unit test: `evaluate()` in dry-run mode admits a
  within-budget pod normally (no warning, `decision == allow`) — add to
  `src/webhook/handler.rs` `#[cfg(test)]` module

### Implementation

- [ ] T013 [US1] Add `enforcement_mode: String` field to `DecisionSummary` in
  `src/webhook/handler.rs`, populated from `resolve_enforcement_mode(...)`
- [ ] T014 [US1] Add `DryRunDeny` variant to `DecisionVerdict` enum in
  `src/webhook/handler.rs`
- [ ] T015 [US1] In `evaluate()` at the `AdmissionVerdict::Deny(violations)`
  branch in `src/webhook/handler.rs`: read `enforcement_mode` from the
  Allocation spec, and if `DryRun`, produce an admit outcome
  (`response.allowed = true`, `response.warnings = Some(vec![...])`,
  `summary.verdict = DryRunDeny`) instead of a deny outcome. If `Enforce`, keep
  existing behaviour unchanged
- [ ] T016 [US1] Thread `enforcement_mode` through all `DecisionSummary`
  constructors (`decision()`, `reject_outcome()`) in
  `src/webhook/handler.rs` so every decision carries the active mode
- [ ] T017 [US1] Update `DecisionSummary::decision()` to accept the resolved
  `EnforcementMode` and store it, defaulting reject outcomes to the mode read
  from the allocation spec (or `"enforce"` when no allocation is available)

**Checkpoint**: Dry-run mode admits over-budget pods with warnings; enforce mode
is unchanged. The decision summary carries the enforcement mode on every
decision.

---

## Phase 3: User Story 2 — Fail-Closed Integrity in Dry-Run (Priority: P2)

**Goal**: Fail-closed paths (capacity data missing/stale, malformed request,
timeout, panic) reject in both dry-run and enforce modes.

**Independent Test**: Put the webhook in dry-run mode, make capacity data stale,
submit a pod, and observe it rejected.

### Tests

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation.**

- [ ] T018 [P] [US2] Unit test: `evaluate()` in dry-run mode with stale capacity
  data rejects (`allowed == false`, `verdict == Error`, reason
  `capacity_data_stale`) — add to `src/webhook/handler.rs` `#[cfg(test)]`
  module
- [ ] T019 [P] [US2] Unit test: `evaluate()` in dry-run mode with missing
  Allocation singleton rejects (`allowed == false`, reason
  `capacity_data_missing`) — add to `src/webhook/handler.rs` `#[cfg(test)]`
  module
- [ ] T020 [P] [US2] Unit test: `evaluate()` in dry-run mode with missing
  ClusterCapacity rejects — add to `src/webhook/handler.rs` `#[cfg(test)]`
  module

### Implementation

This phase has NO implementation tasks — the fail-closed paths already return
BEFORE `check_budget` is reached (see data-model.md §3 state machine). The tests
verify the existing architectural guarantee holds under dry-run mode. If any
test fails, it indicates a regression in the insertion point and must be fixed
by ensuring the enforcement-mode branch is only reachable from the
`check_budget` Deny branch.

- [ ] T021 [US2] Verify (no code change expected): all T018–T020 tests pass
  against the Phase 2 implementation unchanged. If any fail, the dry-run
  insertion point is wrong — fix the branch in `evaluate()` so fail-closed
  paths are structurally unreachable by the mode toggle

**Checkpoint**: All fail-closed paths reject regardless of enforcement mode.
The structural guarantee for FR-006 is verified.

---

## Phase 4: User Story 3 — Dry-Run Observability (Priority: P3)

**Goal**: Dry-run decisions are distinguishable from enforced denies and allows
in both structured logs and metrics.

**Independent Test**: Submit an over-budget pod in dry-run mode and confirm the
log entry carries `decision=dry_run_deny` and `enforcement_mode=dry_run`; scrape
metrics and confirm a `dry_run_deny` verdict series exists.

### Tests

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation.**

- [ ] T022 [P] [US3] Unit test: `VerdictLabel::DryRunDeny` serialises as
  `"dry_run_deny"` — add to `src/metrics.rs` `#[cfg(test)]` module
- [ ] T023 [P] [US3] Unit test: `Metrics::new()` pre-creates the
  `{resource="cpu",verdict="dry_run_deny"}` and
  `{resource="memory",verdict="dry_run_deny"}` series at zero — add to
  `src/metrics.rs` `#[cfg(test)]` module
- [ ] T024 [P] [US3] Unit test: `record_metrics()` maps `DryRunDeny` decision
  verdict to `VerdictLabel::DryRunDeny` — add to `src/webhook/handler.rs`
  `#[cfg(test)]` module

### Implementation

- [ ] T025 [P] [US3] Add `DryRunDeny` variant (serialised `"dry_run_deny"`) to
  `VerdictLabel` enum in `src/metrics.rs`
- [ ] T026 [US3] Add `DryRunDeny` to the pre-creation loop in `Metrics::new()`
  in `src/metrics.rs` so the new series appear at zero from startup
- [ ] T027 [US3] Update `record_metrics()` in `src/webhook/handler.rs` to map
  `DecisionVerdict::DryRunDeny` to `VerdictLabel::DryRunDeny`
- [ ] T028 [US3] Update `emit_log()` in `src/webhook/handler.rs` to handle the
  `DryRunDeny` verdict: log at WARN with `decision = "dry_run_deny"`, the
  violated resource reason, all capacity figures, and the `enforcement_mode`
  field. Add the `enforcement_mode` field to ALL log variants (allow, deny,
  dry_run_deny, error) per FR-009

**Checkpoint**: Dry-run decisions are a first-class signal in logs and metrics.
Every log entry carries `enforcement_mode`.

---

## Phase 5: Integration & BDD Tests

**Purpose**: End-to-end coverage through the real admission path (mocked
apiserver) and readable Gherkin scenarios.

### Integration Tests

- [ ] T029 [P] Create `tests/integration/dry_run.rs` with a tower-test mocked
  apiserver: dry-run mode admits an over-budget pod with warnings and
  `allowed: true`
- [ ] T030 [P] Create integration test in `tests/integration/dry_run.rs`:
  dry-run mode with stale capacity data still rejects (fail-closed integrity)
- [ ] T031 [P] Create integration test in `tests/integration/dry_run.rs`:
  enforce mode rejects an over-budget pod (no behaviour change)
- [ ] T032 [P] Create integration test in `tests/integration/dry_run.rs`:
  mode switch from dry-run to enforce takes effect on the next decision
  (patch the Allocation spec, verify subsequent decision rejects)
- [ ] T033 Add `[[test]]` entries for `dry_run` and `dry_run_bdd` in
  `Cargo.toml`

### BDD Tests

- [ ] T034 [P] Create `tests/bdd/features/dry_run.feature` with Gherkin
  scenarios: dry-run admits over-budget pod with warning; dry-run rejects on
  stale capacity data; enforce mode rejects over-budget pod
- [ ] T035 [P] Create `tests/bdd/steps/dry_run_steps.rs` with cucumber-rs step
  definitions for the dry-run feature file
- [ ] T036 [P] Create `tests/bdd/dry_run_steps.rs` harness entry point (matches
  the existing `budget_steps.rs` pattern)

**Checkpoint**: Full integration and BDD coverage for all three user stories.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, deploy manifest reconciliation, and final quality
gate.

- [ ] T037 [P] Update `README.md`: add "Enforcement Modes (Enforce / Dry-Run)"
  section under Configuration documenting `spec.enforcementMode`, the two
  values, the default, and the `kubectl patch` toggle command
- [ ] T038 [P] Update `README.md`: add `enforcementMode` to the Allocation CRD
  spec table
- [ ] T039 [P] Update `README.md`: update the Failure Modes table to note that
  fail-closed paths reject in both modes
- [ ] T040 [P] Update `README.md`: add `dry_run_deny` to the Prometheus Metrics
  verdict label values table and the Structured Logging `decision` field values
- [ ] T041 [P] Update `README.md`: add `enforcement_mode` to the Structured
  Logging fields table
- [ ] T042 Run the full quality gate and fix until green:
  `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- [ ] T043 Run the quickstart.md validation scenarios and confirm all pass
  against the implemented code

---

## Dependencies & Execution Order

### Phase Dependencies

- **Foundational (Phase 1)**: No dependencies — can start immediately. BLOCKS
  all user stories.
- **US1 — Shadow Evaluation (Phase 2)**: Depends on Phase 1. This is the MVP.
- **US2 — Fail-Closed Integrity (Phase 3)**: Depends on Phase 2 (the dry-run
  branch must exist before we can verify fail-closed paths are unaffected).
- **US3 — Dry-Run Observability (Phase 4)**: Depends on Phase 2 (the `DryRunDeny`
  verdict variant must exist before metrics/logging can reference it).
- **Integration & BDD (Phase 5)**: Depends on Phases 2–4.
- **Polish (Phase 6)**: Depends on all prior phases.

### Within Each User Story

- Tests (if included) MUST be written and FAIL before implementation
- Types/enums before logic
- Logic before logging/metrics
- Story complete before moving to next priority

### Parallel Opportunities

- T001–T003 (foundational tests) can run in parallel — different test functions
  in the same module
- T010–T012 (US1 tests) can run in parallel — different test functions
- T018–T020 (US2 tests) can run in parallel — different test functions
- T022–T024 (US3 tests) can run in parallel — different test functions
- T029–T032 (integration tests) can run in parallel — different test functions
  in the same file
- T037–T041 (README updates) can run in parallel — different sections of the
  same file

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Foundational (CRD schema + type + resolution helper)
2. Complete Phase 2: User Story 1 (dry-run admit branch)
3. **STOP and VALIDATE**: Test the dry-run admit path independently
4. This is a working MVP — over-budget pods are admitted with warnings in
   dry-run mode

### Incremental Delivery

1. Foundational → type exists, CRD schema updated, singleton defaults to enforce
2. US1 → dry-run mode admits over-budget pods with warnings (MVP)
3. US2 → verify fail-closed paths are unaffected (verification, no new code)
4. US3 → dry-run decisions are distinguishable in logs and metrics
5. Integration & BDD → end-to-end coverage
6. Polish → README, quality gate, quickstart validation

---

## Notes

- [P] tasks = different files or different test functions, no dependencies
- [Story] label maps task to specific user story for traceability
- The feature is additive — no existing behaviour changes in enforce mode
- The fail-closed paths need NO code changes (US2 is verification-only) — the
  architectural guarantee is that error paths return before `check_budget`
- Commit after each task or logical group
- Stop at any checkpoint to validate independently
