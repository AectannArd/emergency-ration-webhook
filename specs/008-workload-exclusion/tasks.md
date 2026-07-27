# Tasks: Workload Exclusion Policy

**Input**: Design documents from `/specs/008-workload-exclusion/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/admission-exclusion.md, `.specify/memory/constitution.md`

**Tests**: This feature is admission-core logic. Per Constitution Principle VIII (Test-First, NON-NEGOTIABLE) and Principle VI (integration coverage), **every** task below writes its test FIRST, watches it fail for the right reason, then implements the minimal code to pass. Red-Green-Refactor per behaviour, end-to-end — no pile-of-tests then pile-of-impl.

**Organization**: Phases map to the data-model layers. US1/US2/US3 share a single exemption insertion point in `evaluate()` (the `check_exemption` pure function handles all three reasons via OR semantics), so the foundational + insertion work is front-loaded; the user-story phases then add integration + BDD coverage for each reason and the OR combination.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story (US1 namespace, US2 priority class, US3 combined OR, or SHARED)

## Key design anchors (do not deviate)

- The exemption check is a **single** `check_exemption()` call inserted in `evaluate()` AFTER the `Allocation` singleton AND its status are found, but BEFORE the freshness check and budget check (data-model §3.1).
- **Fail-closed paths reject before the exemption check**: missing allocation, missing status, stale data, timeout, panic (task constraints). Cold-start self-exemption at the truly-absent-Allocation layer is the apiserver `namespaceSelector`'s job (FR-009 / research R4); the webhook layer exempts its own namespace once the Allocation is cached (FR-007).
- `check_exemption` order: webhook namespace (FR-007) → `excludedNamespaces` → `excludedPriorityClasses`; first match wins (data-model §3.2).
- Priority class is a **string match** on `pod.spec.priorityClassName` — NO PriorityClass resource watch (research R3).
- Excluded pods are **still counted** in allocation accounting — the Allocation Controller is unchanged (research R5).
- `Exempt` verdict → `allowed: true`, no warnings; bumps `capacity_admission_exemptions_total{reason}` (NOT the verdicts counter); latency still observed (data-model §3.3/§4.2).
- `--namespace`/`NAMESPACE` config is RETAINED (FR-010) — it seeds `AppState.webhook_namespace`.

---

## Phase 1: Setup

**Purpose**: Confirm the starting point is green before any change.

- [x] T000 Baseline quality gate passes on `spec/008-workload-exclusion` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`). **DONE** — baseline green before this task list was implemented.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The pure data + logic every user story depends on. No story work can begin until this phase is complete.

**⚠️ CRITICAL**: US1/US2/US3 all build on `AllocationSpec` carrying the new fields, the `check_exemption` pure function, the `Exempt` verdict, the `AppState.webhook_namespace` thread, and the exemptions metric.

### T001 [SHARED] Add `excludedNamespaces` + `excludedPriorityClasses` to `AllocationSpec`

File: `src/crd/allocation.rs`

- **RED first**: write tests (in the file's `#[cfg(test)]` block) that:
  - construct an `AllocationSpec` with the two new `Option<Vec<String>>` fields and assert `serde_json` round-trips them as camelCase `excludedNamespaces` / `excludedPriorityClasses` (array of strings);
  - deserialise a pre-spec-008 Allocation JSON (fields absent) into `AllocationSpec` and assert both fields are `None` (backward-compatible default — FR-004);
  - assert the generated CRD schema exposes both as `type: array` of `string`, nullable, and NOT in the `spec.required` list (FR-004).
- **GREEN**: add the two fields with doc comments matching data-model §1. Run the new tests; watch the existing suite still compile-pass once all construction sites are updated (see T002).
- **REFACTOR**: keep the doc comments concise and consistent with `enforcement_mode`.

### T002 [SHARED] Update all `AllocationSpec { ... }` construction sites for the new fields

Files: `src/webhook/handler.rs` (test fixtures), `src/controllers/allocation.rs` (`default_allocation_singleton` + tests), `tests/integration/{capacity_awareness,budget_enforcement,fail_safe,performance,dry_run}.rs`, `tests/bdd/steps/{fail_safe_steps,dry_run_steps,capacity_steps}.rs`.

- Set the two new fields to `None` at every construction site (no behaviour change — these are the backward-compatible defaults). `default_allocation_singleton()` in particular seeds both as `None` (data-model §7; the controller is otherwise UNCHANGED).
- This is the mechanical tail of T001 so the crate compiles. No new tests; the existing suite must stay green.

### T003 [SHARED] Add `ExemptionReason` enum + pure `check_exemption()` function

File: `src/crd/allocation.rs` (pure helper operating on `&AllocationSpec`, analogous to `resolve_enforcement_mode`); re-export from `src/crd/mod.rs`.

- **RED first**: write the full unit test matrix from data-model §8 (each test names the behaviour it proves):
  - webhook namespace match → `Some(WebhookNamespace)` (FR-007);
  - namespace list match → `Some(Namespace)` (FR-001);
  - priority class match → `Some(PriorityClass)` (FR-002);
  - no match → `None`;
  - OR semantics + first-match precedence: webhook-ns beats namespace beats priority class (FR-003, data-model §3.2 order);
  - empty lists / `None` lists → `None` (FR-004);
  - absent priority class on the pod (`None` / empty string) → never `PriorityClass` match (US2 AC4);
  - duplicate entries in a list (e.g. `["a","a"]`) → single match, no error (Edge Cases).
- **GREEN**: implement `ExemptionReason { Namespace, PriorityClass, WebhookNamespace }` (Debug, Clone, Copy, PartialEq, Eq) with `as_str()` → `"namespace"`/`"priority_class"`/`"webhook_namespace"` (the metric + log label value), and `check_exemption(pod_namespace: Option<&str>, pod_priority_class: Option<&str>, spec: &AllocationSpec, webhook_namespace: &str) -> Option<ExemptionReason>` in the data-model §3.2 order. An empty-string priority class must not match.
- **REFACTOR**: extract the per-list containment check into a small helper if it clarifies the three-step order.

### T004 [SHARED] Add `capacity_admission_exemptions_total{reason}` metric

File: `src/metrics.rs`.

- **RED first**: write tests that:
  - `Metrics::new()` pre-creates the `capacity_admission_exemptions_total` series at zero for all three reasons (`namespace`, `priority_class`, `webhook_namespace`) — same startup-pre-creation pattern as the verdicts counter (data-model §4.1, contract);
  - `metrics.record_exemption("namespace")` increments only that series; the other two stay at their current values.
- **GREEN**: add an `exemptions: IntCounterVec` (labels `["reason"]`), register it, pre-create all three reason series at zero in the existing startup loop, and add `record_exemption(&self, reason: &str)`. Add a `HELP`/`TYPE` line check. The metric takes `&str` (not `ExemptionReason`) so `metrics` stays decoupled from `crd`.
- **REFACTOR**: group the new metric with the existing registration block; update the module doc table (now eight metrics).

### T005 [SHARED] Add `DecisionVerdict::Exempt` + summary plumbing

File: `src/webhook/handler.rs`.

- **RED first**: write tests that:
  - `emit_log` / `record_metrics` for an `Exempt` summary bump `capacity_admission_exemptions_total{reason}` with the correct reason and do NOT bump `capacity_admission_verdicts_total` (data-model §4.2); latency is observed; capacity gauges are not refreshed (no figures);
  - a `DecisionSummary` carrying `exemption_reason: Some(ExemptionReason::Namespace)` round-trips the reason.
- **GREEN**: add `Exempt` to `DecisionVerdict`; add `exemption_reason: Option<ExemptionReason>` (+ a label string) to `DecisionSummary`; add a `DecisionSummary::exempt(request, reason, enforcement)` constructor that sets `verdict = Exempt`, `exemption_reason = Some(reason)`, leaves resource figures default (no budget check ran); extend `emit_log` with an `Exempt => INFO` arm carrying `decision = "exempt"`, `exemption_reason`, workload, operation, latency_ms; extend `record_metrics` so `Exempt` calls `record_exemption` and skips verdict/gauges (still observes latency).

**Checkpoint**: pure data + logic in place. The exemption insertion in `evaluate()` can now be wired.

---

## Phase 3: User Story 1 — Exclude by Namespace (Priority: P1) 🎯 MVP

**Goal**: an operator can patch `excludedNamespaces` on the Allocation CRD and pods in those namespaces are admitted without a budget check (FR-001).

**Independent Test**: over-budget pod in an excluded namespace → admitted (Exempt); same pod in a non-excluded namespace → denied (over budget).

### T006 [US1] Wire the exemption check into `evaluate()` (the single insertion point)

Files: `src/webhook/handler.rs` (`evaluate` + `AppState` + `run_decision`), `src/main.rs` (`AppState::new` wiring).

- **RED first**: add integration-style unit tests in `handler.rs` calling `evaluate(...)` directly (pinned clock) that fail today:
  - an over-budget pod whose namespace is in `excludedNamespaces` → `allowed == true`, `verdict == Exempt`, `exemption_reason == Some(Namespace)`, NO warnings, budget check never ran (the over-budget figures are not enforced);
  - the same over-budget pod in a non-excluded namespace → still denied (`Deny`) — the budget check is untouched for non-exempt pods (US1 AC2);
  - a pod in the webhook's own namespace (`webhook_namespace`), over budget, with both exclusion lists empty → `Exempt(WebhookNamespace)` (FR-007 at the webhook layer; "empty CRD cache" = empty exclusion config);
  - fail-closed integrity: an over-budget pod in an excluded namespace whose Allocation **status is missing** is still rejected (`Error`) before the exemption check (task constraint); ditto a missing Allocation singleton.
- **GREEN**:
  - add `webhook_namespace: String` to `AppState`; extend `AppState::new` and `AppState::with_clock` to accept it (thread `config.namespace.clone()` from `main.rs` — FR-010);
  - add `webhook_namespace: &str` as the last parameter of `evaluate()`; thread it through `run_decision()` from `&state.webhook_namespace`;
  - extract the pod's namespace (`request.namespace.as_deref()`) and priority class (`request.object → spec → priority_class_name`) and call `check_exemption(...)` AFTER the Allocation singleton + status are found and BEFORE the freshness check; on `Some(reason)` return an exempt `DecisionOutcome` (allowed, no warnings);
  - update all `evaluate(...)` call sites in the `handler.rs` tests and all `AppState::new`/`AppState::with_clock` call sites across `src/main.rs` + the test files (use `"capacity-admission"` for test webhook namespaces so pods in `"default"` are not accidentally exempt).
- **REFACTOR**: extract the pod-namespace/priority-class extraction into a small helper; keep the insertion block readable with a comment citing data-model §3.1.

### T007 [US1] Integration test: namespace exclusion end-to-end through `handle()`

File: `tests/integration/exclusion.rs` (new) — mirrors `tests/integration/dry_run.rs` harness (real reflector stores + `handle()`).

- **RED first**: write `#[tokio::test]` cases:
  - Allocation singleton with `excludedNamespaces: ["monitoring"]`; over-budget pod in `monitoring` → `allowed`, uid echoed, no warnings (US1 AC1);
  - same config, over-budget pod in `app-team-a` → denied 403 with over-budget message (US1 AC2);
  - webhook-namespace self-exemption: over-budget pod submitted in the webhook's own namespace (`AppState.webhook_namespace`) with empty exclusion config → admitted (FR-007);
  - exemption counter increments: after an exempt decision, `/metrics` text contains `capacity_admission_exemptions_total{reason="namespace"} 1` (and the verdicts counter for that resource stays at its prior value) — FR-008/SC-003.
- **GREEN**: the implementation from T006 makes these pass; the test file is the deliverable. (Write the test first and watch it fail before T006 lands if doing strict per-file TDD; here it lands alongside the integration harness.)

**Checkpoint**: US1 is independently demonstrable. An operator can exclude namespaces via the CRD.

---

## Phase 4: User Story 2 — Exclude by Priority Class (Priority: P2)

**Goal**: an operator can patch `excludedPriorityClasses` and pods with a matching `priorityClassName` are admitted regardless of namespace (FR-002). No new production code — `check_exemption` already handles it; this phase adds coverage.

**Independent Test**: over-budget pod with an excluded priority class → admitted; same pod with no/different priority class → subject to the budget.

### T008 [US2] Integration test: priority class exclusion end-to-end

File: `tests/integration/exclusion.rs` (extend).

- **RED first**: `#[tokio::test]` cases:
  - Allocation with `excludedPriorityClasses: ["system-node-critical"]`; over-budget pod carrying `priorityClassName: "system-node-critical"` in a non-excluded namespace → admitted, `reason="priority_class"` counter increments (US2 AC1);
  - same config, over-budget pod with no `priorityClassName` in the same namespace → denied (US2 AC2);
  - same config, over-budget pod with a different priority class (`"gold"`) → denied (only an exact match exempts);
  - empty-string `priorityClassName` (`""`) → denied (Edge Cases: absent == empty string).
- **GREEN**: covered by T006's `evaluate()` insertion reading `pod.spec.priorityClassName`. Confirm the harness builds pods with a `priorityClassName` field.

**Checkpoint**: US2 independently demonstrable. Priority-class exclusion (impossible before via `namespaceSelector`) works.

---

## Phase 5: User Story 3 — Combined OR Semantics (Priority: P3)

**Goal**: with both lists configured, a pod matching EITHER is exempt; matching both counts once (FR-003).

**Independent Test**: ns-only match, pc-only match, both, and neither, each yielding the correct outcome and reason.

### T009 [US3] Integration test: combined namespace + priority class (OR) semantics

File: `tests/integration/exclusion.rs` (extend).

- **RED first**: with BOTH `excludedNamespaces: ["kube-system"]` and `excludedPriorityClasses: ["system-node-critical"]`:
  - pod with `priorityClassName: "system-node-critical"` in `app-team-a` → admitted, `reason="priority_class"` (US3 AC1);
  - pod with no priority class in `kube-system` → admitted, `reason="namespace"` (US3 AC2);
  - pod with no priority class in `app-team-a` → denied (US3 AC3);
  - pod matching BOTH (`kube-system` + `system-node-critical`) → admitted once, reason is the first-match (`namespace`, per data-model §3.2 order: namespace before priority class) — exemption is boolean, not double-counted (US3 AC4). Assert the `namespace` counter increments by 1, not 2.
- **GREEN**: covered by `check_exemption`'s first-match order. The test is the deliverable.

### T010 [US3] BDD feature + steps for exclusion (optional but recommended — Principle VI)

Files: `tests/bdd/features/admission_exclusion.feature` (new Gherkin), `tests/bdd/steps/exclusion_steps.rs` (new), wired into the existing cucumber runner (`tests/bdd/`).

- Write the Gherkin scenarios from `quickstart.md` (US1/US2/US3) FIRST, watch them fail (no steps / no binding), then implement the step bindings against the same `handle()` harness used by `tests/integration/exclusion.rs`. If the BDD wiring adds risk, defer to a follow-up — the integration tests in T007–T009 already satisfy Principle VI for this feature; note any deferral explicitly.

**Checkpoint**: all three stories independently demonstrable and combined OR proven.

---

## Phase 6: Polish & Cross-Cutting

**Purpose**: deploy manifests, docs, and the production wiring that makes the feature operable.

### T011 [SHARED] CRD schema: add the two optional array fields to `deploy/crds.yaml`

File: `deploy/crds.yaml`.

- Add `excludedNamespaces` and `excludedPriorityClasses` under the Allocation `spec.properties` (data-model §5): `type: array`, `nullable: true`, `items: { type: string }`, with the descriptive `description`. Neither added to the `required: ["budgetPercent"]` array.
- **Test**: extend the `crd_schema_*` unit test in `src/crd/allocation.rs` (T001) to assert both fields appear in the derive-generated schema and are absent from `required`; the hand-written `deploy/crds.yaml` is kept in sync with the derive by a `#[test]` that re-renders and compares the relevant slice if a comparison helper already exists, otherwise by review + the schema unit test.

### T012 [SHARED] Simplify the `namespaceSelector` in `deploy/webhook-config.yaml`

File: `deploy/webhook-config.yaml`.

- Reduce the `namespaceSelector` to defence-in-depth for the webhook's own namespace only (FR-009, data-model §6, contract): `key: kubernetes.io/metadata.name`, `operator: NotIn`, `values: ["capacity-admission"]`. Remove `kube-system` / `kube-public` (those move to the CRD, operator-configured). Update the file's header comment to explain the defence-in-depth role + cold-start rationale.

### T013 [SHARED] README: document the exclusion policy (Principle X)

File: `README.md`.

- Add the two new `Allocation` spec fields to the CRD table (type, constraint, description), mirroring the `enforcementMode`/`nodeSelectors` rows.
- Add a **Workload Exclusion** subsection: OR semantics, check order (webhook ns → namespaces → priority classes), string-match-only priority class, backward-compatible default, the "excluded pods are still counted" note, and the cold-start/self-exemption story (apiserver `namespaceSelector` defence-in-depth + CRD at runtime).
- Add runnable `kubectl patch` examples for namespace, priority class, both, and removal.
- Update the **Prometheus Metrics** table + label vocabularies with `capacity_admission_exemptions_total{reason}` (reason ∈ {namespace, priority_class, webhook_namespace}); note `Exempt` decisions do NOT bump the verdicts counter.
- Update the **Structured Logging** table with the `exempt` decision and `exemption_reason` field.
- Update the **Webhook Self-Admission (Bootstrap)** section + the **Failure Modes** note (exempt decisions are an explicit allow, not a fail-closed path).

### T014 [SHARED] Final quality gate + PR

- Run the full gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` — all green (Constitution XI). Fix any failure; do not declare success with a failing test (Principle XI).
- Commit per task/logical group on `spec/008-workload-exclusion`; push and open PR to `main` titled `feat: workload exclusion policy — namespace + priority class CRD config (spec-008)`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: T000 baseline — done.
- **Phase 2 (Foundational)**: T001 → T002 (T002 is the compile tail of T001); T003, T004, T005 can proceed in priority order. T003 depends on T001 (uses `AllocationSpec`). T005 depends on T003 (uses `ExemptionReason`).
- **Phase 3 (US1)**: T006 depends on T003 + T005 (and threads `AppState.webhook_namespace`). T007 depends on T006.
- **Phase 4 (US2)**: T008 depends on T006 (same insertion point; no new production code).
- **Phase 5 (US3)**: T009 depends on T006. T010 depends on T006/T007 harness.
- **Phase 6 (Polish)**: T011/T012/T013 are independent of each other (different files) but follow the code landing. T014 is last.

### Within Each Task

- Test written FIRST and watched to fail (RED), then minimal code (GREEN), then refactor — one behaviour per cycle (Principle VIII).

### Parallel Opportunities

- T003, T004 are different files and could proceed in parallel once T001/T002 land.
- T011, T012, T013 are different files (Polish) and are parallel-safe.

---

## Notes

- [P] tasks = different files, no dependencies.
- The Allocation Controller pod-counting logic is **never** touched (excluded pods stay counted — research R5).
- No PriorityClass resource watch is added (research R3).
- The `--namespace`/`NAMESPACE` config is **retained** (FR-010) and now also seeds `AppState.webhook_namespace`.
- The exemption check is a single insertion in `evaluate()`; the three user stories differ only in which `check_exemption` reason fires and the test coverage around it.
