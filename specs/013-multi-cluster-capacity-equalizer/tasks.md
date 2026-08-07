# Tasks: Multi-Cluster Capacity Equalizer (spec-013)

**Input**: Design documents from `/specs/013-multi-cluster-capacity-equalizer/`
(plan.md, spec.md, research.md, data-model.md, contracts/equalizer-config-crd.md,
contracts/target-cluster-api.md, quickstart.md)

**Prerequisites**: plan.md, spec.md (required); research.md, data-model.md,
contracts/, quickstart.md (all present).

**Tests**: TDD is NON-NEGOTIABLE (Constitution Principle VIII). Every
implementation task is preceded by its test task (RED → GREEN → REFACTOR).

**Organization**: Tasks are grouped by user story. The contracts
(`contracts/equalizer-config-crd.md`, `contracts/target-cluster-api.md`) are
authoritative; where data-model.md or quickstart.md describe behavior, they agree
with the contracts.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: Which user story this task belongs to (US1/US2/US3)
- All paths are repository-relative

## Path Conventions

Single project: `src/`, `tests/`, `deploy/` at repository root. The equalizer
binary lives under `src/bin/capacity-equalizer/`. The equalizer library module
lives under `src/equalizer/`. See `plan.md` §Project Structure for the full tree.

---

## Phase 1: Setup

**Purpose**: Create the implementation branch, add the binary target to
Cargo.toml, create the module skeleton, confirm baseline green.

- [ ] T001 Create implementation branch `spec/013-multi-cluster-capacity-equalizer` off `main`. Confirm `cargo build` + `cargo test` pass on the unmodified tree (baseline green — Constitution Principle XI). Commit nothing yet.
- [ ] T002 Add `[[bin]] name = "capacity-equalizer" path = "src/bin/capacity-equalizer/main.rs"` to `Cargo.toml`. Create `src/bin/capacity-equalizer/main.rs` with a minimal `fn main() {}` stub. Create `src/equalizer/mod.rs` with a module-level doc comment. Add `pub mod equalizer;` to `src/lib.rs`. Confirm `cargo build --bin capacity-equalizer` compiles. Do NOT commit yet (foundational phase will fill in the module).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The EqualizerConfig CRD types, the pure equalization algorithm, and
the multi-cluster client construction helper — all pure/unit-testable, depended
on by every user story. No user-story work can begin until this phase is complete.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Tests for Foundational (write FIRST, watch RED)

- [ ] T003 [P] Unit test: EqualizerConfig CRD identity — correct name (`equalizerconfigs.emergency-ration.dev`), scope `Cluster`, kind `EqualizerConfig`, short name `eqconf`, status subresource declared — in `tests/equalizer/algorithm.rs` (or a dedicated CRD test file). Watch fail (CRD type absent).
- [ ] T004 [P] Unit test: EqualizerConfig spec fields serialise camelCase (`cpuTargetBudgetPercent`, `memoryTargetBudgetPercent`, `targets` with `name` + `kubeconfigSecretRef`), round-trip, and carry `#[schemars(range(min=0,max=100))]` on the budget fields. Watch fail (struct absent).
- [ ] T005 [P] Unit test: `SecretRef.key` defaults to `"kubeconfig"` when absent. Watch fail.
- [ ] T006 [P] Unit test: EqualizerConfig status fields serialise camelCase — `clusters` array, `condition`, `lastReconciled`; `ClusterState` serialises kebab-case (`healthy`/`over`/`unreachable`/`config-error`); `FleetCondition` serialises kebab-case (`healthy`/`compensating`/`degraded`). Watch fail.
- [ ] T007 [P] Unit test: `equalize()` — all-under case (target 80, util 65/55/45, uniform 100_000m → budgets 80/80/80, all Good). Data-model §2.3 Example 1. Watch fail (function absent).
- [ ] T008 [P] Unit test: `equalize()` — one over (target 80, util 65/55/90 → 75/75/90). Example 2. Watch fail.
- [ ] T009 [P] Unit test: `equalize()` — over drops (target 80, util 65/55/86 → 77/77/86). Example 3. Watch fail.
- [ ] T010 [P] Unit test: `equalize()` — all over (target 80, util 85/85/85 → 85/85/85, all Over). Example 4. Watch fail.
- [ ] T011 [P] Unit test: `equalize()` — non-uniform capacity (A=100_000m@60%, B=200_000m@60%, C=200_000m@95%, target 80 → 65/73/95). Example 5. Watch fail.
- [ ] T012 [P] Unit test: `equalize()` — single cluster under target → budget=target. Single cluster over → frozen. Watch fail.
- [ ] T013 [P] Unit test: `equalize()` — zero-capacity cluster (allocatable=0) does not contribute overflow and gets budget=target. Watch fail.
- [ ] T014 [P] Unit test: `equalize()` — multiple over-clusters, combined overflow distributed among good clusters. Watch fail.
- [ ] T015 [P] Unit test: `equalize()` — over→good transition (cluster was over at 90%, drops to 70% which is under target → becomes Good, budget=target). Watch fail.
- [ ] T016 [P] Unit test: fleet condition aggregation — all healthy → `Healthy`; one over → `Compensating`; one unreachable → `Degraded`; mixed over+unreachable → `Degraded` (highest severity). Watch fail.
- [ ] T017 [P] Unit test: `build_target_client(kubeconfig_bytes)` constructs a `kube::Client` from valid kubeconfig YAML bytes (use a test kubeconfig fixture). Watch fail (function absent).

### Implementation for Foundational (GREEN)

- [ ] T018 [US1] Define the EqualizerConfig CRD in `src/equalizer/crd.rs`: `EqualizerConfigSpec`, `TargetCluster`, `SecretRef` (with `key` default), `EqualizerConfigStatus`, `ClusterObservation`, `ClusterState` enum, `FleetCondition` enum, and the `FLEET_EQUALIZER_NAME` constant. Use `#[derive(CustomResource, ...)]` with `#[kube(group = "emergency-ration.dev", version = "v1", kind = "EqualizerConfig", status = "EqualizerConfigStatus", shortname = "eqconf")]`. Add `pub mod crd;` to `src/equalizer/mod.rs`. Make T003-T006 pass.
- [ ] T019 [US1] Implement `pub fn equalize(observations: &[ClusterResourceObservation], target_budget_percent: i32) -> Vec<ComputedBudget>` in `src/equalizer/algorithm.rs` per data-model.md §2.2. Include `ClusterResourceObservation`, `BudgetState`, `ComputedBudget` types. Use i128 intermediates for the absolute overflow arithmetic; clamp budgets to [0,100]. Make T007-T015 pass.
- [ ] T020 [US1] Implement `pub fn fleet_condition(states: &[ClusterState]) -> FleetCondition` in `src/equalizer/algorithm.rs` (or `crd.rs` next to the enum). Aggregation: any Unreachable/ConfigError → Degraded; else any Over → Compensating; else Healthy. Make T016 pass.
- [ ] T021 [US1] Implement `pub async fn build_target_client(kubeconfig_bytes: &[u8]) -> Result<Client>` in `src/equalizer/cluster_client.rs` using `Kubeconfig::read_from_yaml` + `Config::from_custom_kubeconfig` (research R1, mirrors `src/bin/erw-verify/client.rs`). Make T017 pass.
- [ ] T022 [US1] Implement the binary entry point `src/bin/capacity-equalizer/main.rs`: install rustls ring CryptoProvider as the first line (CI failure catalog Layer 2), init tracing-subscriber, call the reconcile loop (stub: `todo!()` for now — Phase 3 fills it in). Confirm `cargo build --bin capacity-equalizer` compiles.

**Checkpoint**: Foundation ready — `cargo test --test algorithm` green; the CRD
types, algorithm, and client helper are proven pure/unit-testable. The reconcile
loop (the wiring) is still a stub.

---

## Phase 3: User Story 1 — Equalization: All Clusters Within Target (Priority: P1) 🎯 MVP

**Goal**: The full reconcile loop — read EqualizerConfig, construct per-target
clients from Secrets, GET each target's Allocation + ClusterCapacity status,
compute budgets via `equalize()`, patch each target's Allocation.spec overrides,
write EqualizerConfig.status. The all-under-target case is the baseline.

**Independent Test**: 3 mocked target clusters all under target → each patched
to target budget, status reports Healthy (US1 AC1/AC2).

### Tests for User Story 1 (write FIRST, watch RED)

- [ ] T023 [P] [US1] Integration test: 3 mocked target apiservers, each returning Allocation at 65/55/45% utilization + ClusterCapacity at 100_000m. EqualizerConfig with cpuTargetBudgetPercent=80. After reconcile: each target mock received a PATCH with `cpuBudgetPercent: 80`. Status reports all clusters Healthy, fleet condition Healthy. In `tests/equalizer/reconcile.rs`. Watch fail (reconcile loop absent).
- [ ] T024 [P] [US1] BDD scenario in `tests/bdd/features/equalizer.feature` + step definitions in `tests/bdd/steps/equalizer_steps.rs`: "All clusters within target — budgets set to target" per quickstart V1.4. Watch fail.

### Implementation for User Story 1 (GREEN)

- [ ] T025 [US1] Implement the reconcile loop `pub async fn reconcile(home_client: &Client, eq_config: &EqualizerConfig) -> EqualizerConfigStatus` in `src/equalizer/reconcile.rs` per data-model.md §3 and contracts/target-cluster-api.md:
  1. For each target (concurrent via `tokio::try_join_all` or `futures::join_all`): read kubeconfig Secret from home cluster → `build_target_client` → GET Allocation status (utilization) + ClusterCapacity status (allocatable). On error, record Unreachable/ConfigError.
  2. Build `ClusterResourceObservation` vectors for CPU and RAM.
  3. Call `equalize()` twice (CPU + RAM, independently).
  4. For each reachable cluster (concurrent): PATCH `Allocation.spec` with computed `cpuBudgetPercent` + `memoryBudgetPercent` via strategic-merge patch (contracts/target-cluster-api.md §3.1). ONLY the two override fields — never `budgetPercent`.
  5. Build `EqualizerConfigStatus` with per-cluster observations + fleet condition + timestamp.
  Make T023 pass.
- [ ] T026 [US1] Implement the main runtime loop in `src/bin/capacity-equalizer/main.rs`: `Client::try_default()` for the home cluster, create the EqualizerConfig CRD singleton if absent (or log + idle if the operator hasn't created it — the equalizer does NOT auto-create EqualizerConfig, per contract §4.2), then loop every 10s (configurable via flag/env): read EqualizerConfig spec → `reconcile()` → write EqualizerConfig.status via `patch_status`. Install the rustls CryptoProvider BEFORE `Client::try_default()`. Make the binary runnable end-to-end.
- [ ] T027 [US1] REFACTOR: run `cargo clippy -- -D warnings` + `cargo fmt --check` on all new files. Remove dead code/imports. Confirm the algorithm unit tests + reconcile integration test + BDD all pass.

**Checkpoint**: US1 fully functional. The equalizer reads the config, connects to
targets, computes all-under budgets, patches overrides, writes status. `cargo
test --test reconcile equalize_all_under` + `cargo test --test equalizer_bdd`
green. This is the MVP — an operator can deploy the equalizer and it will set
every cluster to the target budget when all are under.

---

## Phase 4: User Story 2 — Over-Limit Compensation (Priority: P2)

**Goal**: The over-cluster freeze + overflow compensation algorithm exercised
through the full reconcile loop. Dynamic recalculation when the over-cluster
drops.

**Independent Test**: 3 clusters at 65/55/90% → budgets 75/75/90; when the 90%
drops to 86% → 77/77/86 (US2 AC1/AC2).

**Note**: the pure algorithm is already proven in Phase 2 (T007-T015). US2 adds
the integration-level verification that the reconcile loop patches the correct
per-resource budgets when over-clusters are present, and that CPU/RAM are
equalized independently when their states disagree.

### Tests for User Story 2 (write FIRST, watch RED)

- [ ] T028 [P] [US2] Integration test: 3 mocks, CPU util 65/55/90%, all 100_000m. After reconcile: cluster C receives `cpuBudgetPercent: 90` (frozen), A/B receive `cpuBudgetPercent: 75` (compensated). In `tests/equalizer/reconcile.rs`. Watch fail (may already pass from T025 if the loop is generic — still write the test FIRST per TDD).
- [ ] T029 [P] [US2] Integration test: two-cycle scenario — cycle 1 util 65/55/90% (budgets 75/75/90), cycle 2 util 65/55/86% (budgets 77/77/86). Assert the patches change between cycles. Watch fail.
- [ ] T030 [P] [US2] Integration test: all-over (85/85/85%) → all frozen at 85, no compensation patches. Watch fail.
- [ ] T031 [P] [US2] Integration test: CPU/RAM independence — CPU all-under (80/80/80), RAM one-over (75/75/90 → RAM 75/75/90). Assert CPU and RAM override fields differ on the same cluster. Watch fail.

### Implementation for User Story 2 (VERIFICATION-ONLY — likely no new code)

- [ ] T032 [US2] Run the US2 integration tests. If they pass (the reconcile loop from T025 already calls `equalize()` generically, so over-compensation should work out of the box), this is verification-only — no new code needed. If any test fails, the reconcile loop has a bug in how it patches per-resource budgets or handles the over/good partition — fix it at the root (not the test). Commit baseline-green evidence.

**Checkpoint**: US2 certified. Over-cluster compensation works end-to-end through
the reconcile loop, with dynamic recalculation and CPU/RAM independence proven.

---

## Phase 5: User Story 3 — Target Reachability and Status Reporting (Priority: P3)

**Goal**: Unreachable/config-error clusters are skipped, reported in status, and
the remaining clusters continue equalizing. Recovery on the next cycle.

**Independent Test**: 3 clusters, one unreachable → 2 patched normally, 1
reported Unreachable, no crash (US3 AC1).

**Note**: the reconcile loop from T025 already records Unreachable/ConfigError
in the per-cluster observation. US3 adds the integration-level verification.

### Tests for User Story 3 (write FIRST, watch RED)

- [ ] T033 [P] [US3] Integration test: 3 mocks, cluster C's apiserver returns an error. Assert A/B receive their computed budgets, C is NOT patched (no PATCH call to C's mock), C's status is `Unreachable` with an error message. In `tests/equalizer/reconcile.rs`. Watch fail.
- [ ] T034 [P] [US3] Integration test: cluster C's kubeconfig Secret is missing from the home-cluster mock. Assert C is `ConfigError`, A/B managed normally. Watch fail.
- [ ] T035 [P] [US3] Integration test: recovery — C unreachable on cycle 1, reachable on cycle 2. Assert C transitions `Unreachable → Healthy`, budget patched on cycle 2. Watch fail.
- [ ] T036 [P] [US3] Integration test: full status shape — `kubectl get equalizerconfig -o yaml` equivalent (the status struct has all 10 fields per ClusterObservation + condition + lastReconciled). Assert via the returned `EqualizerConfigStatus` struct, not kubectl. Watch fail.

### Implementation for User Story 3 (VERIFICATION-ONLY — likely no new code)

- [ ] T037 [US3] Run the US3 integration tests. The reconcile loop from T025 already handles errors per-cluster (records ClusterState, skips patching). If all pass, verification-only. If failures, fix the error-handling path in `reconcile.rs` at the root. Commit baseline-green evidence.

**Checkpoint**: US3 complete. The equalizer is production-grade — unreachable
clusters don't block the fleet, errors are reported, recovery is automatic.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Deploy manifests, Docker image, CI integration, documentation,
erw-verify extension, and the final quality gate.

- [ ] T038 Generate the EqualizerConfig CRD manifest: write a small test or binary that prints `EqualizerConfig::crd()` as YAML, output to `deploy/equalizer/crds.yaml`. Verify the schema matches contracts/equalizer-config-crd.md §2-3 (spec fields with range constraints, status fields, scope Cluster, short name eqconf). Contract §5.
- [ ] T039 Write `deploy/equalizer/rbac.yaml`: ServiceAccount, ClusterRole (get Secrets, CRUD equalizerconfigs), ClusterRoleBinding. Also include an example target-cluster ClusterRole comment block (get/patch allocations, get clustercapacities) for operators to apply in each target. Research R9, contracts/target-cluster-api.md §4.
- [ ] T040 Write `deploy/equalizer/deployment.yaml`: Deployment running `capacity-equalizer`, ENV vars for reconcile interval + namespace, volume mounts for kubeconfig Secrets (or they're read via API — clarify in the manifest comments). Reference the `Dockerfile.equalizer` image.
- [ ] T041 Write `deploy/equalizer/equalizer-config.example.yaml`: a commented example EqualizerConfig singleton with 2 target clusters + example kubeconfig Secret objects. This is the operator's quickstart reference (Principle X).
- [ ] T042 [P] Write `Dockerfile.equalizer`: multi-stage build (rust:1.89-bookworm builder → distroless runtime), targeting `--bin capacity-equalizer`. Dummy-deps caching layer must stub ALL `[[bin]]` paths (src/main.rs, src/bin/erw-verify/main.rs, src/bin/capacity-equalizer/main.rs — CI failure catalog Layer 9). Research R8.
- [ ] T043 Add `[[test]]` entries to `Cargo.toml` for `tests/equalizer/algorithm.rs` and `tests/equalizer/reconcile.rs` (integration tests under subdirectories must be declared explicitly). Add `[[test]] name = "equalizer_bdd" path = "tests/bdd/steps/equalizer_steps.rs" harness = false`. Pitfall #1 from the skill.
- [ ] T044 [P] Document the EqualizerConfig CRD (spec fields, status fields, singleton convention), deployment instructions, kubeconfig Secret setup, and operational behavior in `README.md` (Constitution Principle X). Include the worked example (3 clusters, 80% target, 65/55/90 → 75/75/90).
- [ ] T045 [P] Update `CONTRIBUTING.md` with the equalizer build/test workflow (`cargo build --bin capacity-equalizer`, `cargo test --test algorithm`, `cargo test --test reconcile`, `cargo test --test equalizer_bdd`). Principle XIII.
- [ ] T046 Write `src/bin/erw-verify/scenarios/equalizer.rs` (FR-015): a new erw-verify scenario module that orchestrates a 2-cluster fixture (two kind clusters or two namespaces in one cluster), installs the webhook in both, deploys the equalizer in one, creates an EqualizerConfig, patches pod loads to trigger equalization, and asserts the budget patches land. Register it in the erw-verify scenario runner (conditional on a `--multi-cluster` flag or similar). This is the heaviest test — may be `#[ignore]` by default.
- [ ] T047 Run the full quality gate (Constitution Principles VIII, IX, XI): `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (unit + integration + BDD). ALL must be green. Fix any failure at its root.
- [ ] T048 Run `quickstart.md` validation: execute the V1.1–V3.5 commands from `specs/013-multi-cluster-capacity-equalizer/quickstart.md` and confirm all pass. SC-001 through SC-005.

**Checkpoint**: Feature complete and documented. Ready for PR.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies — branch + stubs.
- **Foundational (Phase 2)**: depends on Setup — BLOCKS all user stories.
- **US1 (Phase 3)**: depends on Foundational. The reconcile loop is the core wiring.
- **US2 (Phase 4)**: depends on Foundational + US1 (exercises the same reconcile loop with over-cluster scenarios). Verification-only if US1's loop is generic.
- **US3 (Phase 5)**: depends on Foundational + US1 (exercises the error-handling paths in the reconcile loop). Verification-only.
- **Polish (Phase 6)**: depends on all user stories. Manifests, Dockerfile, docs, quality gate.

### User Story Dependencies

- **US1 (P1)**: depends on Foundational only. The MVP — full reconcile loop.
- **US2 (P2)**: depends on Foundational + US1 (the algorithm is already proven in Foundational; US2 verifies it through the loop). Independently testable via over-cluster mocks.
- **US3 (P3)**: depends on Foundational + US1 (error handling already in the loop). Independently testable via unreachable-cluster mocks.

### Within Each User Story

- Tests written FIRST and watched RED (Constitution Principle VIII).
- Pure functions before wiring; wiring before integration tests.
- `cargo clippy` + `cargo fmt --check` after each phase.

### Parallel Opportunities

- T003–T017 (foundational tests) are all `[P]` — different concerns, can be written together.
- T023/T024 (US1 integration + BDD) are `[P]`.
- T028–T031 (US2 tests) are `[P]`.
- T033–T036 (US3 tests) are `[P]`.
- T044/T045 (docs) are `[P]`.

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1).
2. **STOP and VALIDATE**: the equalizer reads config, connects to targets, computes all-under budgets, patches overrides, writes status. `cargo test --test algorithm` + `cargo test --test reconcile` + `cargo test --test equalizer_bdd` green.
3. This is a deployable MVP — an operator can deploy the equalizer and it will set every cluster to the target budget.

### Incremental Delivery

1. Foundational types + algorithm (pure, unit-tested truth table).
2. US1 → reconcile loop → MVP (all-under case works end-to-end).
3. US2 → verify over-compensation through the loop (likely no new code — verification).
4. US3 → verify reachability/error handling through the loop (likely no new code — verification).
5. Polish → manifests, Dockerfile, docs, erw-verify, quality gate → PR-ready.

---

## Notes

- The `equalize()` function is the most critical component — it is pure, fully
  unit-testable via the 5-case truth table (data-model §2.3), and reused
  identically by the reconcile loop. Get it right first; everything else is
  wiring around it.
- The reconcile loop does NOT auto-create the EqualizerConfig singleton
  (contract §4.2) — the operator must create it. The binary logs + idles if
  absent.
- The equalizer patches ONLY `cpuBudgetPercent` / `memoryBudgetPercent` on target
  Allocation singletons (FR-007). NEVER touch `budgetPercent`, `status`,
  `enforcementMode`, or any other field. The strategic-merge patch must contain
  only the two override keys.
- CPU and RAM are equalized via two SEPARATE `equalize()` calls — one per
  resource dimension (FR-014). The reconcile loop calls `equalize()` for CPU,
  `equalize()` for RAM, then merges the results into per-cluster
  `(cpu_budget, mem_budget)` pairs for patching.
- The rustls CryptoProvider MUST be installed as the FIRST line of
  `src/bin/capacity-equalizer/main.rs`, before `Client::try_default()` or any
  TLS-using code (CI failure catalog Layer 2 — the panic fires at the first TLS
  operation, often `Client::try_default()`).
- The `Dockerfile.equalizer` dummy-deps caching layer MUST stub ALL `[[bin]]`
  paths: `src/main.rs`, `src/bin/erw-verify/main.rs`, AND
  `src/bin/capacity-equalizer/main.rs` (CI failure catalog Layer 9).
- Integration test targets under `tests/equalizer/` must be declared as `[[test]]`
  entries in Cargo.toml or `cargo test` won't find them (Pitfall #1).
- The `.dockerignore` at the repo root is an untracked leftover from a prior
  session; do NOT commit it as part of this spec's work.
- If the implementing agent discovers the reconcile loop is generic enough that
  US2/US3 tests pass without new code (verification-only phases), that is the
  correct outcome — the algorithm and error handling were proven in Foundational.
  Do NOT invent unnecessary code changes to "earn" those phases.
