# Feature Specification: Capacity Admission Webhook

**Feature Branch**: `spec/capacity-admission-webhook`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "We create a webhook for Kubernetes that is going to
track cluster capacity in CPU and RAM and ensure that the scheduled workloads do
not overgo specified capacity percent."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Budget Enforcement (Priority: P1)

A cluster operator submits a new workload (or updates an existing one). The
system determines whether the workload's resource demands fit within the
remaining cluster capacity budget — a configurable percentage of total
allocatable CPU and RAM. If the workload fits, it is admitted. If it would push
cluster-wide allocation over the ceiling, it is rejected with a clear
explanation of which resource(s) exceeded the budget and by how much.

The budget is calculated against **declared resource requests** (what the pod
asks for in its spec), not live runtime usage. This makes the admission decision
deterministic: it reflects what the scheduler will actually try to reserve, and
it does not depend on a metrics pipeline being healthy.

**Why this priority**: This is the entire reason the webhook exists — preventing
cluster overcommit before it happens. Without this, there is no product. Every
other story depends on or extends this core enforcement capability.

**Independent Test**: Submit a pod whose requests fit within the budget and
observe it admitted; submit a pod whose requests exceed the budget and observe
it rejected with a message citing the violated resource and the budget figures.
Delivers standalone value: the cluster is protected from overcommit.

**Acceptance Scenarios**:

1. **Given** a cluster with 100 CPU and 200 GiB RAM allocatable and a budget
   ceiling of 80% (80 CPU, 160 GiB), and current allocation at 70 CPU / 110 GiB,
   **When** a pod requesting 5 CPU / 40 GiB is submitted,
   **Then** the pod is **admitted** (75 CPU / 150 GiB — both under ceiling).
2. **Given** the same cluster state, **When** a pod requesting 15 CPU / 10 GiB is
   submitted, **Then** the pod is **rejected** — CPU would reach 85 (over the 80
   ceiling) — and the rejection message names CPU as the violated resource,
   shows current (70), requested increment (15), projected total (85), and
   ceiling (80).
3. **Given** a cluster at exactly the ceiling (80 CPU allocated out of 80 CPU
   budget), **When** any pod requesting >0 CPU is submitted,
   **Then** the pod is **rejected** — the budget is a hard ceiling, not a soft
   target.
4. **Given** a pod requesting 0 CPU and 0 RAM (no resource requests declared),
   **When** it is submitted, **Then** it is **admitted** — it consumes nothing
   against the budget.
5. **Given** an existing pod consuming 10 CPU is updated to request 20 CPU,
   **When** the update is submitted, **Then** the system evaluates the delta
   (+10 CPU) against the budget and admits or rejects accordingly.

---

### User Story 2 - Capacity Awareness (Priority: P2)

A cluster operator needs to understand the cluster's capacity situation at any
time: how much total capacity exists, how much is currently allocated, what the
budget ceiling is, and how close the cluster is to the limit. Every admission
decision — admit or deny — is observable with the capacity figures that drove
it.

Denials carry a human-readable message sufficient for the workload owner to
understand why their pod was rejected and what they would need to change. The
system also exposes current capacity utilisation as metrics so dashboards and
alerts can be built on top of it.

**Why this priority**: A capacity guardian that cannot explain its own decisions
cannot be trusted in production or debugged during an incident. Observability is
required for the v1 admission path — it is not deferred to a "polish phase."

**Independent Test**: Submit pods that trigger both an admit and a deny, and
observe that each decision is accompanied by the capacity state (total
allocatable, current allocation, ceiling, remaining) that was used. Query the
exposed metrics endpoint and confirm capacity utilisation figures are present.

**Acceptance Scenarios**:

1. **Given** any admission decision (admit or deny), **When** the decision is
   made, **Then** a structured log entry is emitted containing: the workload
   identity, the decision, the resource type(s) evaluated, and the capacity
   figures used (total, current allocation, requested, projected, ceiling).
2. **Given** a denial, **When** the rejection is returned, **Then** the rejection
   message identifies the violated resource, the current allocation, the
   requested increment, the projected total, and the ceiling — in a format a
   workload owner can act on without contacting the platform team.
3. **Given** the system is running, **When** an operator queries the metrics
   endpoint, **Then** current capacity utilisation per resource type (CPU, RAM)
   is available, alongside admission verdict counts and decision latency.
4. **Given** cluster capacity changes (nodes added or removed),
   **When** the capacity state updates, **Then** the new total is reflected in
   subsequent admission decisions and metrics without restart.

---

### User Story 3 - Fail-Safe Operation (Priority: P3)

When the system cannot authoritatively verify that a workload fits within the
budget — for any reason (a component is down, the capacity data is stale or
unreachable, a timeout fires, a request is malformed) — the admission request is
**rejected**, never silently admitted.

Every failure path maps to one of two declared outcomes: reject (the default),
or a narrow explicitly-configured exception with a recorded justification. There
is no "undefined" or "best-effort" category. A denial under degraded knowledge
is always the safe outcome, because a capacity guardian that admits when it
cannot measure has failed its only job.

**Why this priority**: Without fail-safe behaviour, the webhook becomes a
liability under partial failure — it would admit workloads it cannot evaluate,
which is worse than having no webhook at all. This is non-negotiable but ranks
after P1/P2 because it governs failure behaviour rather than delivering the
primary capability.

**Independent Test**: Simulate each failure condition (capacity data
unavailable, component down, timeout, malformed AdmissionReview) and assert that
each results in a rejection — never an admission — with a logged reason.

**Acceptance Scenarios**:

1. **Given** the capacity data is unavailable or stale beyond a configurable
   freshness threshold, **When** a pod is submitted, **Then** the pod is
   **rejected** (fail-closed) and the reason "capacity data unavailable/stale"
   is logged.
2. **Given** a component in the capacity-tracking pipeline is down or
   unresponsive, **When** a pod is submitted, **Then** the pod is **rejected**
   and the reason identifies which component was unreachable.
3. **Given** an admission request is malformed or cannot be deserialised,
   **When** it arrives, **Then** it is **rejected** and the reason identifies
   the deserialisation failure.
4. **Given** the admission decision exceeds a configurable timeout,
   **When** the timeout fires, **Then** the request is **rejected** rather than
   left pending or defaulting to admit.
5. **Given** any error type not explicitly mapped to a declared exception,
   **When** it occurs, **Then** the request is **rejected** — unknown error
   types reject by default; there is no third category.

---

### Edge Cases

- **Pod with no resource requests**: treated as consuming 0 CPU / 0 RAM. Admitted
  unless other pods already exceed the budget (the no-request pod is not the one
  that pushes it over).
- **Pod requesting exactly the remaining budget**: admitted — the ceiling is
  inclusive (allocation == ceiling is allowed; allocation > ceiling is not).
- **Pod with resource limits but no requests**: per Kubernetes semantics, limits
  without requests default `requests = limits` for scheduling. The system uses
  the same convention so its accounting matches the scheduler.
- **Budget ceiling set to 100%**: the webhook still tracks and reports
  allocation; it only rejects when total requests genuinely exceed total
  allocatable (overcommit beyond physical capacity).
- **Budget ceiling set to 0%**: every pod requesting >0 resources is rejected.
  This is a valid "circuit-breaker" configuration, not a bug.
- **Cluster with zero nodes**: total capacity is 0. Any pod requesting >0 is
  rejected. A pod requesting 0 is admitted (vacuous).
- **Node removed while a pod is pending**: capacity drops; the next admission
  check uses the updated (lower) total. Already-running pods are not evicted by
  the webhook — the webhook controls admission, not the pod lifecycle.
- **Multiple resource types exceeded simultaneously**: the rejection message
  lists all violated resources (e.g., "CPU would reach 85/80, RAM would reach
  165/160"), not just the first.
- **Very large cluster (thousands of nodes/pods)**: the system must remain
  within its performance targets (see Success Criteria) despite the volume of
  capacity data to aggregate.
- **Webhook itself is being deployed (bootstrap problem)**: the webhook's own
  pod may be subject to its own admission check. This is handled at deployment
  time (namespace exclusion or priority), not by special-casing the admission
  logic.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST track total cluster allocatable capacity (CPU and
  RAM) by observing node state, and update the tracked total when nodes are
  added or removed — without interrupting the node lifecycle.
- **FR-002**: The system MUST track current allocation (CPU and RAM) by summing
  declared resource requests across all scheduled pods, updating as pods are
  created, updated, and deleted.
- **FR-003**: The system MUST expose a configurable capacity budget expressed as
  a percentage of total cluster allocatable capacity, applied cluster-wide to
  both CPU and RAM independently.
- **FR-004**: The system MUST evaluate every pod creation and update against the
  remaining budget, admitting the pod if its requests fit within the remaining
  capacity for all tracked resource types, and rejecting it if any resource would
  exceed its ceiling.
- **FR-005**: The system MUST use declared pod resource requests (not live
  runtime usage) as the consumption metric, applying Kubernetes defaulting
  conventions (limits without requests default to `requests = limits`) so that
  accounting is consistent with the kube-scheduler.
- **FR-006**: The system MUST reject any admission request it cannot
  authoritatively evaluate — including but not limited to: capacity data
  unavailable or stale, a component unreachable, request deserialisation
  failure, or decision timeout. There MUST be no code path that admits under
  degraded or unknown conditions.
- **FR-007**: Every rejection MUST carry a human-readable message identifying
  the reason (over-budget with figures, or the specific failure mode) sufficient
  for the workload owner or operator to take action.
- **FR-008**: The system MUST emit structured logs for every admission decision
  (admit and deny) containing the workload identity, the decision, the resource
  type(s), and the capacity figures used.
- **FR-009**: The system MUST expose metrics covering: admission verdicts
  (allow/deny/error) broken down by reason, decision latency distribution,
  capacity data freshness, and current capacity utilisation per resource type
  (CPU, RAM) against the budget ceiling.
- **FR-010**: The system MUST separate capacity-supply tracking (node lifecycle)
  from capacity-consumption enforcement (pod lifecycle) into distinct
  components with a single responsibility each, linked by shared state — so
  that each is independently testable and independently failureable.
- **FR-011**: The capacity budget configuration MUST be adjustable at runtime
  (not compiled in or requiring a restart), so operators can tighten or loosen
  the ceiling in response to cluster conditions without downtime.
- **FR-012**: The system MUST support the three most recent major Kubernetes
  releases (N, N-1, N-2), using only APIs that are stable (GA) across the entire
  supported window.

### Key Entities *(include if feature involves data)*

- **Cluster Capacity**: the total allocatable CPU and RAM across all nodes in the
  cluster, aggregated from node state. Changes as nodes are added or removed.
  This is the "supply" side of the budget.
- **Allocation**: the sum of declared resource requests across all scheduled
  pods, tracked for CPU and RAM independently. Changes as pods are created,
  updated, or deleted. This is the "demand" side of the budget.
- **Budget Ceiling**: the configurable percentage of cluster allocatable capacity
  that defines the maximum allowed allocation. Applied cluster-wide, per resource
  type. This is the policy that the system enforces.
- **Admission Decision**: the verdict (admit/deny) for a specific pod
  submission, accompanied by the capacity figures that were evaluated and, for
  denials, the reason. This is the output of the enforcement logic.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A pod whose declared requests fit within the remaining budget is
  admitted in every case; a pod that would exceed the ceiling is rejected in
  every case — 100% deterministic enforcement with no "best-effort" path.
- **SC-002**: An operator can determine, from any single rejection message
  alone, which resource was violated and the exact figures (current, requested,
  projected, ceiling) — without consulting logs or contacting the platform team.
- **SC-003**: An operator can determine current cluster capacity utilisation
  (percentage of budget consumed, per resource type) from the metrics endpoint
  at any time, and this figure matches the state used by the most recent
  admission decision.
- **SC-004**: Under every enumerated failure condition (capacity data
  unavailable, component down, timeout, malformed request), the system rejects
  the admission request — zero cases of admission under degraded knowledge.
- **SC-005**: The admission decision completes within the performance budget
  (provisional: p99 under 100 ms excluding apiserver overhead, p50 under 50 ms)
  so the webhook does not become a deployment bottleneck.
- **SC-006**: The system runs within a resource footprint (provisional: under
  256 MiB memory request, under 500 millicores CPU request) so it is economical
  to run even on small clusters.
- **SC-007**: The system operates correctly across the three most recent
  Kubernetes releases without API breakage — verified by CI against each version
  in the support window.

## Assumptions

- **Target users** are cluster operators, SREs, and platform engineers running
  Kubernetes clusters where overcommit risk is a concern. They are comfortable
  with Kubernetes concepts (pods, requests/limits, admission webhooks) but
  should not need to read source code to understand why a pod was rejected.
- **Cluster scope is v1**: the budget is applied cluster-wide. Per-node,
  per-namespace, or per-resource-pool partitioning is a deliberately deferred
  future concern, not a v1 gap.
- **Resource accounting uses declared requests**: consistent with the
  kube-scheduler's reservation model. Live usage (metrics-server) is not used —
  the admission decision must be deterministic and must not depend on a
  metrics pipeline being healthy.
- **Single binary, multiple roles**: all capacity-tracking and admission
  components run within one process, deployed as one workload. Splitting into
  separate binaries is a future concern if independent scaling is needed.
- **The webhook controls admission, not lifecycle**: it does not evict running
  pods, drain nodes, or modify the cluster topology. It only gates new pod
  creation and updates. Remediation of existing overcommit is an operator
  decision.
- **Validating-only**: the webhook validates but never mutates admission
  requests. It does not modify the pod object; it only allows or denies.
- **Performance and footprint targets are provisional**: the exact thresholds
  (p99 < 100 ms, < 256 MiB, etc.) are to be ratified during planning against
  realistic cluster sizes and may be adjusted with documented rationale.
- **TLS/cert provisioning** for the webhook endpoint is a deployment concern
  (cert-manager or a provided Secret), handled in the plan — not a feature gap.
