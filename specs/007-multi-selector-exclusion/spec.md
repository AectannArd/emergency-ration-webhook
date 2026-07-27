# Feature Specification: Multi-Selector Node Exclusion

**Feature Branch**: `007-multi-selector-exclusion`

**Created**: 2026-07-27

**Status**: Draft

**Input**: Spec-006 introduced a single optional `LabelSelector` on the
`ClusterCapacity` CRD spec for excluding nodes by label. However, a single
`LabelSelector` ANDs all its requirements — it cannot express OR across
different label keys. Operators who need to exclude nodes from multiple
independent label criteria (e.g. control-plane nodes by role AND experimental
nodes by a custom label) must apply a shared exclusion label first, which is an
unnecessary operational burden. This feature adds support for multiple selectors
that are ORed together, so a node is excluded if it matches ANY one of them.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Exclude Nodes Matching ANY of Several Label Criteria (Priority: P1)

As a cluster operator, I want to configure multiple label selectors on the
`ClusterCapacity` CRD so that a node is excluded if it matches ANY one of them.
For example, I want to exclude both control-plane nodes (by
`node-role.kubernetes.io/control-plane: Exists`) and experimental nodes (by
`node-type/experimental: Exists`) without having to apply a shared label to both
sets first.

**Why this priority**: this is the core feature — without OR semantics, operators
cannot express multi-criteria exclusion without workarounds (shared labels).
This is the only user story; it IS the feature.

**Independent Test**: configure two selectors on the ClusterCapacity CRD, one
matching control-plane nodes and one matching experimental nodes. Verify that
nodes matching either selector are excluded from the capacity aggregate.

**Acceptance Scenarios**:

1. **Given** a cluster with 5 nodes — 2 workers, 1 control-plane (label
   `node-role.kubernetes.io/control-plane`), 1 experimental (label
   `node-type/experimental`), and 1 cordoned,
   **When** the operator configures two selectors:
   `{matchExpressions: [{key: node-role.kubernetes.io/control-plane, operator: Exists}]}`
   and
   `{matchExpressions: [{key: node-type/experimental, operator: Exists}]}`,
   **Then** both the control-plane and experimental nodes are excluded — the
   capacity aggregate reflects only the 2 worker nodes. The cordoned node is
   excluded by the default unschedulable rule (unchanged from spec-006).
2. **Given** two selectors are configured,
   **When** a node carries BOTH labels (matches both selectors),
   **Then** it is excluded (counted once, not double-counted).
3. **Given** two selectors are configured,
   **When** a new node joins the cluster matching neither selector,
   **Then** the new node is counted toward capacity (if schedulable).
4. **Given** two selectors are configured,
   **When** the operator removes one selector (reducing to a single-selector
   configuration),
   **Then** only nodes matching the remaining selector are excluded by label —
   nodes that were previously excluded only by the removed selector are now
   counted.

---

### Edge Cases

- **Empty selector list**: equivalent to "no label exclusion" — only
  unschedulable nodes are excluded. This is the backward-compatible default
  (same as `nodeSelector: None` in spec-006).
- **Single selector in the list**: functionally identical to spec-006's
  `nodeSelector`. The migration path is straightforward.
- **Selector list with an empty selector (`{}`)**: an empty LabelSelector
  matches ALL nodes. If present in the list, ALL schedulable nodes are excluded
  → capacity drops to zero. This is valid (an operator might want to exclude
  everything temporarily) but should be documented as a sharp edge.
- **Selector list with an invalid selector**: same fallback as spec-006 — the
  controller logs a warning for the invalid selector and skips it (the other
  selectors still apply). If ALL selectors are invalid, the controller falls
  back to unschedulable-only exclusion.
- **Runtime transition from `nodeSelector` to `nodeSelectors`**: if an operator
  has a spec-006 `nodeSelector` configured and upgrades to spec-007, the
  migration must preserve the existing exclusion behavior. See the migration
  note in Assumptions.
- **Duplicate selectors**: two identical selectors in the list produce the same
  exclusion as one. No deduplication is needed — a node matching both is still
  excluded once.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The `ClusterCapacity` CRD spec MUST accept a list of label
  selectors (`nodeSelectors` as an array of `LabelSelector`), ORed together — a
  node is excluded if it matches ANY selector in the list.
- **FR-002**: The existing single `nodeSelector` field from spec-006 MUST be
  migrated to the new `nodeSelectors` list field without breaking existing
  deployments. See Assumptions for the migration strategy.
- **FR-003**: The default unschedulable exclusion (spec-006 FR-001) MUST remain
  unchanged and cannot be disabled.
- **FR-004**: A node MUST be counted only if it is schedulable AND does not match
  ANY selector in the `nodeSelectors` list.
- **FR-005**: When `nodeSelectors` is absent or empty, only the unschedulable
  exclusion applies (backward-compatible default).
- **FR-006**: Each selector in the list MUST follow standard Kubernetes
  `LabelSelector` semantics (matchLabels + matchExpressions, ANDed within each
  selector).
- **FR-007**: The selectors MUST be ORed across the list: if a node matches any
  one selector, it is excluded. A node need not match all selectors.
- **FR-008**: If any selector in the list is structurally invalid, the
  controller MUST log a warning for that selector and skip it — the remaining
  selectors still apply. If ALL selectors are invalid, the controller falls back
  to unschedulable-only exclusion.
- **FR-009**: The selectors MUST be runtime-configurable via the
  `ClusterCapacity` CRD spec — changes take effect on the next reconciliation
  cycle without a controller restart.
- **FR-010**: The `ClusterCapacity` status MUST continue to report the
  `excludedBySelector` count (from spec-006). A node excluded by multiple
  selectors MUST be counted once, not per-matching-selector.
- **FR-011**: The multi-selector feature MUST be documented in README.md,
  including the OR semantics, migration from the single-selector field, and
  configuration examples (Constitution Principle X).
- **FR-012**: The existing `excludedByUnschedulable` and `excludedNodeCount`
  status fields (from spec-006) MUST continue to work unchanged.

### Key Entities *(include if feature involves data)*

- **ClusterCapacity CRD (modified)**: the `spec.nodeSelector` field from
  spec-006 is replaced by `spec.nodeSelectors` (a list of `LabelSelector`). The
  status fields from spec-006 are unchanged. This is a CRD schema migration —
  see Assumptions for backward compatibility.
- **Node (Kubernetes Node, unmodified)**: the controller reads
  `spec.unschedulable` and `metadata.labels` as in spec-006.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A cluster with nodes matching different label criteria (e.g.
  control-plane by role, experimental by custom label) can exclude all of them
  simultaneously via a single `nodeSelectors` configuration, without applying a
  shared label.
- **SC-002**: The OR semantics are observable — adding a second selector that
  matches additional nodes strictly increases the excluded count (never
  decreases, since selectors are ORed, not ANDed).
- **SC-003**: An existing deployment with spec-006's single `nodeSelector`
  experiences no behavioral regression after upgrade — the migration to
  `nodeSelectors` preserves the exclusion.
- **SC-004**: Invalid selectors are logged and skipped without halting capacity
  tracking or crashing the controller.

## Assumptions

- **Migration strategy (spec-006 `nodeSelector` → `nodeSelectors`)**: the
  spec-006 singular `nodeSelector: Option<LabelSelector>` field is replaced by
  `nodeSelectors: Option<Vec<LabelSelector>>` (or a struct wrapping a list).
  Since spec-006 was just merged and there are no production deployments with the
  singular field, a clean rename (not dual-field backward compatibility) is
  acceptable. The CRD schema gains `nodeSelectors` (array) and drops
  `nodeSelector` (singular). Existing test clusters that configured
  `nodeSelector` must update to `nodeSelectors` after upgrade.
- **CRD schema migration**: because the CRD is cluster-scoped and
  controller-managed, the operator updates `deploy/crds.yaml` and re-applies.
  The controller's auto-created singleton seeds `nodeSelectors: None`. No data
  migration webhook is needed — the field is optional and defaults to "no
  selectors".
- **OR semantics are at the selector level, not the requirement level**: each
  `LabelSelector` in the list is evaluated independently (its internal
  matchLabels + matchExpressions are ANDed), and the list-level result is OR.
  This matches the most intuitive operator mental model: "exclude nodes matching
  ANY of these label patterns".
- **The `node_filter.rs` module from spec-006 is extended, not replaced**: the
  `labels_match_selector` function is reused per-selector; a new
  `labels_match_any_selector` wrapper ORs the results.
- **No new RBAC or dependencies**: the change is purely in the CRD struct and
  the filter logic. No new permissions or crates.
