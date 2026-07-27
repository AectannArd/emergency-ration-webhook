# Data Model — Multi-Selector Node Exclusion

## 1. CRD Spec Change

### Before (spec-006)

```rust
pub struct ClusterCapacitySpec {
    pub node_selector: Option<LabelSelector>,
}
```

### After (spec-007)

```rust
pub struct ClusterCapacitySpec {
    /// Optional list of label selectors for excluding nodes from the capacity
    /// aggregate (spec-007). A node matching ANY selector is excluded. Each
    /// selector internally ANDs its matchLabels/matchExpressions (standard K8s
    /// semantics); the list-level result is OR.
    pub node_selectors: Option<Vec<LabelSelector>>,
}
```

**JSON field**: `nodeSelectors` (camelCase) — an array of LabelSelector objects.

```yaml
spec:
  nodeSelectors:
    - matchExpressions:
        - key: node-role.kubernetes.io/control-plane
          operator: Exists
    - matchExpressions:
        - key: node-type/experimental
          operator: Exists
```

## 2. Status (unchanged)

The three `ExclusionBreakdown` fields from spec-006 are unchanged:
`excluded_node_count`, `excluded_by_unschedulable`, `excluded_by_selector`.
A node matching multiple selectors is counted once under `excluded_by_selector`.

## 3. Modified Functions

### 3.1 `is_node_counted` — signature change

```rust
// Before (spec-006):
pub fn is_node_counted(
    unschedulable: bool,
    labels: Option<&BTreeMap<String, String>>,
    selector: Option<&LabelSelector>,
) -> bool

// After (spec-007):
pub fn is_node_counted(
    unschedulable: bool,
    labels: Option<&BTreeMap<String, String>>,
    selectors: Option<&[LabelSelector]>,
) -> bool
```

Algorithm:
```
1. if unschedulable → false (unchanged)
2. let Some(sels) = selectors, not empty → else return true
3. let Some(node_labels) = labels → else return true (can't match)
4. return !labels_match_any_selector(node_labels, sels)
```

### 3.2 NEW: `labels_match_any_selector`

```rust
fn labels_match_any_selector(
    labels: &BTreeMap<String, String>,
    selectors: &[LabelSelector],
) -> bool {
    selectors.iter().any(|sel| labels_match_selector(labels, sel))
}
```

Reuses spec-006's `labels_match_selector` per-selector. OR via `iter().any()`.

### 3.3 `effective_selectors` — replaces `effective_selector`

```rust
fn effective_selectors(selectors: Option<&[LabelSelector]>) -> Vec<&LabelSelector> {
    // Filter out invalid selectors, logging a warning for each.
    // Returns the validated subset.
}
```

### 3.4 `sum_node_allocatable` — signature change

```rust
// Before (spec-006):
pub fn sum_node_allocatable<'a, I>(
    nodes: I,
    selector: Option<&LabelSelector>,
) -> (i64, i64, i32, ExclusionBreakdown)

// After (spec-007):
pub fn sum_node_allocatable<'a, I>(
    nodes: I,
    selectors: Option<&[LabelSelector]>,
) -> (i64, i64, i32, ExclusionBreakdown)
```

### 3.5 `read_selectors` — replaces `read_selector`

```rust
async fn read_selectors(capacity_api: &Api<ClusterCapacity>) -> Option<Vec<LabelSelector>> {
    match capacity_api.get(CLUSTER_CAPACITY_NAME).await {
        Ok(cc) => cc.spec.node_selectors,
        // ... same error handling as spec-006's read_selector
    }
}
```

## 4. CRD YAML Delta (deploy/crds.yaml)

The `nodeSelector` property becomes `nodeSelectors` (array):

```yaml
# Before (spec-006):
                nodeSelector:
                  type: object
                  ...

# After (spec-007):
                nodeSelectors:
                  type: array
                  description: >
                    Optional list of label selectors for excluding nodes.
                    A node matching ANY selector is excluded (OR semantics).
                    Each selector uses standard LabelSelector semantics.
                  items:
                    type: object
                    properties:
                      matchLabels:
                        type: object
                        additionalProperties:
                          type: string
                      matchExpressions:
                        type: array
                        items:
                          type: object
                          required: ["key", "operator"]
                          properties:
                            key:
                              type: string
                            operator:
                              type: string
                            values:
                              type: array
                              items:
                                type: string
```

## 5. Test Matrix

| Test | Type | What it proves |
|------|------|----------------|
| `labels_match_any_selector` — matches any | unit | node matching 1 of 3 selectors → true |
| `labels_match_any_selector` — matches none | unit | node matching 0 of 3 → false |
| `labels_match_any_selector` — empty list | unit | false (no selectors → no match) |
| `is_node_counted` — multi-selector match | unit | excluded if matching ANY |
| `is_node_counted` — multi-selector no match | unit | counted if matching none |
| `sum_node_allocatable` — 2 selectors, 2 groups | unit | both groups excluded |
| `sum_node_allocatable` — node matches both selectors | unit | counted once in breakdown |
| `effective_selectors` — skips invalid, keeps valid | unit | mixed list → valid only |
| `effective_selectors` — all invalid | unit | empty vec → unschedulable-only |
| US1: control-plane + experimental excluded | BDD | OR semantics end-to-end |
