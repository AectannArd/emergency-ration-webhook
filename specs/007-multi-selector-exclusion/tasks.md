# Tasks: Multi-Selector Node Exclusion

**Input**: Design documents from `/specs/007-multi-selector-exclusion/`

**Tests**: Strict TDD (Constitution Principle VIII). Tests first, watch fail, then implement.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: CRD Migration — `nodeSelector` → `nodeSelectors` (Foundational)

**Purpose**: Rename the singular field to a list across the CRD struct, controller, manifest, and all existing tests.

### Tests (RED first)

- [ ] T001 [P] Write test: `ClusterCapacitySpec` with `node_selectors: Option<Vec<LabelSelector>>` serialises camelCase as `nodeSelectors` (array) and round-trips in `src/crd/cluster_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T002 [P] Write test: `ClusterCapacity::crd()` generated schema includes `nodeSelectors` (array type) under `spec.properties` in `src/crd/cluster_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T003 [P] Write test: `default_capacity_singleton()` creates `ClusterCapacitySpec { node_selectors: None }` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.

### Implementation (GREEN)

- [ ] T004 Rename `node_selector: Option<LabelSelector>` → `node_selectors: Option<Vec<LabelSelector>>` in `ClusterCapacitySpec` in `src/crd/cluster_capacity.rs`. Update the doc comment. This makes T001, T002 pass.
- [ ] T005 Update `default_capacity_singleton()` in `src/controllers/node_capacity.rs` to construct `ClusterCapacitySpec { node_selectors: None }`. Makes T003 pass.
- [ ] T006 Mechanically update ALL existing `ClusterCapacitySpec { node_selector: ... }` literals across `src/` and `tests/` to `node_selectors: ...`. The compiler will flag every site — fix each. Wrap any existing single-selector literal in `Some(vec![...])`.
- [ ] T007 Run `cargo check --all-targets` — compiles clean. Then `cargo test --lib crd::cluster_capacity` — all CRD tests pass.

**Checkpoint**: CRD migration complete, all existing code compiles with the new field name.

---

## Phase 2: Multi-Selector Filter Logic (US1)

**Goal**: A node is excluded if it matches ANY selector in the list (OR semantics).

### Tests (RED first)

- [ ] T008 [P] [US1] Write test: `labels_match_any_selector` returns `true` when labels match 1 of 3 selectors, `false` when matching none, `false` for empty selector list in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL (function doesn't exist).
- [ ] T009 [P] [US1] Write test: `is_node_counted` with `selectors=Some(&[matching_sel])` returns `false`; with `selectors=Some(&[non_matching, matching])` returns `false` (ANY match excludes); with `selectors=Some(&[non_matching])` returns `true` in `src/controllers/node_filter.rs` #[cfg(test)]. Verify FAIL.
- [ ] T010 [P] [US1] Write test: `sum_node_allocatable` with 2 selectors (control-plane + experimental) and nodes matching each → both excluded, `excluded_by_selector == 2`, `counted == workers_only` in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T011 [P] [US1] Write test: node matching BOTH selectors is excluded once (not double-counted in `excluded_by_selector`) in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.
- [ ] T012 [P] [US1] Write test: `effective_selectors` with a list containing 2 valid + 1 invalid selector returns only the 2 valid ones (logs warning for the invalid one) in `src/controllers/node_capacity.rs` #[cfg(test)]. Verify FAIL.

### Implementation (GREEN)

- [ ] T013 [US1] Add `fn labels_match_any_selector(labels: &BTreeMap<String, String>, selectors: &[LabelSelector]) -> bool` in `src/controllers/node_filter.rs`. Uses `selectors.iter().any(|sel| labels_match_selector(labels, sel))`. Makes T008 pass.
- [ ] T014 [US1] Modify `is_node_counted` in `src/controllers/node_filter.rs`: change `selector: Option<&LabelSelector>` → `selectors: Option<&[LabelSelector]>`. Replace single-selector matching with `labels_match_any_selector`. Makes T009 pass.
- [ ] T015 [US1] Add `fn effective_selectors(selectors: Option<&[LabelSelector]>) -> Vec<&LabelSelector>` in `src/controllers/node_capacity.rs` (replaces `effective_selector`). Filters invalid selectors with `warn!` per entry. Makes T012 pass.
- [ ] T016 [US1] Modify `sum_node_allocatable` signature in `src/controllers/node_capacity.rs`: `selector: Option<&LabelSelector>` → `selectors: Option<&[LabelSelector]>`. Call `effective_selectors` then pass the validated slice. Update the node loop to use `is_node_counted` with the slice. Makes T010, T011 pass.
- [ ] T017 [US1] Rename `read_selector` → `read_selectors` in `src/controllers/node_capacity.rs`. Returns `Option<Vec<LabelSelector>>` reading `cc.spec.node_selectors`. Update callers (`reconcile_now`, watcher closure).
- [ ] T018 [US1] Run `cargo test --lib controllers::node_filter controllers::node_capacity` — all tests pass.

**Checkpoint**: Multi-selector OR logic working, all unit tests green.

---

## Phase 3: Integration + BDD Tests

- [ ] T019 [P] Add integration test scenario to `tests/integration/node_filter.rs`: mock apiserver with `nodeSelectors` (array of 2 selectors); assert both label-groups excluded. Verify FAIL then implement.
- [ ] T020 [P] Add BDD scenario to `tests/bdd/features/node_filter.feature` + step defs in `tests/bdd/steps/node_filter_steps.rs`: "Nodes matching any of multiple selectors are excluded". Verify FAIL then implement.
- [ ] T021 Run `cargo test --test node_filter --test node_filter_bdd` — all pass.

---

## Phase 4: Deploy Manifest + README

- [ ] T022 [P] Update `deploy/crds.yaml`: rename `nodeSelector` → `nodeSelectors` (type: array, items: LabelSelector schema).
- [ ] T023 [P] Update `README.md`: change the "Node Exclusion" section to document multi-selector OR semantics, migration from singular to array, and the control-plane + experimental example.
- [ ] T024 [P] Update `specs/006-schedulable-node-filter/contracts/clustercapacity-crd.md` with a note that spec-007 renames `nodeSelector` → `nodeSelectors`.

---

## Phase 5: Quality Gate

- [ ] T025 Run `cargo fmt --check` — fix issues.
- [ ] T026 Run `cargo clippy --all-targets -- -D warnings` — fix issues.
- [ ] T027 Run `cargo test --all-targets` — all pass.
- [ ] T028 Verify quickstart.md validation commands produce expected results.
- [ ] T029 Final review: confirm FR-001 through FR-012 covered.

---

## Dependencies & Execution Order

- Phase 1 (T001-T007) blocks everything — the field rename must compile first.
- Phase 2 (T008-T018) depends on Phase 1 — implements the OR logic.
- Phase 3 (T019-T021) depends on Phase 2 — integration/BDD coverage.
- Phase 4 (T022-T024) can run in parallel with Phase 3.
- Phase 5 (T025-T029) depends on all prior phases.

### Parallel Opportunities

- T001-T003 (foundational tests) are independent.
- T008-T012 (US1 tests) are independent.
- T019-T020 (integration + BDD) are independent (different files).
- T022-T024 (deploy + README + contract) are independent.
