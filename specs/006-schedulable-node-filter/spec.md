# Feature Specification: Schedulable Node Filter

**Feature Branch**: `006-schedulable-node-filter`

**Created**: 2026-07-27

**Status**: Draft

**Input**: The operator currently counts all nodes — including control-plane
masters and cordoned nodes — into the cluster capacity pool. This inflates the
reported schedulable capacity beyond what the kube-scheduler can actually place
workloads on, because those nodes either carry `NoSchedule` taints or are marked
`spec.unschedulable`. The fix: exclude unschedulable nodes by default and give
operators a configurable label-selector to exclude arbitrary node subsets
(e.g. control-plane nodes).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Cordoned Nodes Excluded by Default (Priority: P1)

As a cluster operator, when I cordon a node (setting `spec.unschedulable = true`),
I expect the cluster capacity pool to drop immediately — that node's CPU and RAM
are no longer available for scheduling, so they must not be counted toward the
budget the admission webhook enforces. Conversely, when I uncordon the node, its
capacity returns to the pool.

This is the correctness foundation: the reported capacity must match what the
kube-scheduler considers schedulable. Without it, the webhook permits more
workloads than the cluster can actually place, defeating its purpose.

**Why this priority**: an over-reported capacity budget is a direct safety
violation (Constitution Principle II). Every cordon event — routine
maintenance, node drain, rolling upgrade — currently leaves phantom capacity in
the pool. This is the most common operational scenario and the highest-impact
fix.

**Independent Test**: cordon a node in a test cluster; verify the
ClusterCapacity status reflects the reduced node count and CPU/memory. Uncordon;
verify it returns. No configuration needed — this is the default behaviour.

**Acceptance Scenarios**:

1. **Given** a cluster with 3 schedulable nodes (total 48 CPU, 96 Gi RAM),
   **When** one node is cordoned (`spec.unschedulable = true`),
   **Then** the ClusterCapacity status drops to 2 nodes with the corresponding
   CPU/memory total, within the controller's normal reconciliation latency.
2. **Given** a cluster with 1 cordoned node and 2 schedulable nodes,
   **When** the cordoned node is uncordoned (`spec.unschedulable = false`),
   **Then** the ClusterCapacity status rises back to 3 nodes.
3. **Given** a cluster where ALL nodes are cordoned,
   **When** the controller reconciles,
   **Then** the ClusterCapacity status reports zero CPU, zero memory, zero
   nodes — and the admission webhook fails closed on all subsequent admission
   requests (no capacity data to validate against).

---

### User Story 2 - Label-Selector Exclusion for Arbitrary Node Subsets (Priority: P2)

As a cluster operator, I want to configure a Kubernetes label selector on the
ClusterCapacity CRD so that nodes matching the selector are excluded from the
capacity pool. This lets me exclude control-plane/master nodes (which carry
`node-role.kubernetes.io/control-plane`), dedicated nodes for system
workloads, or any other node subset I do not want user workloads scheduled on.

The selector is additive to the default unschedulable exclusion: a node is
counted only if it is schedulable AND does not match the label selector.

**Why this priority**: control-plane nodes are present in every cluster and
their capacity is never available to user workloads (they carry `NoSchedule`
taints). Without excluding them, the capacity pool is permanently inflated by
their resources. This is the second most common scenario after cordon exclusion.

**Independent Test**: configure the label selector
`node-role.kubernetes.io/control-plane: Exists` on the ClusterCapacity CRD;
verify that control-plane nodes are excluded from the capacity sum. Remove the
selector; verify they return.

**Acceptance Scenarios**:

1. **Given** a cluster with 2 worker nodes and 1 control-plane node carrying
   the label `node-role.kubernetes.io/control-plane`,
   **When** the operator sets a label selector
   `node-role.kubernetes.io/control-plane: Exists` on the ClusterCapacity CRD,
   **Then** the control-plane node's capacity is excluded — the status reports
   only the 2 worker nodes.
2. **Given** the selector from scenario 1 is active,
   **When** the operator removes the selector (sets it to empty/absent),
   **Then** all schedulable nodes are counted again, including the
   control-plane node (unless it is separately cordoned).
3. **Given** a selector `node-role.kubernetes.io/control-plane: Exists` is
   active and a new worker node joins the cluster,
   **When** the controller observes the new node,
   **Then** the new worker node's capacity is added (it does not match the
   selector) — only the selection criteria, not a point-in-time node list,
   determines inclusion.
4. **Given** a selector is active and a node that was previously excluded
   by the selector has its matching label removed,
   **When** the controller observes the label change,
   **Then** the node's capacity is added back to the pool.

---

### User Story 3 - Observability of Excluded Nodes (Priority: P3)

As a cluster operator, I want the ClusterCapacity status to show not just how
many nodes are counted but how many were excluded, so I can verify the filter is
behaving correctly. Without this, a misconfigured selector or an unexpected
cordon is invisible — the capacity figure drops but I cannot tell why.

**Why this priority**: Principle IV (Observability Before Optimisation). This is
not a functional requirement for correctness — the filter works without it —
but it is essential for operational confidence. Without observability, operators
cannot trust or debug the filter.

**Independent Test**: cordon a node and set a label selector; verify the
ClusterCapacity status reports both the included node count and the excluded
node count, matching expectations.

**Acceptance Scenarios**:

1. **Given** a cluster with 5 nodes — 1 cordoned, 1 matching a label selector,
   and 3 matching neither,
   **When** the controller reconciles,
   **Then** the ClusterCapacity status reports 3 counted nodes and 2 excluded
   nodes (1 unschedulable + 1 label-matched).
2. **Given** the ClusterCapacity status reports excluded node counts,
   **When** an operator inspects the status (via `kubectl describe`),
   **Then** both the total node count and the excluded breakdown are visible
   without requiring access to metrics endpoints.

---

### Edge Cases

- **All nodes excluded** (all cordoned or all matching the selector): capacity
  drops to zero. The webhook fails closed on all admissions — this is correct
  (Constitution Principle I): no verifiable capacity means no admission.
- **Label selector matches no nodes**: no additional exclusion occurs — all
  schedulable nodes are counted. The selector is a no-op, not an error.
- **Empty or absent label selector**: equivalent to "no selector" — only the
  default unschedulable exclusion applies. This preserves backward
  compatibility with existing clusters that have no selector configured.
- **Node transitions during a watch**: cordon/uncordon/label-change events fire
  in real-time through the node watcher; the controller recomputes the
  aggregate on every event, so the status converges without polling delay.
- **Node missing `.status.allocatable`**: already handled (contributes zero);
  the exclusion filter is orthogonal — such a node is still subject to
  unschedulable/label checks for counting purposes.
- **Invalid or unparseable label selector on the CRD**: the controller must not
  crash or silently ignore it. It logs the error and falls back to the default
  behaviour (unschedulable-only exclusion), so a misconfiguration degrades
  gracefully rather than halting capacity tracking.
- **CRD spec changed at runtime**: the label selector is read from the
  ClusterCapacity CRD spec on every reconciliation cycle, so a
  `kubectl patch` takes effect on the next node event without a controller
  restart — consistent with how the Allocation CRD threshold already works.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Node Capacity Controller MUST exclude any node with
  `spec.unschedulable = true` from the cluster capacity aggregate.
- **FR-002**: The Node Capacity Controller MUST count a node only if it is
  schedulable (`spec.unschedulable` is absent or `false`).
- **FR-003**: The ClusterCapacity CRD spec MUST accept an optional label
  selector field. When present, nodes matching the selector MUST be excluded
  from the capacity aggregate.
- **FR-004**: A node MUST be counted only if it is schedulable AND does not
  match the configured label selector. Both conditions are required for
  inclusion.
- **FR-005**: When the label selector is absent or empty, only the
  unschedulable exclusion applies (backward-compatible default behaviour).
- **FR-006**: The label selector MUST follow Kubernetes LabelSelector semantics
  (matchLabels + matchExpressions) so operators can use the same selector
  syntax they already know from node affinity, deployments, etc.
- **FR-007**: The Node Capacity Controller MUST re-evaluate the label selector
  on every node event, not cache the node set at startup — so label changes,
  cordons, and new nodes converge in real-time.
- **FR-008**: The ClusterCapacity status MUST report the count of nodes
  excluded from the aggregate (in addition to the count of included nodes), so
  operators can verify the filter behaviour without inspecting metrics.
- **FR-009**: The ClusterCapacity status MUST report a breakdown of why nodes
  were excluded — at minimum, how many were unschedulable vs label-matched —
  so operators can distinguish cordon-driven exclusions from
  selector-driven ones.
- **FR-010**: If the configured label selector is invalid or unparseable, the
  controller MUST log a warning and fall back to unschedulable-only exclusion
  rather than crashing or silently skipping reconciliation.
- **FR-011**: The label selector MUST be runtime-configurable via the
  ClusterCapacity CRD spec — changes take effect on the next reconciliation
  cycle without a controller restart.
- **FR-012**: The exclusion behaviour MUST be documented in README.md,
  including the default unschedulable exclusion, the label-selector
  configuration, and the status fields for observability (Constitution
  Principle X).

### Key Entities *(include if feature involves data)*

- **ClusterCapacity CRD (modified)**: gains an optional label-selector field
  in its `spec` (previously empty), and new observability fields in its
  `status` (excluded node counts and exclusion-reason breakdown). The spec
  change is the first user-facing configuration field on this CRD; the status
  change extends the existing supply-side observability.
- **Node (Kubernetes Node, unmodified)**: the controller reads
  `spec.unschedulable` and `metadata.labels` — both standard Kubernetes fields.
  No node mutation occurs (Constitution Principle V: read-only on nodes).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A cluster with cordoned nodes reports a strictly lower capacity
  aggregate than the same cluster with those nodes schedulable — the cordon
  exclusion is observable in the ClusterCapacity status.
- **SC-002**: A cluster with control-plane nodes excluded via label selector
  reports a capacity aggregate matching only the worker nodes — verified
  against the sum of worker-node allocatable figures.
- **SC-003**: The controller reconciles a cordon/uncordon or label-change event
  into an updated ClusterCapacity status within the normal watcher event
  latency (same order as the existing capacity-tracking latency).
- **SC-004**: The ClusterCapacity status distinguishes included vs excluded
  node counts, so an operator can confirm at a glance that the filter is active
  and excluding the expected number of nodes.
- **SC-005**: Existing clusters with no label selector configured experience
  no behavioural regression — the only change is that cordoned nodes are now
  excluded (the pre-feature bug fix), and the status gains new fields without
  losing existing ones.

## Assumptions

- The label selector uses standard Kubernetes `LabelSelector` semantics
  (matchLabels + matchExpressions), as defined in
  `apimachinery/pkg/apis/meta/v1`. No custom selector dialect is introduced.
- The default unschedulable exclusion is always active and cannot be disabled —
  there is no configuration to "count cordoned nodes" because that would
  reintroduce the original bug.
- The label selector is optional; absent or empty means "no additional
  exclusion beyond unschedulable." This is the backward-compatible default.
- Node taints (`NoSchedule`, `NoExecute`) are intentionally NOT replicated in
  the exclusion logic. Taint/toleration matching is the kube-scheduler's
  responsibility (Constitution Principle V: separated concerns). A tainted but
  schedulable node with no matching label selector is counted — if the operator
  needs to exclude it, they use the label selector (e.g. on the taint's
  corresponding label) or cordon it.
- The exclusion applies only to the supply side (Node Capacity Controller →
  ClusterCapacity status). The demand side (Allocation Controller, pod requests)
  is unaffected — pods already running on excluded nodes are still counted
  against the budget (they consume real resources regardless of node
  schedulability).
- The new status fields are additive — existing consumers of the
  ClusterCapacity status (the Allocation Controller, the webhook) ignore fields
  they do not read, so the change is forward-compatible.
