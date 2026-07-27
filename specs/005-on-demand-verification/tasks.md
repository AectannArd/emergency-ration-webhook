---

description: "Task list for on-demand infrastructure verification (spec-005)"
---

# Tasks: On-Demand Infrastructure Verification

**Input**: Design documents from `/specs/005-on-demand-verification/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli.md, quickstart.md, `.specify/memory/constitution.md`

**Tests**: TDD is NON-NEGOTIABLE (Constitution Principle VIII). Every pure module gets tests written FIRST, watched to fail, then implemented. Scenario/cluster-integration tasks are tested by the tool's own execution against a real cluster.

**Organization**: Tasks are grouped by user story. Phase 1 = setup, Phase 2 = foundational infrastructure, Phase 3 = US1 (enforcement), Phase 4 = US2 (degradation), Phase 5 = US3 (machine-readable output), Phase 6 = polish.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)
- All file paths are repository-relative

## Key References (read before starting)

- **Spec**: `specs/005-on-demand-verification/spec.md` — 3 user stories, 19 FRs, 8 edge cases
- **Plan**: `specs/005-on-demand-verification/plan.md` — project structure, constitution check
- **Research**: `specs/005-on-demand-verification/research.md` — 18 decisions (R1–R18)
- **Data model**: `specs/005-on-demand-verification/data-model.md` — ScenarioResult, RunSummary, VerifyConfig, run state machine
- **CLI contract**: `specs/005-on-demand-verification/contracts/cli.md` — flags, exit codes, report formats
- **Quickstart**: `specs/005-on-demand-verification/quickstart.md` — build + run instructions
- **Existing config.rs**: `src/config.rs` — reference for hand-rolled arg parsing style
- **Existing CRD types**: `src/crd/allocation.rs`, `src/crd/cluster_capacity.rs` — imported by verify
- **Existing quantity parser**: `src/resources/quantity.rs` — reused for capacity accuracy scenario
- **Existing CI e2e**: `.github/workflows/ci.yml` — reference for readiness gates and smoke test

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Cargo configuration for the second binary target.

- [ ] T001 Add new dependencies to `Cargo.toml` `[dependencies]`: `rcgen = "0.13"`, `serde_yaml = "0.9"`, `base64 = "0.22"`. These are shared across the crate (the verify binary compiles from the same package). Verify `cargo check` passes.
- [ ] T002 Add the `erw-verify` binary target to `Cargo.toml` via `[[bin]] name = "erw-verify"` (auto-disovers `src/bin/erw-verify/main.rs`). Add `[[test]]` entries for the two unit test files: `name = "verify_report" path = "tests/verify/report.rs"` and `name = "verify_args" path = "tests/verify/args.rs"`. Verify `cargo check --bin erw-verify` compiles (empty main is fine for now).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before any scenario or report work.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Implement CLI arg parsing in `src/bin/erw-verify/args.rs`: `VerifyConfig` struct (fields: `kubeconfig: Option<PathBuf>`, `json: bool`, `keep_on_failure: bool`, `timeout_secs: u64`) with a `from_args_and_env` resolver. Precedence for `--kubeconfig`: flag > `KUBECONFIG` env > default (None signals fall-back to `Config::infer`). For `--timeout-secs`: flag > `VERIFY_TIMEOUT_SECS` env > default 120. Boolean flags `--json` / `--keep-on-failure` are present-or-absent. Match the hand-rolled style in `src/config.rs` (no clap). See research R14, data-model.md §2.
- [ ] T004 [P] [TDD] Write unit tests for CLI arg parsing in `tests/verify/args.rs`: precedence (flag > env > default), boolean flag toggling, invalid timeout value falls back to default, missing kubeconfig resolves to None. Run `cargo test --test verify_args` and WATCH tests FAIL (no implementation yet — T003's `from_args_and_env` is the target). NOTE: T003 and T004 are written as a TDD pair — write T004's tests FIRST, watch RED, then implement T003 to reach GREEN.
- [ ] T005 [P] Implement kube::Client construction in `src/bin/erw-verify/client.rs`: function `build_client(kubeconfig: Option<PathBuf>) -> Result<Client>`. When a path is given, use `Kubeconfig::read_from(path)` → `Config::from_custom_kubeconfig(kubeconfig, KubeConfigOptions::default())` → `Client::try_from(config)`. When None, use `Config::infer()` then `Client::try_from`. Call `rustls::crypto::ring::default_provider().install_default().expect("...")` as the FIRST operation (research R17 — rustls CryptoProvider gotcha). See research R1.
- [ ] T006 [P] Implement result types in `src/bin/erw-verify/scenarios/mod.rs`: `ScenarioResult` (name, group, status, duration, detail), `ScenarioGroup` enum (Enforcement, Degradation), `ScenarioStatus` enum (Pass, Fail, Skip), `RunSummary` (total, passed, failed, skipped, exit_code). See data-model.md §2 for exact field definitions.
- [ ] T007 [P] [TDD] Write unit tests for `RunSummary` exit-code derivation in `tests/verify/report.rs` (first sub-section): given a `Vec<ScenarioResult>` with all-pass → exit_code 0; with one fail → exit_code 1; with all skip → exit_code 0. Watch RED, then implement the derivation in `scenarios/mod.rs` to reach GREEN.

**Checkpoint**: Foundation ready — CLI parsing, client construction, result types, and exit-code logic all compile and pass unit tests. Scenario + report work can now begin.

---

## Phase 3: User Story 1 — Verify Enforcement on a Real Cluster (Priority: P1) 🎯 MVP

**Goal**: Install the webhook stack, run the 8 enforcement scenarios, tear down, print report.

**Independent Test**: Run `erw-verify --kubeconfig <path>` against a clean cluster. The 8 enforcement scenarios pass and the cluster is left empty.

### Tests for User Story 1 (pure modules — TDD)

- [ ] T008 [P] [US1] [TDD] Write unit tests for human-readable report rendering in `tests/verify/report.rs`: a `Vec<ScenarioResult>` with mixed pass/fail/skip renders the correct section blocks with ✓/✗/○ markers, summary line with correct counts. Watch RED, then implement rendering to GREEN.
- [ ] T009 [US1] Implement report module in `src/bin/erw-verify/report.rs`: `render_human(results: &[ScenarioResult], summary: &RunSummary) -> String` (colored terminal text per contracts/cli.md) and `derive_summary(results: &[ScenarioResult]) -> RunSummary`. Pure — no I/O. This is the GREEN to T008's RED.

### Implementation for User Story 1

- [ ] T010 [US1] Implement manifest application in `src/bin/erw-verify/setup.rs`: function `apply_manifests(client: &Client) -> Result<()>`. Embed `deploy/crds.yaml`, `deploy/rbac.yaml`, `deploy/deployment.yaml`, `deploy/webhook-config.yaml` via `include_str!`. Parse each into multi-document YAML with `serde_yaml::Deserializer`, deserialize each doc to `serde_json::Value`, derive `ApiResource` from `apiVersion`/`kind`, and apply via `Api::<Dynamic>` with `Patch::Merge`. See research R2. Follow the existing CI manifest order: namespace → RBAC → CRDs → TLS Secret → Deployment → webhook-config.
- [ ] T011 [US1] Implement TLS certificate generation + Secret creation in `src/bin/erw-verify/setup.rs`: function `create_tls_secret(client: &Client) -> Result<()>`. Use `rcgen` (CertificateParams with `SanType::DnsName` for the 3 in-cluster Service DNS names from research R3), generate self-signed cert, extract PEM cert + key, create a `kubernetes.io/tls` Secret via `Api::<Secret>::namespaced`. See research R3–R4.
- [ ] T012 [US1] Implement webhook-config caBundle injection in `src/bin/erw-verify/setup.rs`: after applying the webhook-config manifest, patch its `webhooks[0].clientConfig.caBundle` with the base64-encoded cert from T011 via `Api::<ValidatingWebhookConfiguration>::patch`. See research R2, R3.
- [ ] T013 [US1] Implement readiness waiting in `src/bin/erw-verify/setup.rs`: function `wait_for_readiness(client: &Client, timeout: Duration) -> Result<()>`. Poll `Api::<Pod>::namespaced` with `labelSelector=app=capacity-admission-webhook` until all pods Ready (phase=Running, containerStatuses[].ready=true), then poll `Api::<Allocation>::all().get("cluster-allocation")` until `status.ceilingCpuMilli > 0`. Timeout → setup error (exit 2). See research R5.
- [ ] T014 [US1] Implement cluster-cleanness pre-flight check in `src/bin/erw-verify/setup.rs`: function `check_cluster_clean(client: &Client) -> Result<()>`. List pods in `default` namespace; if any non-system pod exists, return error ("cluster is not empty — this tool requires a clean, throwaway cluster"). See research R16, FR-019.
- [ ] T015 [P] [US1] Implement teardown in `src/bin/erw-verify/teardown.rs`: function `teardown(client: &Client) -> Result<()>`. Delete in reverse dependency order: ValidatingWebhookConfiguration → Deployment → Service → TLS Secret → ClusterRoleBinding → ClusterRole → ServiceAccount → CRD instances (cluster-allocation, cluster-capacity) → CRDs → Namespace. Wait for each `.get()` to return 404 before proceeding. Collect any partial-failure errors. See research R12.
- [ ] T016 [US1] Implement scenario S1 (within-budget pod admitted) in `src/bin/erw-verify/scenarios/enforcement.rs`: create a Pod with small resource requests (cpu=10m, memory=10Mi) in `default`, assert it is created successfully (admitted). Return `ScenarioResult`. See research R6.
- [ ] T017 [US1] Implement scenario S2 (over-budget pod denied) in `src/bin/erw-verify/scenarios/enforcement.rs`: create a Pod with requests exceeding the budget (cpu=999, memory=999Gi), assert creation fails with `kube::Error::Api(e)` where `e.code == 403`, and the message contains the budget-exceeded format. See research R6.
- [ ] T018 [US1] Implement scenario S3 (budgetPercent 0 — circuit-breaker) in `src/bin/erw-verify/scenarios/enforcement.rs`: patch Allocation `spec.budgetPercent` to 0, submit a small pod, assert it is denied (ceiling is 0 → every non-zero request over budget). Restore budgetPercent to 80 after. See research R7.
- [ ] T019 [US1] Implement scenario S4 (budgetPercent 100 — physical overcommit guard) in `src/bin/erw-verify/scenarios/enforcement.rs`: patch budgetPercent to 100, submit a pod with requests exceeding total allocatable, assert denied; submit a pod within total allocatable, assert admitted. Restore budgetPercent to 80 after. See research R7.
- [ ] T020 [US1] Implement scenario S5 (runtime budget adjustment — no restart) in `src/bin/erw-verify/scenarios/enforcement.rs`: patch budgetPercent to a value that would deny the test pod, assert denial; then patch to a value that admits it, assert admission — both without restarting the webhook. See research R7.
- [ ] T021 [US1] Implement scenario S6 (dry-run mode — admit + warning) in `src/bin/erw-verify/scenarios/enforcement.rs`: patch `spec.enforcementMode` to `"dry-run"`, submit an over-budget pod, assert it is ADMITTED (not denied). Verify the dry-run path via the metrics endpoint: scrape `/metrics` via the API proxy and assert `capacity_admission_verdicts_total{verdict="dry_run_deny"}` incremented. Restore enforcementMode to enforce after. See research R8.
- [ ] T022 [US1] Implement scenario S7 (capacity tracking accuracy) in `src/bin/erw-verify/scenarios/enforcement.rs`: read `ClusterCapacity` CRD singleton status; independently list all Nodes and sum `.status.allocatable["cpu"]` and `["memory"]` using the existing `capacity_admission_webhook::resources::quantity` parser; assert the CRD status values match the computed sums. See research R9.
- [ ] T023 [US1] Implement scenario S8 (metrics + health endpoints respond) in `src/bin/erw-verify/scenarios/enforcement.rs`: reach `/healthz` and `/metrics` via the Kubernetes API proxy (`/api/v1/namespaces/capacity-admission/services/capacity-admission-webhook:metrics/proxy/<path>`); assert `/healthz` returns 200 + `ok`, `/metrics` returns valid Prometheus text containing `capacity_admission_verdicts_total`. See research R10.
- [ ] T024 [US1] Implement main orchestration in `src/bin/erw-verify/main.rs`: wire the full lifecycle — `install_default_provider()` → parse args (`args.rs`) → build client (`client.rs`) → pre-flight check (T014) → apply manifests + TLS + caBundle (T010–T012) → wait readiness (T013) → run enforcement scenarios S1–S8 (T016–T023) → teardown (T015) → render report (`report.rs`) → print + exit code. Handle `--keep-on-failure` (skip teardown on scenario fail). Follow the run state machine in data-model.md §1. Initialize `tracing_subscriber` for structured logging.

**Checkpoint**: US1 is functional — `erw-verify` runs against a clean cluster, executes the 8 enforcement scenarios, tears down, and prints a human-readable report. This is the MVP.

---

## Phase 4: User Story 2 — Verify Fail-Closed Paths Under Active Degradation (Priority: P2)

**Goal**: Actively degrade the running webhook and verify each fail-closed path rejects on real infrastructure.

**Independent Test**: Scenarios S9–S11 run after US1's setup phase, each degrading the webhook, asserting rejection, then restoring health.

### Implementation for User Story 2

- [ ] T025 [US2] Implement scenario S9 (kill webhook pods → admission rejected) in `src/bin/erw-verify/scenarios/degradation.rs`: `Api::<Pod>::delete_collection` with the `app=capacity-admission-webhook` label selector; submit a pod; assert the API server rejects it (webhook unreachable, `failurePolicy: Fail` → the apiserver itself rejects). Wait for the Deployment to recreate pods and reach Ready before returning. See research R11.
- [ ] T026 [US2] Implement scenario S10 (delete CRD instances → admission rejected) in `src/bin/erw-verify/scenarios/degradation.rs`: delete the `cluster-capacity` and `cluster-allocation` singleton instances; submit a pod; assert rejection (`capacity_data_missing` — the webhook cannot verify the budget). Wait for controllers to auto-recreate and repopulate singletons (poll for non-zero ceiling) before returning. See research R11.
- [ ] T027 [US2] Implement scenario S11 (stale capacity data → admission rejected) in `src/bin/erw-verify/scenarios/degradation.rs`: patch the Allocation `status.lastUpdated` to a timestamp older than the freshness timeout (30s); submit a pod; assert rejection (`capacity_data_stale`). Wait for the Allocation Controller to re-write a fresh `lastUpdated` before returning. See research R11.
- [ ] T028 [US2] Integrate degradation scenarios into main orchestration in `src/bin/erw-verify/main.rs`: after US1 enforcement scenarios (S1–S8), run S9–S11. Each degradation scenario restores health before the next runs. The teardown at the end cleans up everything regardless. Update the report to include the degradation scenario group.

**Checkpoint**: US2 is functional — the tool now runs all 11 scenarios (8 enforcement + 3 degradation) and reports on all fail-closed paths.

---

## Phase 5: User Story 3 — Machine-Readable Output for Automation (Priority: P3)

**Goal**: `--json` flag emits structured JSON; exit codes are machine-consumable.

### Tests for User Story 3 (pure module — TDD)

- [ ] T029 [P] [US3] [TDD] Write unit tests for JSON report rendering in `tests/verify/report.rs` (new sub-section): given a `Vec<ScenarioResult>`, the JSON output is valid JSON with the exact schema from contracts/cli.md (cluster, started, duration_secs, scenarios[], summary{}, exit_code). Parse the output with `serde_json::from_str` and assert field values. Watch RED, then implement to GREEN.

### Implementation for User Story 3

- [ ] T030 [US3] Implement JSON report rendering in `src/bin/erw-verify/report.rs`: `render_json(results: &[ScenarioResult], summary: &RunSummary, cluster_url: &str, started: DateTime, duration: Duration) -> String`. Matches the JSON schema in contracts/cli.md. Pure — no I/O. This is the GREEN to T029's RED.
- [ ] T031 [US3] Wire `--json` flag into main orchestration in `src/bin/erw-verify/main.rs`: when `config.json` is true, emit `render_json(...)` to stdout instead of `render_human(...)`. When `--json` is set and setup fails before scenarios run, print the error to stderr and exit 2 (no JSON emitted — per contracts/cli.md Error Output section).

**Checkpoint**: US3 is functional — the tool supports both human-readable and JSON output with correct exit codes.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, quality gate, and validation.

- [ ] T032 [P] Add `erw-verify` documentation section to `README.md`: tool overview, throwaway-cluster requirement, CLI flags table (from contracts/cli.md), exit codes table, scenario inventory (11 scenarios), usage examples (human + JSON + keep-on-failure), and a link to `specs/005-on-demand-verification/quickstart.md`. This is a Constitution Principle X obligation — the feature is not complete without it.
- [ ] T033 [P] Verify `.editorconfig` compliance for all new files: `src/bin/erw-verify/**/*.rs` (4-space indent), `tests/verify/*.rs` (4-space), all new `.md` files (2-space, LF endings). Run `editorconfig-checker` if available locally; CI's `editorconfig` job will enforce it.
- [ ] T034 Run quality gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (unit + integration + BDD + the new `verify_report` / `verify_args` tests). All must pass — Constitution Principle XI.
- [ ] T035 Run `cargo build --bin erw-verify --release` to confirm the binary compiles in release mode (catches any dependency issues not surfaced in debug check).
- [ ] T036 Run quickstart validation: build the binary, build the Docker image, load into a `kind` cluster, run `erw-verify --kubeconfig <kind-kubeconfig>` and confirm all 11 scenarios pass and the cluster is left clean. This is the end-to-end validation per `specs/005-on-demand-verification/quickstart.md`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately.
- **Phase 2 (Foundational)**: Depends on Phase 1 — BLOCKS all user stories.
- **Phase 3 (US1)**: Depends on Phase 2. This is the MVP.
- **Phase 4 (US2)**: Depends on Phase 2 + Phase 3 (degradation scenarios run after enforcement scenarios, sharing setup/teardown).
- **Phase 5 (US3)**: Depends on Phase 2 (JSON rendering is pure; wiring needs US1's main.rs).
- **Phase 6 (Polish)**: Depends on all user stories being complete.

### Within Each User Story

- TDD pairs (T003↔T004, T007, T008↔T009, T029↔T030): write the test FIRST, watch RED, then implement GREEN.
- Setup/teardown infrastructure (T010–T015) before scenarios (T016–T023) — scenarios depend on the cluster being set up.
- Scenarios are largely independent within US1 (each creates its own test pods), but S3–S6 patch the shared Allocation singleton, so run sequentially to avoid interference.

### Parallel Opportunities

- Phase 2: T003, T005, T006 can run in parallel (different files). T004/T007 are their TDD test pairs.
- Phase 3: T015 (teardown) is independent of scenarios and can be written in parallel with setup tasks.
- Phase 5: T032 (README) and T033 (editorconfig) are parallel-safe.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 (Cargo.toml).
2. Complete Phase 2 (foundational modules + TDD).
3. Complete Phase 3 (US1: 8 enforcement scenarios + report + orchestration).
4. **STOP and VALIDATE**: run `erw-verify` against a clean cluster, confirm 8 scenarios pass, cluster left clean.

### Incremental Delivery

5. Add Phase 4 (US2: 3 degradation scenarios) → validate all 11 pass.
6. Add Phase 5 (US3: JSON output) → validate `--json` mode.
7. Complete Phase 6 (polish: README, quality gate, quickstart validation).

---

## Notes

- The planning agent does NOT write implementation code. All tasks here are for Claude Code on the VM (implementation agent) via the spec-driven PR workflow. Branch: `spec/on-demand-verification`.
- TDD (Principle VIII) is NON-NEGOTIABLE: every pure module gets a test written first, watched to fail, then implemented.
- README documentation (Principle X) is a deliverable, not an afterthought — T032 is a blocking task, not a nice-to-have.
- CI-green (Principle XI) must hold: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` all pass before the PR is mergeable.
- The verify binary must call `rustls::crypto::ring::default_provider().install_default()` first (research R17) — the same gotcha as the webhook binary.
- Use `Patch::Merge` (not `Patch::Apply`) for all Allocation spec patches (research R7). For status patches, wrap in `{"status": ...}` envelope (kube-rs Patch::Merge status gotcha from project memory).
