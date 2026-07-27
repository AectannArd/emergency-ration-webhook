# Feature Specification: On-Demand Infrastructure Verification

**Feature Branch**: `005-on-demand-verification`

**Created**: 2026-07-27

**Status**: Draft

**Input**: User description: "Add the ability for this project to run on-demand tests
against the real infrastructure. At some point in time I give you a kubeconfig for
the target cluster — you perform setup, tests and teardown and print the report."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Verify Enforcement on a Real Cluster (Priority: P1)

An operator wants to confirm that the capacity admission webhook enforces
budgets correctly when deployed against a real Kubernetes cluster — not just the
mocked-apiserver tests in `cargo test` or the ephemeral `kind` smoke test in CI.
They hand the tool a kubeconfig for a clean, throwaway target cluster. The tool
installs the full webhook stack (CRDs, RBAC, Deployment, Service, webhook
configuration, TLS certificate), waits for it to become ready, runs a structured
suite of enforcement scenarios against the live admission path, tears down
everything it installed, and prints a report.

The enforcement scenarios verify the core behavioural contract on real
infrastructure: a within-budget pod is admitted, an over-budget pod is rejected,
the budget can be adjusted at runtime without restart and the new ceiling takes
effect, dry-run mode admits over-budget pods with a warning, the controller-
populated capacity state matches the cluster's actual node allocatable, and the
metrics and health endpoints respond.

**Why this priority**: without this, the project has no operator-facing way to
verify its constitutional guarantees (fail-closed enforcement, budget accuracy,
runtime configurability) on real infrastructure. Every other story builds on
this lifecycle. It is the minimum viable verification.

**Independent Test**: fully testable by running the tool against a clean cluster
and checking the report shows all enforcement scenarios passing and the cluster
is left empty afterward.

**Acceptance Scenarios**:

1. **Given** a clean target cluster reachable via the provided kubeconfig, **When**
   the operator runs the tool, **Then** the tool installs the webhook stack and
   all components reach Ready state before any scenario runs.
2. **Given** the webhook is installed and capacity state is populated, **When** a
   pod with small resource requests is submitted, **Then** it is admitted.
3. **Given** the webhook is installed and capacity state is populated, **When** a
   pod with requests exceeding the budget is submitted, **Then** it is rejected
   with a message naming the violated resource and the capacity figures.
4. **Given** the webhook is enforcing at budget X, **When** the operator (tool)
   adjusts the budget to Y at runtime, **Then** subsequent admission decisions
   reflect the new ceiling without restarting the webhook.
5. **Given** dry-run mode is enabled, **When** an over-budget pod is submitted,
   **Then** it is admitted and the would-be rejection is surfaced as a warning.
6. **Given** the webhook is installed, **When** the tool reads the capacity state
   from the CRDs, **Then** the controller-computed figures match the actual
   node allocatable summed across the cluster.
7. **Given** the tool has finished running all scenarios, **When** teardown
   completes, **Then** the cluster contains no traces of the webhook installation
   (no namespace, no CRDs, no webhook configuration, no RBAC objects).

---

### User Story 2 - Verify Fail-Closed Paths Under Active Degradation (Priority: P2)

An operator wants proof that the webhook's fail-closed guarantee — the
NON-NEGOTIABLE Constitution Principle I — actually holds when things go wrong on
real infrastructure, not just in mocked unit tests. The tool actively degrades
the running webhook installation mid-verification: it kills webhook pods to test
unreachability rejection, deletes or corrupts the capacity CRDs to test
missing-data rejection, and induces stale capacity data to test the freshness
timeout. After each degradation, it submits a pod and asserts the webhook
rejected it (never admitted under degraded knowledge). Between degradation
scenarios, the tool restores the webhook to a healthy state so subsequent
scenarios start from a known-good baseline.

This is safe because the tool assumes a throwaway cluster (User Story 1's
precondition): actively breaking the webhook installation is non-destructive
when the cluster is disposable.

**Why this priority**: fail-closed is the webhook's defining property. CI smoke
tests and mocked integration tests cover the happy path and the mocked error
paths, but neither exercises the real failure modes of a live Kubernetes cluster
(pod eviction, CRD deletion, cache staleness propagation). This story is what
makes the tool worth running over just trusting CI.

**Independent Test**: runnable as a distinct scenario group after User Story 1's
setup phase. The tool degrades the webhook, submits pods, asserts rejection, and
restores health — each degradation scenario passes or fails independently.

**Acceptance Scenarios**:

1. **Given** the webhook is installed and healthy, **When** the tool kills all
   webhook pods and submits a pod, **Then** the admission request is rejected
   (the API server itself rejects because the webhook is unreachable, per
   `failurePolicy: Fail`).
2. **Given** the webhook is installed, **When** the tool deletes the capacity
   CRD instances and submits a pod, **Then** the webhook rejects with a
   missing-capacity-data outcome (it cannot verify the budget without capacity
   state).
3. **Given** the webhook is installed, **When** the tool induces stale capacity
   data (capacity state older than the freshness timeout) and submits a pod,
   **Then** the webhook rejects with a stale-data outcome.
4. **Given** a degradation scenario has completed, **When** the tool restores the
   webhook to a healthy state, **Then** subsequent scenarios run against a
   known-good baseline (the degradation is fully reversible within the throwaway
   cluster).

---

### User Story 3 - Machine-Readable Output for Automation (Priority: P3)

An operator or CI engineer wants to wire the verification tool into an automated
pipeline — a scheduled job, a pre-production gate, or a CI step. They need
structured, machine-readable output (not just human-readable terminal text) and
clear exit-code semantics so the pipeline can branch on pass/fail. The tool's
JSON output mode emits a structured report — one record per scenario with its
name, status (pass/fail), duration, and failure detail — and the process exits
non-zero if any scenario failed.

**Why this priority**: the human-readable report (User Story 1) serves the
interactive operator; this story serves the automated consumer. It is lower
priority because the tool is already valuable interactively, but JSON output
unlocks unattended/scheduled verification.

**Independent Test**: the JSON schema and exit-code semantics can be validated
independently by running the tool in JSON mode against any reachable cluster and
asserting the output parses and the exit code matches the scenario outcomes.

**Acceptance Scenarios**:

1. **Given** the tool is run with the JSON output flag, **When** verification
   completes, **Then** the output is valid structured JSON containing one record
   per scenario with its name, pass/fail status, and (on failure) the reason.
2. **Given** one or more scenarios failed, **When** the tool exits, **Then** the
   process exit code is non-zero.
3. **Given** all scenarios passed, **When** the tool exits, **Then** the process
   exit code is zero.

---

### Edge Cases

- **Cluster already has workloads**: the tool assumes a clean, throwaway cluster.
  If it detects existing workloads (e.g. pods in the `default` namespace), it
  should warn or refuse, because active degradation (User Story 2) could disrupt
  them. The exact safety heuristic is a plan-phase decision.
- **Webhook not reachable after install**: if the webhook pods never reach Ready
  (image pull failure, RBAC misconfiguration, TLS issue), the tool must report
  the failure with diagnostic detail (pod status, recent logs, events) and
  proceed to teardown — never leave a half-installed stack on the cluster.
- **Capacity state never populates**: the controllers may lag on cold start. The
  tool must wait (with a timeout) for capacity state to become non-zero before
  running scenarios, and report a setup failure (with diagnostics) if it never
  does.
- **Teardown fails mid-way**: if a teardown step fails (API error, timeout), the
  tool must continue tearing down remaining objects and report which objects
  could not be removed, so the operator knows the cluster is not fully clean.
- **Network interruption mid-run**: if the connection to the cluster drops during
  a scenario, the tool must report the failure, attempt teardown, and exit
  non-zero — not hang.
- **Budget edge values**: the enforcement suite must verify both `budgetPercent:
  0` (circuit-breaker — every non-zero request rejected) and `budgetPercent: 100`
  (physical overcommit guard — only requests exceeding actual capacity denied).
- **`--keep-on-failure` escape hatch**: when this flag is set, the tool skips
  teardown on failure so the operator can inspect the live state for debugging.
  Without the flag, teardown always runs — even on failure.
- **Empty cluster (zero nodes)**: if the target cluster has no nodes, capacity is
  zero and every pod is over-budget. The tool should detect this as a setup
  precondition failure rather than running scenarios against a degenerate
  cluster.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The tool MUST accept a kubeconfig specifying the target cluster,
  via a `--kubeconfig` flag, the `KUBECONFIG` environment variable, or the
  default kubeconfig location (flag > env > default precedence).
- **FR-002**: The tool MUST install the full webhook stack — CRDs, RBAC
  (ServiceAccount, ClusterRole, ClusterRoleBinding), the webhook Deployment and
  Service, the ValidatingWebhookConfiguration, and a TLS serving certificate —
  against the target cluster from the project's existing deployment manifests.
- **FR-003**: The tool MUST generate and provision a self-signed TLS serving
  certificate in-process (not relying on cert-manager being installed), with
  SANs covering the in-cluster Service DNS, so the webhook's HTTPS endpoint is
  trusted by the API server.
- **FR-004**: The tool MUST wait for all webhook pods to reach Ready state and
  for capacity state (the controller-computed CRD status) to become non-zero
  before running any verification scenario, with a configurable timeout.
- **FR-005**: The tool MUST verify that a within-budget pod is admitted and an
  over-budget pod is rejected against the live admission path.
- **FR-006**: The tool MUST verify budget edge values: `budgetPercent: 0`
  (circuit-breaker) and `budgetPercent: 100` (physical-overcommit guard).
- **FR-007**: The tool MUST verify that adjusting the budget at runtime (patching
  the Allocation CRD spec) takes effect on subsequent admission decisions
  without restarting the webhook.
- **FR-008**: The tool MUST verify that dry-run enforcement mode admits an
  over-budget pod while surfacing the would-be rejection as a warning.
- **FR-009**: The tool MUST verify that the controller-computed capacity state
  (CRD status) matches the cluster's actual node allocatable (summed across
  nodes).
- **FR-010**: The tool MUST verify that the metrics endpoint and health endpoint
  respond.
- **FR-011**: The tool MUST actively degrade the running webhook and verify that
  each fail-closed path rejects on real infrastructure: webhook pods killed
  (unreachable rejection), capacity CRD instances deleted (missing-data
  rejection), and stale capacity data induced (freshness-timeout rejection).
- **FR-012**: After each degradation scenario, the tool MUST restore the webhook
  to a healthy state before running the next scenario.
- **FR-013**: The tool MUST tear down everything it installed — in reverse
  dependency order — after verification completes, whether the run succeeded or
  failed.
- **FR-014**: The tool MUST support a `--keep-on-failure` flag that skips
  teardown when a scenario fails, leaving the installation in place for
  debugging. Without the flag, teardown always runs.
- **FR-015**: The tool MUST report each scenario's outcome (pass/fail) with
  sufficient detail to diagnose a failure (the scenario name, the expected and
  actual outcome, and any error message from the cluster).
- **FR-016**: The tool MUST print a human-readable report by default, with one
  section per scenario showing pass/fail and details, plus a summary (total /
  passed / failed).
- **FR-017**: The tool MUST support a `--json` flag that emits the report as
  structured machine-readable JSON (one record per scenario: name, status,
  duration, failure detail) instead of human-readable text.
- **FR-018**: The tool MUST exit with code zero if all scenarios passed and
  non-zero if any scenario failed or setup/teardown encountered an unrecoverable
  error.
- **FR-019**: The tool MUST refuse to run or warn prominently if it detects the
  target cluster is not clean (has existing workloads), because active
  degradation (FR-011) could disrupt them. The exact heuristic is a plan-phase
  decision.

### Key Entities

- **Verification Run**: a single execution of the tool against one target
  cluster. Has a lifecycle (setup → scenarios → teardown) and an outcome
  (pass/fail per scenario, overall pass/fail).
- **Verification Scenario**: a single named, independently-outcomed test within
  a run. Each scenario has a name, an expected outcome, an actual outcome, a
  pass/fail status, and (on failure) a diagnostic detail. Scenarios are grouped:
  enforcement scenarios (User Story 1) and degradation scenarios (User Story 2).
- **Verification Report**: the aggregate output of a run — the collection of
  scenario outcomes plus a summary. Rendered as human-readable text (default) or
  JSON (`--json`).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can verify the webhook's enforcement behaviour against
  a real cluster by providing a kubeconfig and running a single command, with no
  manual setup or teardown steps.
- **SC-002**: Every fail-closed path enumerated in the webhook's failure-mode
  matrix is verified to reject on real infrastructure under active degradation,
  not only under mocked-apiserver tests.
- **SC-003**: After a verification run (success or failure), the target cluster
  contains zero traces of the webhook installation unless `--keep-on-failure` was
  explicitly set.
- **SC-004**: A failed scenario's report entry contains enough information
  (expected vs actual outcome, cluster error message) for an operator to diagnose
  the failure without re-running the tool.
- **SC-005**: The JSON report is consumable by an automated pipeline: valid JSON,
  one record per scenario, and an exit code that unambiguously signals pass/fail.
- **SC-006**: The complete verification run (setup + all scenarios + teardown)
  completes within a practical wall-clock time for an operator-initiated check
  (target: under 10 minutes on a typical cluster; provisional, ratify in
  `/speckit-plan`).

## Assumptions

- The target cluster is **clean and throwaway**: the operator guarantees no other
  workloads depend on it. The tool installs into the default webhook namespace
  and actively degrades the installation (User Story 2), which is safe only under
  this guarantee.
- The target cluster runs a Kubernetes version within the project's N-2 support
  window (currently 1.34–1.36).
- The tool is built from the same source tree as the webhook and ships as a
  second binary in the same crate, so it shares the webhook's CRD type
definitions and configuration types.
- The tool applies the existing `deploy/` manifests against the target cluster
  directly (it does not shell out to `kubectl`), keeping the single-binary,
  no-external-dependencies property.
- TLS provisioning uses the manual self-signed certificate path (in-process
  generation), not cert-manager — the tool must not assume cert-manager is
  installed on the target cluster.
- The `--namespace` override for installing into a non-default namespace is out
  of scope for v1; the tool uses the webhook's default namespace
  (`capacity-admission`).
- The tool's own logic (report formatting, scenario orchestration) is unit-
  testable in isolation, separate from the live-cluster integration tests.
