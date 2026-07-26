# Tasks: Controller Singleton Autocreation

**Input**: Design documents from `specs/003-controller-singleton-autocreation/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, quickstart.md,
`.specify/memory/constitution.md`

**Branch**: `spec/controller-singleton-autocreation` (per constitution v2.4.0
branch-and-PR rule)

**Tests**: REQUIRED (constitution Principle VIII, NON-NEGOTIABLE). TDD strict:
write the test first, watch it fail, then implement. The `ensure_singleton`
function has network-dependent logic (Api calls), so extract the decision logic
(create vs skip) into a pure, testable function where possible, or test the
kube::Error matching (404 vs 409 vs other) as a unit.

## Format: `[ID] [P?] [Story?] Description`

---

## Phase 1: Setup (Shared Infrastructure)

- [ ] T001 Read `src/controllers/node_capacity.rs` and `src/controllers/allocation.rs` — confirm the current bug: both call `patch_status` / `get` without ensuring the singleton instance exists
- [ ] T002 [P] Read `specs/003-controller-singleton-autocreation/research.md` and `data-model.md` — understand the `ensure_singleton` pattern (get-or-create, 409 idempotent)

---

## Phase 2: User Story 1 — ClusterCapacity Singleton Autocreation (P1)

**Goal**: The Node Capacity Controller creates `cluster-capacity` if missing.

### Tests (TDD RED)

- [ ] T003 [P] [US1] Write unit tests for `ensure_singleton` in `src/controllers/node_capacity.rs` (`#[cfg(test)]` module) — cover: (1) when `get` returns Ok (instance exists) → no create attempted; (2) when `get` returns 404 → create called with `ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {})`; (3) when create returns 409 AlreadyExists → treated as success (no error). Use a helper that tests the decision logic (what action to take given a get result)

### Implementation (TDD GREEN)

- [ ] T004 [US1] Implement `ensure_singleton` function in `src/controllers/node_capacity.rs` — async fn that takes `&Api<ClusterCapacity>`, calls `get(CLUSTER_CAPACITY_NAME)`, on 404 creates with empty spec, handles 409 as success, logs all paths. See data-model.md §1 for the flowchart
- [ ] T005 [US1] Call `ensure_singleton(&capacity_api).await` at the top of the `run` function in `src/controllers/node_capacity.rs`, before the reflector stream loop starts

**Checkpoint**: ClusterCapacity singleton is auto-created on controller startup.

---

## Phase 3: User Story 2 — Allocation Singleton Autocreation (P2)

**Goal**: The Allocation Controller creates `cluster-allocation` with default
budgetPercent=80 if missing.

### Tests (TDD RED)

- [ ] T006 [P] [US2] Write unit tests for `ensure_singleton` in `src/controllers/allocation.rs` (`#[cfg(test)]` module) — cover: (1) when `get` returns Ok (instance exists with budgetPercent=50) → no create, existing budget preserved; (2) when `get` returns 404 → create called with `Allocation::new(CLUSTER_ALLOCATION_NAME, AllocationSpec { budget_percent: 80 })`; (3) when create returns 409 → treated as success

### Implementation (TDD GREEN)

- [ ] T007 [US2] Implement `ensure_singleton` function in `src/controllers/allocation.rs` — same pattern as node_capacity but creates with `AllocationSpec { budget_percent: 80 }`. See data-model.md §2
- [ ] T008 [US2] Call `ensure_singleton(&allocation_api).await` at the top of the `run` function in `src/controllers/allocation.rs`, before the ticker loop starts

**Checkpoint**: Allocation singleton is auto-created with default budget 80%.

---

## Phase 4: User Story 3 — Documentation + CI Updates (P3)

**Goal**: Contracts, README, and CI reflect the autocreation behaviour.

- [ ] T009 [P] [US3] Revert the CI workaround in `.github/workflows/ci.yml` — remove the `kubectl apply` that creates the `cluster-capacity` ClusterCapacity instance from the "Configure the budget" step. Keep the Allocation creation step (it tests that the controller does NOT overwrite an operator-set budget). Also remove the extended Allocation-status wait loop (T003-T005 from spec-002 tasks were a workaround; the controllers now populate status immediately after singleton creation)
- [ ] T010 [P] [US3] Update `README.md` — remove any manual singleton-creation commands from the Quick Start. Add a note that the controllers auto-create both singletons (ClusterCapacity with empty spec, Allocation with budgetPercent=80). Update the configuration section to note that budgetPercent=80 is the auto-created default, changeable via kubectl patch
- [ ] T011 [P] [US3] Update `specs/001-capacity-admission-webhook/contracts/clustercapacity-crd.md` — add a "Singleton Lifecycle" subsection documenting that the Node Capacity Controller auto-creates the `cluster-capacity` instance with empty spec if it does not exist
- [ ] T012 [P] [US3] Update `specs/001-capacity-admission-webhook/contracts/allocation-crd.md` — add a "Singleton Lifecycle" subsection documenting that the Allocation Controller auto-creates the `cluster-allocation` instance with budgetPercent=80 if it does not exist, and never overwrites an existing instance

---

## Phase 5: Polish & Verification

- [ ] T013 Run `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — all must pass before commit (quality gate)
- [ ] T014 Verify the diff touches only: `src/controllers/node_capacity.rs`, `src/controllers/allocation.rs`, `.github/workflows/ci.yml`, `README.md`, and the two contracts files. No unintended changes
- [ ] T015 Commit with message: `fix(controllers): auto-create singleton CRD instances (spec-003)`

---

## Dependencies & Execution Order

- Phase 1 (T001-T002): no dependencies, start immediately
- Phase 2 (T003-T005): depends on Phase 1
- Phase 3 (T006-T008): depends on Phase 1 (parallel with Phase 2 if separate files)
- Phase 4 (T009-T012): depends on Phases 2+3 (CI and docs reflect the code fix)
- Phase 5 (T013-T015): depends on all prior phases

## Parallel Opportunities

- T002 is [P] (reading, no dependency on T001)
- T006 is [P] (different file from T003)
- T009-T012 are all [P] (different files)
