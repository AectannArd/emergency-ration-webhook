# Data Model — Schedulable Node Filter

## 1. Modified CRD: ClusterCapacity

### 1.1 Spec — new `nodeSelector` field

The `ClusterCapacitySpec` gains one optional field:

```rust
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "emergency-ration.dev",
    version = "v1",
    kind = "ClusterCapacity",
    status = "ClusterCapacityStatus",
    shortname = "cc"
)]
pub struct ClusterCapacitySpec {
    /// Optional label selector for excluding nodes from the capacity
    /// aggregate. Nodes matching the selector are not counted toward
    /// total capacity. When absent or empty, only unschedulable nodes
    /// (`spec.unschedulable = true`) are excluded.
    pub node_selector: Option<LabelSelector>,
}
```

**JSON field**: `nodeSelector` (camelCase, serialized by `#[serde(rename_all =
"camelCase")]`)

**Example CRD spec with control-plane exclusion**:
```yaml
apiVersion: emergency-ration.dev/v1
kind: ClusterCapacity
metadata:
  name: cluster-capacity
spec:
  nodeSelector:
    matchExpressions:
      - key: node-role.kubernetes.io/control-plane
        operator: Exists
```

**Backward compatibility**: when `nodeSelector` is absent or `None`, the filter
uses unschedulable-only exclusion. This is the default for existing deployments.

### 1.2 Status — new observability fields

The `ClusterCapacityStatus` gains three integer fields:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCapacityStatus {
    // --- existing fields (unchanged) ---
    pub total_allocatable_cpu_milli: i64,
    pub total_allocatable_memory_bytes: i64,
    pub node_count: i32,
    pub last_updated: String,

    // --- new fields (spec-006) ---
    /// Number of nodes excluded from the aggregate (unschedulable + selector).
    pub excluded_node_count: i32,
    /// Nodes excluded because `spec.unschedulable = true`.
    pub excluded_by_unschedulable: i32,
    /// Nodes excluded because they matched the label selector.
    pub excluded_by_selector: i32,
}
```

**JSON fields**: `excludedNodeCount`, `excludedByUnschedulable`,
`excludedBySelector` (camelCase)

**Counting semantics**: a node that is both unschedulable AND selector-matched is
counted under `excludedByUnschedulable` only (unschedulable is checked first).
`excludedNodeCount = excludedByUnschedulable + excludedBySelector`. Nodes
missing `.status.allocatable` are still subject to exclusion checks for
counting, but contribute zero to CPU/memory regardless.

### 1.3 Generated CRD YAML (delta — `deploy/crds.yaml`)

The `ClusterCapacity` CRD manifest gains these properties under
`spec.versions[0].schema.openAPIV3Schema`:

```yaml
# Under .spec.properties.spec.properties:
            spec:
              type: object
              properties:
                nodeSelector:                          # NEW
                  type: object                         # NEW
                  description: >                       # NEW
                    Optional label selector for excluding
                    nodes from the capacity aggregate.
                    Uses standard Kubernetes LabelSelector
                    semantics (matchLabels + matchExpressions).
                  properties:                           # NEW
                    matchLabels:                        # NEW
                      type: object                      # NEW
                      additionalProperties:             # NEW
                        type: string                    # NEW
                    matchExpressions:                   # NEW
                      type: array                       # NEW
                      items:                            # NEW
                        type: object                    # NEW
                        required: ["key", "operator"]   # NEW
                        properties:                     # NEW
                          key:                          # NEW
                            type: string                # NEW
                          operator:                     # NEW
                            type: string                # NEW
                          values:                       # NEW
                            type: array                 # NEW
                            items:                      # NEW
                              type: string              # NEW

# Under .spec.properties.status.properties:
            status:
              type: object
              properties:
                # ... existing fields unchanged ...
                excludedNodeCount:                     # NEW
                  type: integer
                  format: int32
                  minimum: 0
                excludedByUnschedulable:               # NEW
                  type: integer
                  format: int32
                  minimum: 0
                excludedBySelector:                    # NEW
                  type: integer
                  format: int32
                  minimum: 0
```

## 2. New Module: `src/controllers/node_filter.rs`

A pure module containing the filtering decision logic. No I/O, no client, no
async — fully unit-testable (Principle VIII).

### 2.1 `is_node_counted(node, selector) -> bool`

The core predicate. Returns `true` if the node should be counted toward the
capacity aggregate.

```rust
pub fn is_node_counted(
    unschedulable: bool,
    labels: Option<&BTreeMap<String, String>>,
    selector: Option<&LabelSelector>,
) -> bool
```

**Algorithm**:
```
1. If unschedulable == true → return false (default exclusion, FR-001)
2. If selector is None or empty (no matchLabels, no matchExpressions) → return true (FR-005)
3. If selector matches the node's labels → return false (label exclusion, FR-003)
4. Otherwise → return true
```

Steps 1 and 3 are the two exclusion layers. A node is counted only if it passes
both (FR-004).

### 2.2 `labels_match_selector(labels, selector) -> bool`

Pure label-matching function implementing Kubernetes LabelSelector semantics
(research R2):

```rust
fn labels_match_selector(
    labels: &BTreeMap<String, String>,
    selector: &LabelSelector,
) -> bool
```

- An **empty selector** (`matchLabels` is None/empty AND `matchExpressions` is
  None/empty) matches **all** nodes → returns `true`. This is the Kubernetes
  convention: an empty LabelSelector is a wildcard.
- `matchLabels`: every `{key, value}` must be present in `labels`.
- `matchExpressions`: each requirement is evaluated and the results are ANDed.

### 2.3 `validate_selector(selector) -> Result<(), SelectorError>`

Structural validation of a `LabelSelector` (research R4):

```rust
pub fn validate_selector(selector: &LabelSelector) -> Result<(), SelectorError>
```

Checks:
- Each `matchExpressions` entry has `operator` in
  `{"In", "NotIn", "Exists", "DoesNotExist"}`
- `In`/`NotIn` entries have non-empty `values`
- `Exists`/`DoesNotExist` entries have empty or absent `values`

Returns `Err(SelectorError::...)` on violation. Called before label matching;
an error triggers the fallback to unschedulable-only exclusion.

```rust
#[derive(Debug, thiserror::Error)]
pub enum SelectorError {
    #[error("unknown operator '{0}' in matchExpression for key '{1}'")]
    UnknownOperator(String, String),
    #[error("operator '{operator}' requires non-empty values for key '{key}'")]
    MissingValues { operator: String, key: String },
    #[error("operator '{operator}' must have empty values for key '{key}'")]
    UnexpectedValues { operator: String, key: String },
}
```

### 2.4 `ExclusionBreakdown` summary struct

Returned by the aggregate function so the controller can populate status:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionBreakdown {
    pub counted: i32,
    pub excluded_unschedulable: i32,
    pub excluded_by_selector: i32,
}
```

`excluded_node_count = excluded_unschedulable + excluded_by_selector`.

## 3. Modified Function: `sum_node_allocatable`

The existing `sum_node_allocatable` in `node_capacity.rs` gains a selector
parameter and returns the exclusion breakdown alongside the existing CPU/memory
counts. The new signature:

```rust
pub fn sum_node_allocatable<'a, I>(
    nodes: I,
    selector: Option<&LabelSelector>,
) -> (i64, i64, i32, ExclusionBreakdown)
where
    I: IntoIterator<Item = &'a Node>,
```

Returns `(cpu_milli, memory_bytes, counted_node_count, breakdown)`.

**Algorithm** (pseudocode):
```
for each node:
    unschedulable = node.spec.unschedulable.unwrap_or(false)
    labels = node.metadata.labels.as_ref()

    if unschedulable:
        breakdown.excluded_unschedulable += 1
        continue

    if selector is Some(sel) and validate_selector(sel).is_ok()
       and labels_match_selector(labels, sel):
        breakdown.excluded_by_selector += 1
        continue

    // node is counted: sum its allocatable
    if let Some(allocatable) = node.status.allocatable:
        cpu += parse_cpu(allocatable["cpu"])
        memory += parse_memory(allocatable["memory"])
    counted += 1
```

If `validate_selector` returns `Err`, the selector is ignored for this cycle
(fallback to unschedulable-only) and the controller logs a warning. The
selector is re-validated on every reconciliation — if the operator fixes the
selector, it takes effect immediately.

## 4. Controller Reconciliation Flow (modified)

The Node Capacity Controller's `run()` function gains a read of the
`ClusterCapacity` spec to obtain the selector on each reconciliation:

```
For each node watcher event:
  1. snapshot = reflector store state
  2. Read ClusterCapacity singleton spec → selector = spec.nodeSelector
  3. (cpu, mem, count, breakdown) = sum_node_allocatable(snapshot, selector)
  4. Patch status with: total_allocatable_cpu_milli, total_allocatable_memory_bytes,
     node_count, excluded_node_count, excluded_by_unschedulable,
     excluded_by_selector, last_updated
```

The selector is read from the CRD spec (not cached at startup) so runtime
changes via `kubectl patch` take effect on the next node event (FR-007, FR-011).

**Singleton autocreation**: `default_capacity_singleton()` now creates a
`ClusterCapacitySpec { node_selector: None }`. The existing singleton is never
overwritten — an operator-set `nodeSelector` is preserved (same as the
Allocation controller preserves `budgetPercent`).

## 5. Error Paths

| Condition | Behaviour | Principle |
|-----------|----------|-----------|
| `nodeSelector` absent or `None` | Unschedulable-only exclusion (default) | FR-005 |
| `nodeSelector` present but empty (`{}`) | Matches all nodes → no label exclusion; only unschedulable excluded | FR-005 (K8s convention) |
| `nodeSelector` structurally invalid | `warn!` log, fallback to unschedulable-only for this cycle | FR-010, Principle III |
| Node missing `metadata.labels` | `labels_match_selector` returns `false` for non-empty selectors (no labels → no match) → node is counted (if schedulable) | Edge case |
| Node missing `.status.allocatable` | Excluded from CPU/memory sum but still counted in `node_count` if schedulable+non-matching | Existing behaviour |
| All nodes excluded | `cpu=0, mem=0, count=0` → webhook fails closed (no capacity data) | Principle I |

## 6. Data Flow Diagram

```
┌─────────────────┐
│  ClusterCapacity │
│  CRD spec:       │      selector read on each reconcile
│   nodeSelector ──┼──────────────────────────┐
│  CRD status:     │                          │
│   totalCpu/Mem   │                          ▼
│   nodeCount      │    ┌───────────────────────────────────┐
│   excluded*  ◄───┼────┤  Node Capacity Controller          │
└─────────────────┘    │  sum_node_allocatable(nodes, sel)  │
                       │    for each node:                  │
                       │      unschedulable? → exclude      │
  ┌───────────────┐    │      selector match? → exclude     │
  │ Node watcher  │───▶│      else: sum allocatable         │
  │ (reflector)   │    │    → patch_status(cpu,mem,count,   │
  └───────────────┘    │       excluded_*)                  │
                       └───────────────────────────────────┘
```

## 7. Test Matrix

| Test | Type | What it proves |
|------|------|----------------|
| `is_node_counted` — schedulable, no selector | unit | returns true |
| `is_node_counted` — unschedulable | unit | returns false (FR-001) |
| `is_node_counted` — schedulable, selector matches | unit | returns false (FR-003) |
| `is_node_counted` — schedulable, selector doesn't match | unit | returns true |
| `labels_match_selector` — matchLabels hit | unit | true |
| `labels_match_selector` — matchLabels miss | unit | false |
| `labels_match_selector` — In operator | unit | true/false |
| `labels_match_selector` — NotIn operator | unit | true/false |
| `labels_match_selector` — Exists operator | unit | true/false |
| `labels_match_selector` — DoesNotExist operator | unit | true/false |
| `labels_match_selector` — empty selector | unit | true (matches all) |
| `validate_selector` — valid | unit | Ok |
| `validate_selector` — unknown operator | unit | Err (FR-010) |
| `validate_selector` — In without values | unit | Err (FR-010) |
| `sum_node_allocatable` — mixed nodes + selector | unit | correct CPU/mem/count/breakdown |
| `sum_node_allocatable` — all unschedulable | unit | zeros (Principle I) |
| `sum_node_allocatable` — invalid selector fallback | unit | unschedulable-only |
| reconcile with cordon event | integration (mock apiserver) | status updates on cordon |
| reconcile with label-selector change | integration (mock apiserver) | status updates on spec patch |
| US1: cordon excludes node | BDD | P1 acceptance |
| US2: selector excludes control-plane | BDD | P2 acceptance |
| US3: status shows excluded counts | BDD | P3 acceptance |
