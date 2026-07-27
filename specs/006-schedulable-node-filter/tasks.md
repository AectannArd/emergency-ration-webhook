# Tasks: Schedulable Node Filter

**Input**: Design documents from `/specs/006-schedulable-node-filter/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/clustercapacity-crd.md

**Tests**: This project uses strict TDD (Constitution Principle VIII). Tests are written FIRST, watched to fail, then implemented to pass. Every implementation task is preceded by its test task.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- Rust project: `src/` at repository root, `tests/` with subdirectories
- CRD definitions: `src/crd/`
- Controller logic: `src/controllers/`
- Deploy manifests: `deploy/`
- Integration tests: `tests/integration/`
- BDD tests: `tests/bdd/features/` + `tests/bdd/steps/`

## Phase 1: Foundational (Blocking Prerequisites)

**Purpose**: CRD struct changes that all user stories depend on. These are the data-layer changes that the filter logic, status observability, and controller modifications all build on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Tests for Foundational CRD Changes (TDD — write first, watch fail)

- [ ] T001 [P] Write test: `ClusterCapacitySpec` with `node_selector: Option<LabelSelector>` field serialises camelCase as `nodeSelector` and round-trips through serde in `src/crd/cluster_capacity.rs` #[cfg(test)] module. Verify the test FAILS (field does not exist yet).
- [ ] T002 [P] Write test: `ClusterCapacityStatus` with new fields `excluded_node_count`, `excluded_by_unschedulable`, `excluded_by_selector` serialise camelCase (`excludedNodeCount`, `excludedByUnschedulable`, `excludedBySelector`) and round-trip in `src/crd/cluster_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T003 [P] Write test: `ClusterCapacity::crd()` generated schema includes `nodeSelector` under `spec.properties` (using `serde_json::pointer` to traverse the CRD JSON) in `src/crd/cluster_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T004 [P] Write test: `default_capacity_singleton()` creates a `ClusterCapacitySpec { node_selector: None }` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL (struct literal mismatch — field doesn't exist yet).

### Implementation for Foundational CRD Changes

- [ ] T005 Add `pub node_selector: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector>` field to `ClusterCapacitySpec` in `src/crd/cluster_capacity.rs`. Add `use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;` import. This makes T001, T003, T004 pass (GREEN).
- [ ] T006 Add `pub excluded_node_count: i32`, `pub excluded_by_unschedulable: i32`, `pub excluded_by_selector: i32` fields to `ClusterCapacityStatus` in `src/crd/cluster_capacity.rs`. This makes T002 pass (GREEN).
- [ ] T007 [P] Update `default_capacity_singleton()` in `src/controllers/node_capacity.rs` to construct `ClusterCapacitySpec { node_selector: None }`. This makes T004 pass (GREEN).
- [ ] T008 [P] Update existing test `status_serialises_camel_case()` in `src/crd/cluster_capacity.rs` to include the three new fields in the constructed status literal (mechanical update — the test currently constructs a status without these fields).
- [ ] T009 Run `cargo test --lib crd::cluster_capacity` — all CRD tests pass (GREEN).

**Checkpoint**: Foundation ready — CRD structs carry the new fields, all existing tests pass, user story implementation can begin.

---

## Phase 2: User Story 1 — Cordoned Nodes Excluded by Default (Priority: P1) 🎯 MVP

**Goal**: Nodes with `spec.unschedulable = true` are excluded from the capacity aggregate. No configuration needed — this is the default behaviour.

**Independent Test**: Create a test cluster with 3 schedulable nodes, cordon one, verify `ClusterCapacity.status.nodeCount` drops to 2 and `excludedByUnschedulable` is 1.

### Tests for User Story 1 (TDD — write first, watch fail)

- [ ] T010 [P] [US1] Write test: `is_node_counted(unschedulable=true, labels=None, selector=None)` returns `false` (FR-001) in `src/controllers/node_filter.rs` #[cfg(test)]. Verify the module doesn't exist yet — test fails to compile.
- [ ] T011 [P] [US1] Write test: `is_node_counted(unschedulable=false, labels=None, selector=None)` returns `true` (FR-002) in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T012 [P] [US1] Write test: `sum_node_allocatable` with 3 nodes where 1 is `unschedulable=true` returns `(cpu_of_2, mem_of_2, 2)` and `ExclusionBreakdown { counted: 2, excluded_unschedulable: 1, excluded_by_selector: 0 }` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL (signature changed, new return type).
- [ ] T013 [P] [US1] Write test: `sum_node_allocatable` where ALL nodes are unschedulable returns `(0, 0, 0, breakdown)` — zero capacity (Principle I interaction) in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.

### Implementation for User Story 1

- [ ] T014 [US1] Create `src/controllers/node_filter.rs` with: `pub fn is_node_counted(unschedulable: bool, labels: Option<&BTreeMap<String, String>>, selector: Option<&LabelSelector>) -> bool`, `pub struct ExclusionBreakdown { counted: i32, excluded_unschedulable: i32, excluded_by_selector: i32 }` (with `#[derive(Debug, Clone, Default, PartialEq, Eq)]`), and `pub mod node_filter;` declaration in `src/controllers/mod.rs`. Implement `is_node_counted` with only the unschedulable check for now (return `false` if `unschedulable == true`, `true` otherwise — selector logic added in US2). This makes T010, T011 pass (GREEN).
- [ ] T015 [US1] Modify `sum_node_allocatable` signature in `src/controllers/node_capacity.rs` to accept `selector: Option<&LabelSelector>` as a second parameter and return `(i64, i64, i32, ExclusionBreakdown)`. Add the unschedulable check inside the loop: skip nodes where `node.spec.unschedulable == Some(true)`, incrementing `breakdown.excluded_unschedulable`. Import `ExclusionBreakdown` and `is_node_counted` from `node_filter`. This makes T012, T013 pass (GREEN).
- [ ] T016 [US1] Update `reconcile_now()` and the watcher `for_each` closure in `src/controllers/node_capacity.rs` to call `sum_node_allocatable(snapshot, selector)` with `selector=None` (the selector wiring comes in US2; for now pass `None` to exercise the unschedulable path). Extract `ExclusionBreakdown` from the return and pass the three new fields to `patch_status`.
- [ ] T017 [US1] Modify `patch_status()` and `patch_once()` in `src/controllers/node_capacity.rs` to accept the three new exclusion-count parameters (`excluded_node_count`, `excluded_by_unschedulable`, `excluded_by_selector`) and write them into the `ClusterCapacityStatus` struct.
- [ ] T018 [US1] Update the existing mock-apiserver integration test `reconcile_now_lists_nodes_then_patches_status` in `src/controllers/node_capacity.rs` #[cfg(test)] to assert the new status fields (`excludedNodeCount`, `excludedByUnschedulable`) are present in the PATCH body (value `0` since the mock node is not unschedulable). Mechanical update — add assertions, don't change the test scenario.
- [ ] T019 [US1] Run `cargo test --lib controllers::node_capacity controllers::node_filter` — all tests pass (GREEN).

**Checkpoint**: User Story 1 fully functional and testable. Cordoned nodes are excluded by default. The capacity pool is now accurate.

---

## Phase 3: User Story 2 — Label-Selector Exclusion (Priority: P2)

**Goal**: An optional `LabelSelector` on the `ClusterCapacity` CRD spec excludes arbitrary node subsets by label (e.g. control-plane nodes).

**Independent Test**: Configure `nodeSelector.matchExpressions: [{key: node-role.kubernetes.io/control-plane, operator: Exists}]`; verify control-plane nodes are excluded from the capacity sum.

### Tests for User Story 2 (TDD — write first, watch fail)

- [ ] T020 [P] [US2] Write test: `labels_match_selector` with `matchLabels` that match node labels returns `true`; mismatched returns `false` in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL (function doesn't exist).
- [ ] T021 [P] [US2] Write test: `labels_match_selector` with `matchExpressions` operator `In` — node label value in values returns `true`, not in returns `false` in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T022 [P] [US2] Write test: `labels_match_selector` with operators `NotIn`, `Exists`, `DoesNotExist` — each evaluated correctly in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T023 [P] [US2] Write test: `labels_match_selector` with an empty selector (no `matchLabels`, no `matchExpressions`) returns `true` (matches all — K8s convention, FR-005) in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T024 [P] [US2] Write test: `is_node_counted` with `selector=Some(matching_selector)` returns `false` (FR-003); with `selector=Some(non_matching)` returns `true` in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T025 [P] [US2] Write test: `validate_selector` returns `Ok(())` for valid selectors, `Err(SelectorError::UnknownOperator)` for bad operator, `Err(SelectorError::MissingValues)` for `In` without values in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T026 [P] [US2] Write test: `sum_node_allocatable` with 3 nodes (2 workers + 1 control-plane with label `node-role.kubernetes.io/control-plane`) and selector matching that label returns `(cpu_of_2, mem_of_2, 2)` and `ExclusionBreakdown { counted: 2, excluded_unschedulable: 0, excluded_by_selector: 1 }` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T027 [P] [US2] Write test: `sum_node_allocatable` with an invalid selector (e.g. operator `"Matches"`) falls back to unschedulable-only exclusion — logs warning, no selector-based exclusion (FR-010) in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.

### Implementation for User Story 2

- [ ] T028 [US2] Implement `fn labels_match_selector(labels: &BTreeMap<String, String>, selector: &LabelSelector) -> bool` in `src/controllers/node_filter.rs`. Iterate `matchLabels` (all must match) and `matchExpressions` (each evaluated by operator: `In`, `NotIn`, `Exists`, `DoesNotExist`), AND the results. Empty selector returns `true`. This makes T020–T023 pass (GREEN).
- [ ] T029 [US2] Implement `pub fn validate_selector(selector: &LabelSelector) -> Result<(), SelectorError>` and `pub enum SelectorError` with variants `UnknownOperator`, `MissingValues`, `UnexpectedValues` (derive `thiserror::Error`) in `src/controllers/node_filter.rs`. Check operator validity and value-presence rules per research R4. This makes T025 pass (GREEN).
- [ ] T030 [US2] Extend `is_node_counted` in `src/controllers/node_filter.rs` with the selector path: after the unschedulable check, if `selector` is `Some(sel)` and `validate_selector(sel).is_ok()` and `labels_match_selector(labels, sel)` returns `true`, return `false`. This makes T024 pass (GREEN).
- [ ] T031 [US2] Wire the selector into `sum_node_allocatable` in `src/controllers/node_capacity.rs`: inside the node loop, after the unschedulable check, call `is_node_counted` for the selector path. If `validate_selector` returns `Err`, log `warn!` and skip selector matching for this cycle (fallback). Track `excluded_by_selector` in the breakdown. This makes T026, T027 pass (GREEN).
- [ ] T032 [US2] Wire the selector read in the controller: in `reconcile_now()` and the watcher `for_each` closure in `src/controllers/node_capacity.rs`, read `node_selector` from the `ClusterCapacity` CRD spec (via the reflector cache or a `capacity_api.get()` call). Pass `spec.node_selector.as_ref()` to `sum_node_allocatable`. This implements FR-007/FR-011 (runtime-configurable selector).
- [ ] T033 [US2] Run `cargo test --lib controllers::node_filter controllers::node_capacity` — all tests pass (GREEN).

**Checkpoint**: User Stories 1 AND 2 both work. Cordoned nodes excluded by default; control-plane nodes excludable via label selector.

---

## Phase 4: User Story 3 — Observability of Excluded Nodes (Priority: P3)

**Goal**: The `ClusterCapacity` status reports excluded node counts with a reason breakdown so operators can verify the filter is active.

**Independent Test**: With 5 nodes (1 cordoned, 1 label-matched, 3 counted), verify `status.excludedNodeCount=2`, `excludedByUnschedulable=1`, `excludedBySelector=1`, `nodeCount=3`.

### Tests for User Story 3 (TDD — write first, watch fail)

- [ ] T034 [P] [US3] Write test: `sum_node_allocatable` with 5 nodes (1 unschedulable, 1 selector-matched, 3 counted) returns `ExclusionBreakdown { counted: 3, excluded_unschedulable: 1, excluded_by_selector: 1 }` and `excluded_node_count == 2` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T035 [P] [US3] Write test: node that is BOTH unschedulable AND selector-matched is counted under `excluded_unschedulable` only (not double-counted) in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.

### Implementation for User Story 3

- [ ] T036 [US3] Verify the `ExclusionBreakdown.excluded_node_count` computed in `sum_node_allocatable` equals `excluded_unschedulable + excluded_by_selector` and is passed correctly to `patch_status`. This is mostly a verification task — the breakdown logic was implemented in T015/T031. Fix the counting order if T034/T035 reveal a double-count bug (unschedulable check must come before selector check). This makes T034, T035 pass (GREEN).
- [ ] T037 [US3] Run `cargo test --lib controllers::node_capacity` — all observability tests pass (GREEN).

**Checkpoint**: All three user stories are functional and independently testable.

---

## Phase 5: Integration & BDD Tests

**Purpose**: Wire the feature into the existing test infrastructure — mock-apiserver integration tests and BDD `.feature` scenarios.

### Integration Tests (mock apiserver via tower-test)

- [ ] T038 [P] Write integration test: cordon event updates capacity status — mock apiserver serves a node list with one `unschedulable: true` node; controller reconciles; assert status PATCH has `nodeCount` excluding the cordoned node and `excludedByUnschedulable: 1` in `tests/integration/node_filter.rs`. Verify FAIL.
- [ ] T039 [P] Write integration test: label-selector change updates capacity — mock apiserver serves nodes with labels; `ClusterCapacity` spec has `nodeSelector`; controller reconciles; assert status excludes matching nodes in `tests/integration/node_filter.rs`. Verify FAIL.
- [ ] T040 Implement the integration test scenarios from T038/T039 in `tests/integration/node_filter.rs` using the existing `tower-test` mock apiserver pattern (see `tests/integration/capacity_awareness.rs` for the established pattern). This makes T038, T039 pass (GREEN).
- [ ] T041 Add `[[test]] name = "node_filter" path = "tests/integration/node_filter.rs"` to `Cargo.toml`.

### BDD Tests (cucumber-rs)

- [ ] T042 [P] Write BDD feature file: `@cordon` scenario (cordoned node excluded), `@selector` scenario (control-plane excluded by label), `@observability` scenario (status shows breakdown) in `tests/bdd/features/node_filter.feature`. Follow the Gherkin style of existing `.feature` files.
- [ ] T043 [P] Write BDD step definitions in `tests/bdd/steps/node_filter_steps.rs` implementing the Given/When/Then steps from T042, using the `World` struct pattern from `tests/bdd/steps/capacity_steps.rs`.
- [ ] T044 Add `[[test]] name = "node_filter_bdd" path = "tests/bdd/steps/node_filter_steps.rs" harness = false` to `Cargo.toml`.
- [ ] T045 Run `cargo test --test node_filter --test node_filter_bdd` — all integration and BDD tests pass (GREEN).

**Checkpoint**: Feature is covered by integration and BDD tests.

---

## Phase 6: Deploy Manifest & README

**Purpose**: Update the CRD manifest and user-facing documentation.

- [ ] T046 Update `deploy/crds.yaml`: add `nodeSelector` (with `matchLabels` + `matchExpressions` sub-properties) under the `ClusterCapacity` spec schema; add `excludedNodeCount`, `excludedByUnschedulable`, `excludedBySelector` integer fields under the status schema. Match the schema in `data-model.md` §1.3.
- [ ] T047 [P] Update `README.md`: add a "Node Exclusion" section documenting (a) default unschedulable exclusion, (b) `nodeSelector` label-selector configuration with `kubectl patch` examples, (c) the new status observability fields. Include the control-plane exclusion example from `contracts/clustercapacity-crd.md`. Required by FR-012 / Constitution Principle X.
- [ ] T048 [P] Update `specs/001-capacity-admission-webhook/contracts/clustercapacity-crd.md` (the base contract) with a note pointing to the spec-006 delta for `nodeSelector` and the new status fields. Cross-reference, don't duplicate.

**Checkpoint**: Manifests and docs match the implementation.

---

## Phase 7: Quality Gate & Validation

**Purpose**: Full CI gate — all jobs must pass.

- [ ] T049 Run `cargo fmt --check` — fix any formatting issues.
- [ ] T050 Run `cargo clippy -- -D warnings` — fix any lint issues.
- [ ] T051 Run `cargo test --all-targets` — all unit + integration + BDD tests pass.
- [ ] T052 Run the quickstart validation scenarios from `specs/006-schedulable-node-filter/quickstart.md` — verify each test command produces the expected results.
- [ ] T053 Verify existing E2E CI (k8s 1.34/1.35/1.36) still passes with the updated CRD manifest. The CRD change is additive so no E2E workflow changes are needed.
- [ ] T054 Final review: confirm all FR-001 through FR-012 are covered by tests or implementation, and the Constitution Check in `plan.md` still holds.

**Checkpoint**: CI green, feature complete.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Foundational (Phase 1)**: No dependencies — start immediately. BLOCKS all user stories.
- **User Story 1 (Phase 2)**: Depends on Phase 1.
- **User Story 2 (Phase 3)**: Depends on Phase 1 + the `is_node_counted` function from Phase 2 (T014). US2 extends the selector logic added in US1.
- **User Story 3 (Phase 4)**: Depends on Phase 1 + the `ExclusionBreakdown` from Phase 2/3. Mostly verification — the counting logic is implemented in US1/US2.
- **Integration & BDD (Phase 5)**: Depends on Phases 2–4 (all user stories implemented).
- **Deploy & README (Phase 6)**: Can start in parallel with Phase 5 (different files).
- **Quality Gate (Phase 7)**: Depends on all prior phases.

### Within Each User Story

- Tests (RED) MUST be written and watched to FAIL before implementation
- Pure functions before controller wiring
- Controller wiring before status patching
- Story complete before moving to next priority

### Parallel Opportunities

- T001–T004 (foundational tests) can all run in parallel (different test functions in the same file, but independently writable).
- T020–T027 (US2 tests) can run in parallel (independent test functions).
- T038–T039 (integration tests) and T042–T043 (BDD tests) can run in parallel (different files).
- T047 (README) and T048 (base contract update) can run in parallel with Phase 5.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Foundational CRD changes
2. Complete Phase 2: User Story 1 (cordon exclusion)
3. **STOP and VALIDATE**: `cargo test --lib controllers::node_capacity controllers::node_filter`
4. At this point, the phantom-capacity bug is fixed — cordoned nodes are excluded

### Incremental Delivery

1. Foundational → CRD structs carry new fields
2. US1 → Cordon exclusion works (bug fix)
3. US2 → Label-selector exclusion works (configurability)
4. US3 → Status observability works (operator visibility)
5. Integration + BDD → Feature is covered by the full test pyramid
6. Deploy + README → Manifests and docs match the implementation
7. Quality Gate → CI green, feature complete

---

## Notes

- All tasks follow strict TDD: test first (RED), implement (GREEN), refactor.
- The `node_filter.rs` module is pure (no async, no client, no I/O) — fully unit-testable.
- No new dependencies, no new RBAC, no new CRD version — purely additive.
- The `ClusterCapacity` spec previously had no fields (`struct ClusterCapacitySpec {}`); this adds the first user-configurable field on the supply CRD.
- Existing tests that construct `ClusterCapacitySpec {}` or `ClusterCapacityStatus { ... }` literals need mechanical updates (the compiler will flag these).
- Commit after each task or logical group, following the branch-and-PR rule (Constitution: `spec/006-schedulable-node-filter` branch → PR → merge).
