# Contract: ClusterCapacity CRD (spec-007 delta)

> **Delta on spec-006**. The `spec.nodeSelector` singular field is replaced by
> `spec.nodeSelectors` (array). Status fields are unchanged.

## Spec Change

### `spec.nodeSelectors` (replaces `spec.nodeSelector`)

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `nodeSelectors` | `array<LabelSelector>` | no | List of label selectors. A node matching ANY selector is excluded (OR semantics). Each selector uses standard K8s LabelSelector semantics internally (AND). Absent or empty = no label exclusion. |

**Example**: exclude control-plane + experimental nodes simultaneously:

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

## Migration from spec-006

The singular `nodeSelector` field is removed. Replace:

```yaml
# Before (spec-006)
spec:
  nodeSelector:
    matchExpressions:
      - key: node-role.kubernetes.io/control-plane
        operator: Exists

# After (spec-007)
spec:
  nodeSelectors:
    - matchExpressions:
        - key: node-role.kubernetes.io/control-plane
          operator: Exists
```

Wrap the single selector in a list. No data migration webhook needed — the field
is optional and defaults to no selectors.

## Status (unchanged from spec-006)

`excludedNodeCount`, `excludedByUnschedulable`, `excludedBySelector` — same
semantics. A node matching multiple selectors is counted once.
