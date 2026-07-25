# Tasks: Capacity Admission Webhook

**Input**: Design documents from `specs/001-capacity-admission-webhook/`

**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md,
contracts/, quickstart.md, `.specify/memory/constitution.md`

**Branch**: `spec/capacity-admission-webhook` (per constitution v2.2.0 branch-and-PR rule)

**Tests**: Tests are REQUIRED for all phases — the constitution (Principle VIII,
NON-NEGOTIABLE) mandates strict TDD (Red-Green-Refactor). Every implementation
task is preceded by its test task; tests are watched to fail before
implementation begins. The constitution supersedes the template's "tests are
optional" default.

**Organization**: Tasks are grouped by user story to enable independent
implementation and testing of each story. The three components (Node Capacity
Controller, Allocation Controller, Admission Webhook) are distributed across
stories: the full working pipeline lands in US1, the observability layer in US2,
and the fail-safe error paths in US3.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/`, `deploy/` at repository root (per plan.md)

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization, dependency manifest, formatting configuration,
and source-tree scaffolding.

- [ ] T001 Create `Cargo.toml` with all dependencies (kube 4.2.0 features `runtime,derive,client,rustls-tls`; k8s-openapi 0.28.0 features `latest,schemars`; schemars 1.0; tokio 1.x `full`; axum + hyper 1.x; rustls 0.23; serde/serde_json 1.x; tracing + tracing-subscriber 0.1/0.3; prometheus 0.14), `rust-version = "1.89"`, `edition = "2024"`, binary name `capacity-admission-webhook`
- [ ] T002 [P] Create `.editorconfig` at repo root — UTF-8, LF line endings, final newline, no trailing whitespace; `*.rs` 4-space indent; `*.toml`/`*.yaml`/`*.yml`/`*.json` 2-space indent; `*.sh` 4-space indent; `*.feature` 2-space indent; `Makefile` tab indent (per constitution Principle IX)
- [ ] T003 [P] Create `rustfmt.toml` — canonical Rust formatting (edition 2024, match arm style, import granularity)
- [ ] T004 Create `src/` module structure: `main.rs`, `lib.rs`, `crd/mod.rs`, `controllers/mod.rs`, `webhook/mod.rs`, `resources/mod.rs`, `metrics.rs`, `config.rs` — each with module declarations and minimal stubs so `cargo build` succeeds
- [ ] T005 [P] Create `deploy/` directory with placeholder files: `deployment.yaml`, `webhook-config.yaml`, `rbac.yaml`, `crds.yaml`, `cert-setup.yaml`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can
be implemented.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T006 [P] Write unit tests for Kubernetes resource.Quantity parsing in `src/resources/quantity.rs` (`#[cfg(test)]` module) — covering: CPU `"500m"`→500, `"2"`→2000, `"0"`; memory `"1Gi"`→1073741824, `"512Mi"`, `"1G"`→1000000000, `"1073741824"` (bare bytes); invalid inputs return error; boundary values (zero, max i64)
- [ ] T007 Implement resource.Quantity parser in `src/resources/quantity.rs` — `parse_cpu(&str) -> Result<i64>` (milli-CPUs) and `parse_memory(&str) -> Result<i64>` (bytes); handle SI suffixes (k/M/G/T/P, powers of 1000) and IEC suffixes (Ki/Mi/Gi/Ti/Pi, powers of 1024) per data-model.md §5
- [ ] T008 [P] Write unit tests for pod resource-request extraction in `src/resources/quantity.rs` (`#[cfg(test)]` module) — covering: pod with explicit requests, pod with limits-but-no-requests (defaults `requests=limits` per FR-005), pod with no resources (→0), init containers (`max(sum(regular), max(init))` per allocation-crd.md §Defaulting), multi-container pod summing
- [ ] T009 Implement pod resource-request extraction in `src/resources/quantity.rs` — `extract_pod_requests(&PodSpec) -> (cpu_milli: i64, memory_bytes: i64)`; iterate `containers` + `initContainers`; apply Kubernetes defaulting convention; return effective request as `max(sum(regular), max(init))`
- [ ] T010 [P] Implement `ClusterCapacity` CRD type in `src/crd/cluster_capacity.rs` — `#[derive(CustomResource)]` with `group="emergency-ration.dev"`, `version="v1"`, `kind="ClusterCapacity"`, `namespaced=false`, `status="ClusterCapacityStatus"`, `shortname="cc"`; empty `ClusterCapacitySpec`; `ClusterCapacityStatus` with `total_allocatable_cpu_milli: i64`, `total_allocatable_memory_bytes: i64`, `node_count: i32`, `last_updated: String` (per data-model.md §1)
- [ ] T011 [P] Implement `Allocation` CRD type in `src/crd/allocation.rs` — `#[derive(CustomResource)]` with `group="emergency-ration.dev"`, `version="v1"`, `kind="Allocation"`, `namespaced=false`, `status="AllocationStatus"`, `shortname="alloc"`; `AllocationSpec` with `budget_percent: i32` (0–100, `#[schemars(range(min=0,max=100))]`); `AllocationStatus` with `allocated_cpu_milli`, `allocated_memory_bytes`, `ceiling_cpu_milli`, `ceiling_memory_bytes`, `utilization_percent_cpu: f64`, `utilization_percent_memory: f64`, `last_updated: String` (per data-model.md §2)
- [ ] T012 [P] Implement configuration module in `src/config.rs` — `Config` struct parsed from CLI flags / env vars: `port` (default 8443), `tls_cert_file` (default `/tls/tls.crt`), `tls_key_file` (default `/tls/tls.key`), `decision_timeout_ms` (default 100), `capacity_freshness_timeout_secs` (default 30), `namespace` (default `capacity-admission`); use `clap` or manual `std::env` parsing
- [ ] T013 [P] Implement `AdmissionError` enum and fail-closed response mapping in `src/webhook/error.rs` — variants: `OverBudget`, `CapacityDataStale`, `CapacityDataMissing`, `DeserialisationFailure`, `QuantityParseFailure`, `Timeout`, `InternalError`, `Unknown`; `impl From<AdmissionError> for AdmissionResponse` always sets `allowed: false` with the correct `status.code` and `status.message` per contracts/admission-webhook.md §Error Path Matrix
- [ ] T014 [P] Initialise tracing subscriber (`tracing_subscriber::fmt()` with structured fields) and Prometheus default registry in `src/main.rs` startup — configure log level from env (`RUST_LOG`), initialise before any component starts

**Checkpoint**: Foundation ready — quantity parsing, CRD types, config, error
framework, and tracing are in place. User story implementation can now begin.

---

## Phase 3: User Story 1 — Budget Enforcement (Priority: P1) 🎯 MVP

**Goal**: A cluster operator submits a pod; the system admits it if its resource
requests fit within the remaining budget, or rejects it with a clear message
citing the violated resource and budget figures. This delivers the full working
pipeline: Node Capacity Controller → Allocation Controller → Admission Webhook.

**Independent Test**: Submit a pod whose requests fit within the budget and
observe it admitted; submit a pod whose requests exceed the budget and observe
it rejected with a message citing the violated resource and the budget figures
(spec acceptance scenarios 1–5).

### Tests for User Story 1

> **NOTE**: Write these tests FIRST (TDD RED), ensure they FAIL before implementation.

- [ ] T015 [P] [US1] Write integration tests for budget enforcement in `tests/integration/budget_enforcement.rs` — using `tower-test` mock; pre-populate Allocation reflector store with fixture state (allocated=70000m CPU, ceiling=80000m); cover: (1) pod under ceiling → admitted, (2) pod over ceiling → denied with figures, (3) pod exactly at ceiling → admitted (inclusive), (4) pod with zero requests → admitted, (5) pod update evaluated as delta (spec scenarios 1–5)
- [ ] T016 [P] [US1] Write BDD feature file `tests/bdd/features/budget_enforcement.feature` + step definitions in `tests/bdd/steps/budget_steps.rs` — Given/When/Then for each acceptance scenario from spec.md US1; `World` holds a mocked Allocation store + admission handler

### Implementation for User Story 1

- [ ] T017 [P] [US1] Implement budget calculation function in `src/webhook/admission.rs` — `check_budget(allocated: (i64,i64), pod_request: (i64,i64), ceiling: (i64,i64)) -> AdmissionVerdict`; pure function implementing data-model.md §4 algorithm; returns `Admit` if `allocated+request ≤ ceiling` for both resources (inclusive), `Deny` with violated-resource list + figures if any over; both resources checked independently
- [ ] T018 [P] [US1] Implement Node Capacity Controller in `src/controllers/node_capacity.rs` — `kube::runtime::reflector` on `Api::<Node>::all(client)`; on every node event, re-sum `.status.allocatable` (CPU→milli, memory→bytes) across all cached nodes; patch `ClusterCapacity` CRD `.status` subresource via `patch_status`; update `last_updated` timestamp (per contracts/clustercapacity-crd.md §Controller Behaviour)
- [ ] T019 [P] [US1] Implement Allocation Controller in `src/controllers/allocation.rs` — `kube::runtime::reflector` on `Api::<Pod>::all(client)` + watch `ClusterCapacity` CRD; on pod/capacity/budget event: sum resource requests across non-terminal pods (Pending/Running/Unknown, excluding Failed/Succeeded per allocation-crd.md §Pod Phase Filtering), apply defaulting convention, compute ceiling from supply+budget, patch `Allocation` CRD `.status`
- [ ] T020 [US1] Implement admission webhook HTTP handler in `src/webhook/handler.rs` — axum `POST /validate` route; deserialise `AdmissionReview`; read cached `Allocation` status from reflector `Store`; extract pod resource requests (T009); call `check_budget` (T017); construct `AdmissionReview` response with echoed `uid`, `allowed`, and `status.message`; wire `GET /healthz` readiness endpoint (depends on T017)
- [ ] T021 [US1] Wire all components in `src/main.rs` — spawn Node Capacity Controller + Allocation Controller as `kube::runtime::Controller` tasks on shared `tokio` runtime; start Allocation CRD reflector for webhook cache; start `axum` HTTPS server (rustls) on configured port with `/validate` + `/healthz` routes; gracefully shut down on SIGTERM (depends on T018, T019, T020)
- [ ] T022 [P] [US1] Create CRD manifests in `deploy/crds.yaml` — `ClusterCapacity` + `Allocation` CustomResourceDefinition manifests (can be generated via `ClusterCapacity::crd()` / `Allocation::crd()` at build time or committed as static YAML); match schemas in data-model.md §1–2
- [ ] T023 [P] [US1] Create RBAC manifests in `deploy/rbac.yaml` — `ServiceAccount`, `ClusterRole` (get/list/watch on `nodes`+`pods`; get/list/watch/update/patch on `clustercapacities`+`allocations` status), `ClusterRoleBinding` (per contracts/clustercapacity-crd.md + allocation-crd.md §RBAC)

**Checkpoint**: User Story 1 is fully functional and independently testable.
The cluster is protected from overcommit. This is the MVP — deploy/demo ready.

---

## Phase 4: User Story 2 — Capacity Awareness (Priority: P2)

**Goal**: Every admission decision is observable with capacity figures.
Operators can query capacity utilisation via metrics and CRD status at any time.
Rejection messages carry actionable figures (current, requested, projected,
ceiling).

**Independent Test**: Submit pods that trigger both an admit and a deny; observe
each decision is accompanied by the capacity state used. Query the `/metrics`
endpoint and confirm capacity utilisation figures are present (spec acceptance
scenarios 1–4).

### Tests for User Story 2

- [ ] T024 [P] [US2] Write integration tests for capacity awareness in `tests/integration/capacity_awareness.rs` — verify: (1) structured log entries contain all required fields (workload, decision, resource_type, allocated, requested, projected, ceiling, budget_percent, freshness_seconds per contracts/admission-webhook.md §Logging), (2) rejection messages contain actionable figures, (3) metrics registry exposes verdict counters + allocation gauges
- [ ] T025 [P] [US2] Write BDD feature file `tests/bdd/features/capacity_awareness.feature` + step definitions in `tests/bdd/steps/capacity_steps.rs` — Given/When/Then for spec.md US2 acceptance scenarios; assert log fields and metric values from the mocked `World`

### Implementation for User Story 2

- [ ] T026 [US2] Add structured `tracing` log spans to webhook handler in `src/webhook/handler.rs` — every admission decision emits a span with all fields from contracts/admission-webhook.md §Logging Contract (workload, operation, decision, reason, resource_type, allocated, requested, projected, ceiling, budget_percent, freshness_seconds, latency_ms); INFO on admit, WARN on deny, ERROR on failure
- [ ] T027 [US2] Implement Prometheus metric definitions and `/metrics` endpoint in `src/metrics.rs` — register: `capacity_admission_verdicts_total{resource,verdict}` (Counter), `capacity_admission_decision_duration_seconds` (Histogram), `capacity_admission_capacity_freshness_seconds` (Gauge), `capacity_admission_allocation_ratio{resource}` (Gauge), `capacity_admission_total_allocatable{resource}` (Gauge), `capacity_admission_current_allocation{resource}` (Gauge), `capacity_admission_ceiling{resource}` (Gauge); expose via `GET /metrics` on the axum server (per data-model.md §Metrics)
- [ ] T028 [US2] Format rejection messages with capacity figures in `src/webhook/error.rs` — `OverBudget` variant message format: `"CPU budget exceeded: allocated {A}m, requested {R}m, projected {P}m, ceiling {C}m"` (and memory equivalent); both-resources-over case lists both, newline-separated (per contracts/admission-webhook.md §Error Path Matrix)
- [ ] T029 [US2] Wire capacity gauges from CRD status in `src/metrics.rs` — update `total_allocatable`, `current_allocation`, `ceiling`, and `allocation_ratio` gauges from the cached `ClusterCapacity` + `Allocation` CRD status whenever the reflector store updates; ensures metrics match the state used by the most recent admission decision (SC-003)

**Checkpoint**: Every decision is observable. Metrics endpoint and structured
logs are live. Rejection messages are self-explanatory.

---

## Phase 5: User Story 3 — Fail-Safe Operation (Priority: P3)

**Goal**: When the system cannot authoritatively verify that a workload fits —
for any reason — the admission request is rejected, never silently admitted.
Every failure path maps to a declared outcome; there is no "undefined" category.

**Independent Test**: Simulate each failure condition (capacity data stale, CRD
missing, component down, timeout, malformed AdmissionReview) and assert each
results in `allowed: false` with a logged reason (spec acceptance scenarios 1–5).

### Tests for User Story 3

- [ ] T030 [P] [US3] Write integration tests for fail-safe paths in `tests/integration/fail_safe.rs` — using `tower-test` mock; assert `allowed: false` for each: (1) stale capacity data (lastUpdated beyond threshold), (2) Allocation CRD not populated, (3) ClusterCapacity CRD missing, (4) malformed AdmissionReview (deserialisation failure), (5) decision timeout exceeded, (6) unknown error catch-all (per quickstart.md Scenario 3)
- [ ] T031 [P] [US3] Write BDD feature file `tests/bdd/features/fail_safe.feature` + step definitions in `tests/bdd/steps/fail_safe_steps.rs` — Given/When/Then for spec.md US3 acceptance scenarios; each failure path asserts `allowed: false` + correct reason in the response

### Implementation for User Story 3

- [ ] T032 [US3] Implement capacity data freshness check in `src/webhook/handler.rs` — before the budget check, compare `Allocation.status.last_updated` to current time; if older than `--capacity-freshness-timeout` (default 30s), return `AdmissionError::CapacityDataStale` → deny with message `"capacity data unavailable: last refresh {T}s ago exceeds {threshold}s threshold"` (per contracts/admission-webhook.md §Error Path Matrix)
- [ ] T033 [US3] Handle AdmissionReview deserialisation failure in `src/webhook/handler.rs` — if `serde_json::from_slice` fails on the request body, return `AdmissionError::DeserialisationFailure` → deny with `status.code: 400` and message `"admission request malformed: {parse error}"` (per data-model.md §Admission Decision States path 1)
- [ ] T034 [US3] Handle resource quantity parse failure in `src/webhook/handler.rs` — if `extract_pod_requests` encounters an unparseable quantity string in the pod spec, return `AdmissionError::QuantityParseFailure` → deny with `status.code: 400` and message `"cannot parse resource quantity in pod spec: {field}={value}"` (per data-model.md §Admission Decision States path 3)
- [ ] T035 [US3] Implement decision timeout in `src/webhook/handler.rs` — wrap the admission decision in a `tokio::time::timeout` of `--decision-timeout` (default 100ms); on elapsed, return `AdmissionError::Timeout` → deny with `status.code: 500` and message `"admission decision timed out after {timeout}ms"` (per contracts/admission-webhook.md §Error Path Matrix)
- [ ] T036 [US3] Implement `catch_unwind` guard around admission decision in `src/webhook/handler.rs` — wrap the entire decision logic in `std::panic::catch_unwind`; on `Err`, return `AdmissionError::InternalError` → deny with `status.code: 500` and message `"internal error: panic in admission handler"` (per R12 fail-closed implementation details)
- [ ] T037 [US3] Implement unknown-error catch-all in `src/webhook/handler.rs` — the top-level error handler maps any `Err` not matching a known `AdmissionError` variant to `AdmissionError::Unknown` → deny with `status.code: 500` and message `"internal error: {error description}"`; guarantees Principle III's "no third category" (per contracts/admission-webhook.md §Error Path Matrix final row)

**Checkpoint**: All failure paths reject. The webhook is fail-closed under every
enumerated condition. Zero paths admit under degraded knowledge.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Deployment manifests, CI, performance validation, and final
quality gate.

- [ ] T038 [P] Create `deploy/deployment.yaml` — `Deployment` (2 replicas per R8) + `Service` (port 8443) in namespace `capacity-admission`; resource requests `< 256Mi` memory, `< 500m` CPU (SC-006); volume mounts for TLS cert/key from Secret
- [ ] T039 [P] Create `deploy/webhook-config.yaml` — `ValidatingWebhookConfiguration` per contracts/admission-webhook.md §Webhook Configuration Contract: `failurePolicy: Fail`, `sideEffects: None`, `timeoutSeconds: 5`, `matchPolicy: Exact`, rules for `pods` CREATE+UPDATE, `namespaceSelector` excluding `capacity-admission`+`kube-system`+`kube-public`
- [ ] T040 [P] Create `deploy/cert-setup.yaml` — cert-manager `Certificate` resource (default path) with documented fallback to a manually-provided TLS Secret for clusters without cert-manager (per R9 TLS provisioning decision)
- [ ] T041 Create CI workflow in `.github/workflows/ci.yml` — matrix job across Kubernetes 1.34/1.35/1.36 (per R3 + Principle VII); each: `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test` (unit+integration) + E2E via `k3d`/`kind` cluster (`#[ignore]` tests with `--ignored` flag)
- [ ] T042 Write performance benchmark test in `tests/integration/performance.rs` — measure p50/p99 admission decision latency over 10,000 iterations with pre-populated cache; assert p99 < 100ms, p50 < 50ms (SC-005); print results with `--nocapture`
- [ ] T043 Run all validation scenarios from `specs/001-capacity-admission-webhook/quickstart.md` — execute each scenario (budget enforcement, capacity awareness, fail-safe, performance); verify all expected outputs match; document any deviations
- [ ] T044 Run final quality gate — `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (unit + integration + BDD) all green; verify no `#[ignore]` E2E test is accidentally included in default `cargo test`; confirm `.editorconfig` compliance across all file types (constitution Development Workflow quality gate)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup (Phase 1) completion — BLOCKS all user stories
- **User Stories (Phases 3–5)**: All depend on Foundational (Phase 2) completion
  - US1 (Phase 3) is the MVP — no dependency on other stories
  - US2 (Phase 4) layers on US1's webhook handler (adds observability to existing decisions)
  - US3 (Phase 5) layers on US1's webhook handler (adds error-path handling)
  - US2 and US3 are independent of each other and can proceed in parallel after US1
- **Polish (Phase 6)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Depends on Foundational only — no dependencies on other stories
- **User Story 2 (P2)**: Depends on US1's webhook handler (T020) for the decision path it instruments; independently testable via mocked handler
- **User Story 3 (P3)**: Depends on US1's webhook handler (T020) for the decision path it guards; independently testable via mocked handler

### Within Each User Story

- Tests (TDD RED) MUST be written and FAIL before implementation (constitution Principle VIII)
- Budget calculation / pure logic before I/O-bound components
- Controllers before main.rs wiring
- Webhook handler before deployment manifests
- Story complete before moving to next priority

### Parallel Opportunities

- **Phase 1**: T002, T003, T005 are independent files (all `[P]`)
- **Phase 2**: T006+T008 (test tasks), T010+T011 (CRD types), T012+T013+T014 (config/error/tracing) are mutually independent (all `[P]`); T007 follows T006, T009 follows T008
- **Phase 3**: T015+T016 (test tasks), T017+T018+T019 (budget calc + 2 controllers), T022+T023 (manifests) are mutually independent (all `[P]`); T020 depends on T017; T021 depends on T018+T019+T020
- **Phase 4**: T024+T025 (test tasks) are parallel; T026→T027→T028→T029 are sequential (all touch webhook/metrics)
- **Phase 5**: T030+T031 (test tasks) are parallel; T032–T037 are sequential (all modify `handler.rs`)
- **Phase 6**: T038+T039+T040 (manifests) are independent files (all `[P]`)

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together (TDD RED phase):
Task: "Integration tests for budget enforcement in tests/integration/budget_enforcement.rs"
Task: "BDD feature for budget enforcement in tests/bdd/features/budget_enforcement.feature"

# Launch all independent implementation tasks together (TDD GREEN phase):
Task: "Budget calculation in src/webhook/admission.rs"
Task: "Node Capacity Controller in src/controllers/node_capacity.rs"
Task: "Allocation Controller in src/controllers/allocation.rs"
Task: "CRD manifests in deploy/crds.yaml"
Task: "RBAC manifests in deploy/rbac.yaml"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL — blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (`cargo test --test integration budget_enforcement`)
5. Deploy/demo if ready — the cluster is protected from overcommit

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (**MVP!**)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Test independently → Deploy/Demo
5. Complete Polish phase → CI green across N-2 matrix

### Single-Developer Strategy (Sequential)

1. Complete Setup + Foundational
2. US1 (P1) → validate MVP
3. US2 (P2) → validate observability
4. US3 (P3) → validate fail-safe
5. Polish → CI + performance + final gate

---

## Notes

- [P] tasks = different files, no dependencies on incomplete tasks in the same phase
- [Story] label maps task to specific user story for traceability
- Each user story is independently completable and testable
- TDD is NON-NEGOTIABLE (constitution Principle VIII) — verify tests fail before implementing
- Commit after each task or logical group; merge to `main` only via pull request (constitution v2.2.0)
- Stop at any checkpoint to validate a story independently
- The webhook's hot path reads from an in-process reflector cache — no network calls, no I/O — so the p99 < 100ms target is achievable by design
