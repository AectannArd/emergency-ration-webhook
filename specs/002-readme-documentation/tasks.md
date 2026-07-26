# Tasks: README Documentation

**Input**: Design documents from `specs/002-readme-documentation/`

**Prerequisites**: plan.md (required), spec.md (required), research.md,
data-model.md, quickstart.md, `.specify/memory/constitution.md`

**Branch**: `spec/readme-documentation` (per constitution v2.3.0 branch-and-PR
rule)

**Tests**: Accuracy validation against source code IS REQUIRED (constitution
Principle VIII, adapted for documentation per plan.md). The quickstart.md
validation scenarios (VR-001–VR-008) serve as the test spec — each README
section is verified against its source file before the task is marked complete.
FR-012 (accuracy) is the hard gate.

**Organization**: Tasks are grouped by user story to enable independent
implementation and validation of each story. Each user story produces one
major README section. The deliverable is a single file (`README.md`), so tasks
are sequenced to build it section-by-section, with accuracy validation after
each section.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different sections, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)
- Include exact file paths and source-of-truth references in descriptions

## Path Conventions

- **Deliverable**: `README.md` at repository root
- **Design docs**: `specs/002-readme-documentation/` (research.md, data-model.md,
  quickstart.md — READ-ONLY reference)
- **Source of truth**: `src/config.rs`, `src/metrics.rs`, `src/crd/*.rs`,
  `src/main.rs`, `src/webhook/*.rs`, `deploy/*.yaml`,
  `.github/workflows/ci.yml`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Read the authoritative source files and lock the reference values
the README must reproduce. All values are already enumerated in research.md —
this phase confirms them against source before writing begins.

- [ ] T001 Read `src/config.rs` and confirm the 7-row configuration table in `specs/002-readme-documentation/research.md` §R1 — verify every flag name, env-var name, type, and default matches the `Config` struct, `impl Default`, and `resolve()` calls exactly
- [ ] T002 [P] Read `src/crd/allocation.rs` and `src/crd/cluster_capacity.rs`; confirm the CRD field tables in research.md §R2–R3 — verify every field name (camelCase), type, unit, and constraint matches the Rust structs and `#[serde(rename_all = "camelCase")]` casing
- [ ] T003 [P] Read `src/metrics.rs` and confirm the 7-row metrics table in research.md §R5 — verify every metric name, type (counter/histogram/gauge), labels, and histogram buckets match the `Metrics::new()` registrations
- [ ] T004 [P] Read `src/main.rs`, `deploy/deployment.yaml`, and `deploy/webhook-config.yaml`; confirm the endpoints table (research.md §R4), deployment details (§R7), and namespaceSelector exclusions
- [ ] T005 [P] Read `.github/workflows/ci.yml` and confirm the Kubernetes version matrix (research.md §R8: 1.34, 1.35, 1.36)

**Checkpoint**: All reference values confirmed against source. Any discrepancy
between research.md and the actual code MUST be resolved here — the README
documents the code, not the research notes.

---

## Phase 2: Foundational (README Skeleton)

**Purpose**: Create the README file with its section structure, title, and
overview. This is the scaffold all three user stories fill.

**⚠️ CRITICAL**: No section content is written here — only the skeleton.

- [ ] T006 Create `README.md` at repo root with the section tree from `specs/002-readme-documentation/data-model.md` §1 — include the title (`# emergency-ration-webhook`), one-line description, and all `##` / `###` headings as empty placeholders. Preserve the exact section order: Overview, Quick Start, Configuration, Metrics & Observability, Failure Modes, Kubernetes Compatibility, Architecture, Development, License. Do NOT delete the existing content blindly — check `git diff` to confirm only the stub is replaced

**Checkpoint**: README skeleton exists with all headings. Each user story phase
fills its assigned sections.

---

## Phase 3: User Story 1 — Installation & Quick Start (Priority: P1) 🎯 MVP

**Goal**: An operator can follow the README quick start from clone to a running
webhook in a Kubernetes cluster.

**Independent Test**: Follow the quick start on a fresh `k3d`/`kind` cluster;
verify pods reach Ready, `/healthz` returns 200, and a test pod is
admitted/rejected per budget (quickstart.md Scenario 1).

### Accuracy Validation for User Story 1

> **NOTE**: Validate against source BEFORE writing, per Principle VIII (adapted
> for documentation — research.md is the "failing test" the README must pass).

- [ ] T007 [P] [US1] Verify deploy manifest filenames and content referenced by the quick start exist in `deploy/` — confirm `deployment.yaml`, `rbac.yaml`, `crds.yaml`, `webhook-config.yaml`, `cert-setup.yaml` all exist and the `kubectl apply` order in data-model.md §1 Quick Start subsection is correct

### Implementation for User Story 1

- [ ] T008 [US1] Write the **Overview** section in `README.md` — 2–3 paragraphs: what the webhook is (Kubernetes validating admission webhook that enforces a cluster capacity budget), why it exists (prevent overcommit of CPU/RAM), and the fail-closed guarantee. Reference `specs/001-capacity-admission-webhook/spec.md` for the full feature spec
- [ ] T009 [US1] Write the **Quick Start** section in `README.md` (the `## Quick Start` heading and all subsections from data-model.md §1) — cover: (1) Prerequisites (Rust toolchain OR pre-built image, `kubectl`, a Kubernetes cluster), (2) Build the image (`docker build` referencing `Dockerfile`), (3) Deploy to Kubernetes (`kubectl apply` in the correct order: CRDs → RBAC → TLS cert → Deployment+Service → ValidatingWebhookConfiguration), (4) Verify (pods Ready, healthz, test pod admitted). Use actual `kubectl` commands referencing the real manifest filenames in `deploy/`
- [ ] T010 [US1] Write the **TLS Provisioning** subsection in `README.md` — cover both paths: (1) cert-manager automated (reference `deploy/cert-setup.yaml` with the `cert-manager.io/inject-ca-from` annotation), (2) manual Secret (create a TLS Secret, base64-encode the CA bundle into the webhook config). Note that TLS is mandatory for the admission endpoint

**Checkpoint**: User Story 1 complete. An operator can deploy the webhook from
the README alone. This is the MVP — the cluster is protected from overcommit
and the deployment is documented.

---

## Phase 4: User Story 2 — Configuration Reference (Priority: P2)

**Goal**: An operator can find every configurable parameter with its default,
type, and effect, and adjust the budget at runtime without a restart.

**Independent Test**: Pick any flag or CRD field at random from the README;
verify its name, type, default, and effect match source (quickstart.md
Scenario 2).

### Accuracy Validation for User Story 2

- [ ] T011 [P] [US2] Cross-check the configuration table in `README.md` against `src/config.rs` — for each of the 7 rows: flag name matches a `resolve(args, "--<flag>", ...)` call, env-var matches the second arg, default matches `impl Default for Config`, type matches the struct field. This is VR-001 from quickstart.md

### Implementation for User Story 2

- [ ] T012 [US2] Write the **Configuration** section in `README.md` (the `## Configuration` heading and all subsections from data-model.md §1) — include: (1) **CLI Flags & Environment Variables** — reproduce the 7-row table from data-model.md §2 exactly (flag, env-var, type, default, description), (2) **Precedence** — state "CLI flag → environment variable → compiled default" and that unparseable values fall back to default (FR-008), (3) a note that flags are how the Deployment passes args (reference `deploy/deployment.yaml` args section)
- [ ] T013 [US2] Write the **Allocation CRD** subsection in `README.md` — reproduce the spec and status field tables from data-model.md §3 (Allocation). Document: group/version/kind (`emergency-ration.dev/v1`, `Allocation`), short name (`alloc`), scope (cluster, singleton `cluster-allocation`), `budgetPercent` (0–100), and all 7 status fields with their units. Document the runtime budget-adjustment workflow: `kubectl patch allocation cluster-allocation --type=merge -p '{"spec":{"budgetPercent":N}}'` takes effect without restart (FR-009)
- [ ] T014 [P] [US2] Write the **ClusterCapacity CRD** subsection in `README.md` — reproduce the status field table from data-model.md §3 (ClusterCapacity). Document: group/version/kind, short name (`cc`), scope (cluster, singleton `cluster-capacity`), empty spec (supply-side, controller-written), and all 4 status fields with their units
- [ ] T015 [P] [US2] Document budget edge cases in the Configuration section — note that `budgetPercent: 0` is a circuit-breaker (every pod requesting >0 is rejected) and `budgetPercent: 100` guards against physical overcommit. Reference spec.md edge cases

**Checkpoint**: User Story 2 complete. Every configuration knob is documented
and cross-checked against source.

---

## Phase 5: User Story 3 — Operations & Observability (Priority: P3)

**Goal**: An operator can monitor the webhook, interpret metrics and rejection
messages, understand the fail-closed model, and know the K8s support window.

**Independent Test**: Scrape `/metrics`; confirm all 7 metric families appear.
Trigger a denial; confirm the message and log format match the README
(quickstart.md Scenarios 3 and 4).

### Accuracy Validation for User Story 3

- [ ] T016 [P] [US3] Cross-check the metrics table in `README.md` against `src/metrics.rs` — for each of the 7 metrics: name matches `Opts::new(...)` arg, type matches constructor, labels match the label slice. This is VR-003 from quickstart.md
- [ ] T017 [P] [US3] Cross-check the endpoints table in `README.md` against `src/main.rs` and `deploy/deployment.yaml` — confirm `/validate` on HTTPS 8443, `/metrics` and `/healthz` on HTTP 9090. This is VR-004

### Implementation for User Story 3

- [ ] T018 [US3] Write the **Metrics & Observability** section in `README.md` (the `## Metrics & Observability` heading and subsections from data-model.md §1) — include: (1) **HTTP Endpoints** — reproduce the 3-row table from data-model.md §4 (endpoint, protocol, port, path, purpose), (2) **Prometheus Metrics** — reproduce the 7-row table from data-model.md §5 exactly (metric name, type, labels, description), including the histogram bucket boundaries, (3) a note that the metrics port is plaintext HTTP and should not be exposed externally without a network policy
- [ ] T019 [P] [US3] Write the **Structured Logging** subsection in `README.md` — document that the webhook uses `tracing` with structured fields; list the key fields emitted on every admission decision (workload identity, decision, resource type, capacity figures: allocated/requested/projected/ceiling). Document the `RUST_LOG` env var for log level (default `info`). Include an example structured log line
- [ ] T020 [P] [US3] Write the **Rejection Messages** subsection in `README.md` — document the format from research.md §R6: each violation names the resource, current allocation, requested increment, projected total, and ceiling. When both CPU and RAM exceed, both are reported (CPU first). Include a concrete example rejection message (e.g., "CPU would reach 85000m / 80000m")
- [ ] T021 [US3] Write the **Failure Modes** section in `README.md` (the `## Failure Modes` heading) — reproduce the 6-row failure-mode table from data-model.md §6 (condition, outcome, logged reason). Emphasise: every degradation path rejects; `failurePolicy: Fail` ensures the apiserver rejects if the webhook is unreachable; there is no "best-effort" or silent-admit path (FR-006). Reference constitution Principle I
- [ ] T022 [US3] Write the **Kubernetes Compatibility** section in `README.md` (the `## Kubernetes Compatibility` heading) — document the N-2 support window (three most recent major releases: currently 1.34, 1.35, 1.36 per research.md §R8), state that all APIs used are GA/stable, and reference the CI version matrix in `.github/workflows/ci.yml` (FR-007)

**Checkpoint**: User Story 3 complete. The webhook's observability surface,
failure model, and compatibility window are fully documented.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Architecture overview, development guide, and final accuracy
validation across the entire README.

- [ ] T023 [P] Write the **Architecture** section in `README.md` (the `## Architecture` heading) — brief (1–2 paragraphs + a text/ASCII diagram or table) describing the 3-component model: Node Capacity Controller (watches nodes → ClusterCapacity CRD), Allocation Controller (watches pods + capacity → Allocation CRD), Admission Webhook (reads Allocation CRD → admit/deny). Link to `specs/001-capacity-admission-webhook/data-model.md` for the full architecture detail. Do NOT duplicate the full spec — summarise for an operator audience
- [ ] T024 [P] Write the **Development** section in `README.md` (the `## Development` heading) — cover: (1) Build (`cargo build`), (2) Tests (`cargo test` for unit+integration, `cargo test -- --ignored` for E2E), (3) Formatting (`cargo fmt --check`, `cargo clippy -- -D warnings`), (4) Project structure (brief tree of `src/` modules). Reference the quality gate from constitution Development Workflow
- [ ] T025 [P] Write the **License** section in `README.md` — state `Apache-2.0` (per `Cargo.toml` `license` field)
- [ ] T026 Run full accuracy validation: execute all 8 verification rules (VR-001–VR-008) from `specs/002-readme-documentation/data-model.md` §7 and `specs/002-readme-documentation/quickstart.md` against the completed `README.md`. Every documented value MUST match source. Fix any discrepancy before marking complete
- [ ] T027 Verify `README.md` renders correctly — check GitHub-Flavoured Markdown: tables align, code blocks have language tags, internal links work, no broken anchors. Confirm `.editorconfig` compliance (LF endings, final newline, no trailing whitespace per constitution Principle IX)
- [ ] T028 Verify the README is the single entry point (FR-011, SC-005) — read top-to-bottom; confirm an operator can install, configure, monitor, and troubleshoot using ONLY the README. Confirm deeper material is linked, not delegated. No section says "see source code" for essential information

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately. All tasks [P].
- **Foundational (Phase 2)**: Depends on Phase 1 (reference values confirmed).
- **User Stories (Phases 3–5)**: Depend on Phase 2 (skeleton exists).
  - US1 (Quick Start) should be done first — it establishes the deployment
    context US2 and US3 reference.
  - US2 (Configuration) and US3 (Operations) can proceed after US1.
- **Polish (Phase 6)**: Depends on all user stories complete.

### Within Each User Story

1. Accuracy validation task (cross-check against source) — VR rules
2. Content writing tasks (fill the README sections)
3. Section-level review (values match source)

### Parallel Opportunities

- Phase 1: T002–T005 are all [P] (different source files)
- Phase 4: T014–T015 are [P] (different subsections, no dependency on each other)
- Phase 5: T016–T017 are [P] (different source files); T019–T020 are [P]
- Phase 6: T023–T025 are [P] (different sections)

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: confirm reference values
2. Complete Phase 2: README skeleton
3. Complete Phase 3: Quick Start section
4. **STOP and VALIDATE**: an operator can deploy from the README alone

### Incremental Delivery

1. Setup + Skeleton → Foundation ready
2. Add Quick Start → Test deploy independently → MVP!
3. Add Configuration → Test config reference independently
4. Add Operations → Test metrics/fail-closed independently
5. Polish → Full accuracy validation across all sections

---

## Notes

- This is a **single-file deliverable** (`README.md`). All tasks write to the
  same file, so tasks within a phase are NOT parallel unless they write to
  clearly separated sections (marked [P] where safe).
- **No new code, tests, or dependencies are created.** The README documents
  what already exists.
- **Accuracy is the gate** (FR-012). A beautifully written README with a wrong
  default value or a misspelled metric name is a defect. VR-001–VR-008 in
  data-model.md are the acceptance tests.
- The existing `README.md` is a 4-line stub. It is replaced entirely — but
  verify with `git diff` that only the stub content is removed, not any
  unrelated tracked file.
