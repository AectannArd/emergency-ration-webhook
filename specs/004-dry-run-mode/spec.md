# Feature Specification: Dry-Run Enforcement Mode

**Feature Branch**: `spec/dry-run-mode`

**Created**: 2026-07-27

**Status**: Draft

**Input**: User description: "Add a dry-run feature to the operator."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Shadow Evaluation (Priority: P1)

A cluster operator wants to evaluate the impact of the capacity admission
webhook on an existing cluster *before* it starts blocking workloads. They
switch the webhook to dry-run mode: the webhook evaluates every pod admission
request exactly as it would in enforce mode — performing the budget check
against the same capacity state — but it admits every pod that would have been
rejected solely for exceeding the budget. The pod goes through, but the
operator can see, from logs, metrics, and admission warnings, which workloads
*would* have been blocked had enforcement been active.

This lets an operator install the webhook on a production cluster, run it for a
period in dry-run mode, inspect which deployments would be affected, adjust
budgets or workloads accordingly, and then flip to enforce mode with confidence.
It eliminates the "big bang" adoption risk: no existing workload is blocked by a
newly-installed webhook whose budget was set without knowing the current
workload profile.

**Why this priority**: This is the core capability — audit-mode evaluation. Without
it, operators face a chicken-and-egg problem: they cannot safely enable
enforcement without knowing the impact, but they cannot know the impact without
enabling the webhook. Dry-run mode breaks the deadlock by making the webhook
safe to install in audit mode first.

**Independent Test**: Install the webhook in dry-run mode on a cluster, submit a
pod whose requests exceed the budget, and observe that the pod is admitted but
the decision is recorded as a dry-run would-deny in logs and metrics and a
warning is returned to the caller. Switch to enforce mode and submit the same
pod; observe it rejected.

**Acceptance Scenarios**:

1. **Given** the webhook is in dry-run mode and a cluster with capacity and a
   configured budget, **When** a pod whose requests fit within the remaining
   budget is submitted, **Then** the pod is **admitted** and the decision is
   logged and recorded in metrics as a normal allow (the budget is not
   violated, so dry-run makes no difference).
2. **Given** the webhook is in dry-run mode, **When** a pod whose requests
   exceed the budget is submitted, **Then** the pod is **admitted** (`allowed:
   true`), the admission response carries a warning containing the would-be
   rejection reason (the budget-violation message naming the resource, the
   current allocation, the requested increment, the projected total, and the
   ceiling), and the decision is logged and recorded in metrics as a dry-run
   would-deny.
3. **Given** the webhook is in enforce mode (the default), **When** a pod whose
   requests exceed the budget is submitted, **Then** the pod is **rejected**
   exactly as before — dry-run mode does not change enforce-mode behaviour.
4. **Given** the webhook is in dry-run mode, **When** the mode is switched to
   enforce via a CRD spec patch, **Then** the next admission decision that would
   exceed the budget is rejected — the mode change takes effect without a
   restart on the next cached spec read.

---

### User Story 2 - Fail-Closed Integrity in Dry-Run (Priority: P2)

When the webhook is in dry-run mode and it encounters a condition where it
cannot authoritatively evaluate the request — capacity data missing, capacity
data stale, request malformed, decision timeout, internal error — it **rejects**
the request just as it would in enforce mode. Dry-run mode only converts
*over-budget* denials into admits; it does **not** convert *error* rejections
into admits. An admission under degraded knowledge is never safe, regardless of
enforcement mode.

The operator can trust dry-run metrics: every dry-run would-deny represents a
*real* budget violation that enforcement would catch, not an error path that
dry-run silently swallowed.

**Why this priority**: Without this, dry-run mode would silently admit pods the
webhook cannot evaluate, which is worse than not having the webhook at all — it
would hide the very failures the webhook exists to surface. This is the
non-negotiable safety property that makes dry-run mode trustworthy.

**Independent Test**: Put the webhook in dry-run mode, make capacity data
unavailable (delete the Allocation singleton or let it go stale), submit a pod,
and observe it rejected — not admitted — with the fail-closed reason logged.

**Acceptance Scenarios**:

1. **Given** the webhook is in dry-run mode and the capacity data is stale
   (Allocation status age exceeds the freshness threshold), **When** a pod is
   submitted, **Then** the pod is **rejected** (fail-closed) with reason
   "capacity data stale" — dry-run does not override fail-closed.
2. **Given** the webhook is in dry-run mode and the Allocation singleton is
   missing (not yet populated), **When** a pod is submitted, **Then** the pod is
   **rejected** (fail-closed) with reason "capacity data missing".
3. **Given** the webhook is in dry-run mode and the admission request is
   malformed (unparseable AdmissionReview), **When** it arrives, **Then** the
   request is **rejected** with reason "deserialisation failure".
4. **Given** the webhook is in dry-run mode and the decision exceeds the
   timeout, **When** the timeout fires, **Then** the request is **rejected**
   with reason "timeout".
5. **Given** the webhook is in dry-run mode and an over-budget pod triggers an
   internal panic in the decision path, **When** the panic is caught, **Then**
   the request is **rejected** with reason "internal error" — not admitted.

---

### User Story 3 - Dry-Run Observability (Priority: P3)

An operator running the webhook in dry-run mode can build dashboards and alerts
that answer: how many pods would have been blocked, which resources were
violated, and how the would-be rejection rate trends over time. The dry-run
decision is distinguishable from enforced allows and enforced denials in both
structured logs and metrics, so the operator is never confused about whether a
rejection is real (enforce mode active) or simulated (dry-run audit).

The operator can also switch modes at runtime and see the mode change reflected
in observability without restarting the process.

**Why this priority**: Observability is what makes dry-run mode useful — without
the ability to see and quantify what dry-run *would* block, the mode is just a
silent pass-through. It ranks after P1/P2 because it extends the core
capability rather than delivering it.

**Independent Test**: Submit a mix of within-budget and over-budget pods in
dry-run mode, then query the metrics endpoint and inspect structured logs.
Confirm the within-budget admits and the over-budget would-be-denies are
distinguishable in both signals.

**Acceptance Scenarios**:

1. **Given** the webhook is in dry-run mode, **When** an over-budget pod is
   admitted (dry-run would-deny), **Then** a structured log entry is emitted at
   WARN level containing: the workload identity, the decision value
   `dry_run_deny` (or equivalent), the violated resource(s), and the same
   capacity figures a real deny would carry.
2. **Given** the webhook is in dry-run mode, **When** a within-budget pod is
   admitted, **Then** a structured log entry is emitted at INFO level with a
   normal allow decision (indistinguishable from an enforce-mode allow).
3. **Given** the webhook is running, **When** the metrics endpoint is scraped,
   **Then** admission verdict metrics distinguish dry-run would-deny decisions
   from enforce-mode deny and allow decisions, so an operator can build a
   dashboard answering "how many pods would dry-run block?" without conflating
   them with real denies or real allows.
4. **Given** the webhook is in dry-run mode, **When** the operator switches to
   enforce mode via a CRD spec patch, **Then** subsequent decisions are logged
   and recorded in metrics as enforce-mode decisions (no `dry_run` qualifier),
   confirming the mode change propagated to observability without a restart.
5. **Given** any dry-run decision, **When** the decision is made, **Then** the
   enforcement mode is included in the structured log entry, so an operator
   reviewing logs can immediately tell whether a given decision was evaluated
   under enforce or dry-run semantics.

---

### Edge Cases

- **`enforcementMode` field absent from the spec**: treated as `enforce` (the
  safe default). A pre-existing Allocation singleton created before this feature
  has no `enforcementMode` field; the webhook MUST treat the absent field as
  `enforce`, not as `dry-run`, so upgrading does not silently disable
  enforcement.
- **`enforcementMode` set to an unknown value** (e.g., a typo like `audit`):
  treated as `enforce` (the safe default). An invalid value MUST NOT enable
  dry-run mode; it falls back to enforcement so an operator typo cannot
  accidentally disable the safety guard.
- **Mode switch while in-flight requests are being evaluated**: requests that
  began evaluation under one mode complete under that mode. The mode is read
  once per decision from the cached Allocation spec; a mid-flight patch does not
  retroactively change a decision already in progress. The next request picks up
  the new mode.
- **Both resources over budget in dry-run**: the warning message lists all
  violated resources (CPU and RAM), exactly as a real rejection message would —
  the operator sees the complete picture of what would be blocked, not just the
  first violation.
- **Dry-run mode and zero-budget (`budgetPercent: 0`)**: every pod requesting
  more than zero resources is admitted with a dry-run would-deny warning. This
  is the most extreme dry-run scenario — the webhook would block everything,
  but in dry-run it admits everything while logging the would-be blocks.
- **Dry-run mode with a pod that has no resource requests**: admitted normally
  (consuming zero budget). There is no budget violation, so dry-run makes no
  difference — the decision is a normal allow.
- **Controller cold start with dry-run enabled**: if the Allocation singleton is
  auto-created by the controller, the default `enforcementMode` is `enforce`.
  Dry-run is opt-in; the operator must explicitly patch the spec to enter audit
  mode. The webhook never boots into dry-run by default.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST support an enforcement mode setting on the
  Allocation CRD spec, with two valid values: `enforce` (the default) and
  `dry-run`.
- **FR-002**: The enforcement mode MUST be adjustable at runtime via a CRD spec
  update (e.g., `kubectl patch`) without restarting the webhook process, taking
  effect on the next admission decision after the cached spec is updated.
- **FR-003**: When `enforcementMode` is absent from the Allocation spec or set
  to an unrecognised value, the system MUST treat it as `enforce` — the safe
  default that preserves the existing fail-closed budget enforcement behaviour.
- **FR-004**: In dry-run mode, when an admission request would be denied solely
  because the pod's projected allocation exceeds the budget, the system MUST
  admit the pod (`allowed: true`) instead of denying it.
- **FR-005**: When a pod is admitted under dry-run mode due to an over-budget
  condition, the AdmissionResponse MUST carry a warning containing the would-be
  rejection message — identifying the violated resource(s), the current
  allocation, the requested increment, the projected total, and the ceiling —
  surfaced via the admission warnings field.
- **FR-006**: Dry-run mode MUST NOT alter the outcome of fail-closed paths. When
  the system cannot authoritatively evaluate a request (capacity data missing,
  capacity data stale, request malformed, decision timeout, internal error), the
  request MUST be rejected regardless of `enforcementMode`. Dry-run converts
  only over-budget denials; it never converts error rejections.
- **FR-007**: The system MUST emit structured logs for every dry-run decision
  that would have been a rejection, containing the workload identity, the
  violated resource(s), the capacity figures, and a clear indicator that the
  decision was a dry-run would-deny (distinct from an enforced deny and a
  normal allow).
- **FR-008**: The system MUST expose metrics that distinguish dry-run would-deny
  decisions from enforce-mode allow and deny decisions, so operators can build
  dashboards quantifying the would-be impact of enabling enforcement.
- **FR-009**: The enforcement mode active at the time of each decision MUST be
  included in the structured log entry for that decision, so an operator can
  determine from logs alone whether a given decision was evaluated under enforce
  or dry-run semantics.
- **FR-010**: The auto-created Allocation singleton MUST default to
  `enforcementMode: enforce`. Dry-run mode is opt-in; the operator must
  explicitly set it.

### Key Entities *(include if feature involves data)*

- **Enforcement Mode**: the toggle between `enforce` (reject over-budget pods)
  and `dry-run` (admit over-budget pods with a warning). Lives in the Allocation
  CRD `spec` alongside `budgetPercent`. Defaults to `enforce`. Read by the
  admission webhook from its in-process cache on every decision.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can install the webhook in dry-run mode on a cluster
  with existing workloads and, after an observation period, determine exactly
  which workloads would have been blocked by enforcement — 100% of over-budget
  admissions are surfaced via warnings, logs, and metrics.
- **SC-002**: Switching between dry-run and enforce mode (in either direction)
  takes effect without a process restart, on the next admission decision after
  the spec update propagates to the webhook's cache.
- **SC-003**: Under every fail-closed condition (capacity data missing/stale,
  request malformed, timeout, internal error), the system rejects the request in
  both dry-run and enforce modes — zero cases of admission under degraded
  knowledge, regardless of enforcement mode.
- **SC-004**: An operator can distinguish dry-run would-deny decisions from
  enforced denies and normal allows in both structured logs and metrics, without
  ambiguity — the dry-run verdict is a first-class, distinguishable signal.
- **SC-005**: The enforcement mode does not degrade admission decision latency:
  the dry-run path performs the same budget check as the enforce path, so the
  performance targets (provisional p99 < 100 ms, p50 < 50 ms) apply unchanged.

## Assumptions

- **Admission warnings are available** in all supported Kubernetes versions
  (the N-2 window: 1.34–1.36 at the time of writing). The admission `warnings`
  field was introduced in Kubernetes 1.19 and is GA; all versions in the support
  window are far above this floor.
- **Dry-run is per-cluster, not per-namespace or per-workload.** The
  `enforcementMode` lives on the cluster-scoped Allocation singleton, so it
  applies to all monitored namespaces uniformly. Granular dry-run scope
  (e.g., dry-run in one namespace, enforce in another) is a deliberately
  deferred future concern.
- **Dry-run evaluates the full decision pipeline.** The budget check, capacity
  freshness check, request parsing, and fail-closed guards all run identically
  in both modes. The only behavioural difference is the final verdict for
  over-budget conditions.
- **The webhook controls the verdict, not the lifecycle.** As in enforce mode,
  dry-run mode never mutates the pod object, never evicts running pods, and never
  modifies cluster state. It only decides allow/deny and emits a warning.
- **Validating-only webhook.** The webhook does not mutate the admission request,
  even in dry-run mode. The `warnings` field is set on the AdmissionResponse, not
  via a mutating webhook patch. This is consistent with the existing
  valid-only/no-mutation architecture (Constitution Principle V).
- **Metrics and logs for dry-run decisions are new signals.** Existing enforce-
  mode metrics and log formats are preserved; dry-run adds a distinguishable
  verdict variant rather than overloading existing ones, so dashboards built on
  current metrics are not broken by this feature.
