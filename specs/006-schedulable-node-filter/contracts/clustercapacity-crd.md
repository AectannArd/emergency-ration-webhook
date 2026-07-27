# Contract: ClusterCapacity CRD (spec-006 delta)

> **Delta document**: this describes the spec-006 changes to the existing
> `ClusterCapacity` CRD contract. The base contract lives at
> `specs/001-capacity-admission-webhook/contracts/clustercapacity-crd.md`.
> All unmodified sections from the base contract remain in force.

> **Superseded by spec-007**: the singular `spec.nodeSelector` field introduced
> here was renamed to `spec.nodeSelectors` — a **list** of selectors with OR
> semantics (a node matching any selector is excluded) — in spec-007
> (`specs/007-multi-selector-exclusion/`). This document records the spec-006
> contract as shipped; for the current field shape see
> `specs/007-multi-selector-exclusion/contracts/clustercapacity-crd.md`.

## Overview

The `ClusterCapacity` CRD gains:
- An optional **`spec.nodeSelector`** field (standard Kubernetes `LabelSelector`)
  for excluding nodes from the capacity aggregate by label.
- Three new **status** fields (`excludedNodeCount`, `excludedByUnschedulable`,
  `excludedBySelector`) reporting how many nodes were excluded and why.

The CRD version stays `v1`. The changes are additive — existing instances
without the new fields continue to function.

## Spec Schema

### `spec.nodeSelector` (NEW — optional)

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `nodeSelector` | `LabelSelector` | no | Nodes matching this selector are excluded from the capacity aggregate. Uses standard Kubernetes `LabelSelector` semantics (`matchLabels` + `matchExpressions`). When absent or empty, only unschedulable nodes are excluded. |

**LabelSelector** is the standard Kubernetes type
(`io.k8s.apimachinery.pkg.apis.meta.v1.LabelSelector`). Its schema:

```yaml
nodeSelector:
  matchLabels:           # optional: map of {key: value} pairs (ANDed)
    node-role.kubernetes.io/control-plane: ""
  matchExpressions:      # optional: list of requirements (ANDed)
    - key: node-role.kubernetes.io/control-plane
      operator: Exists   # In | NotIn | Exists | DoesNotExist
      values: []          # required for In/NotIn; empty for Exists/DoesNotExist
```

**Semantics**:
- An **empty** `nodeSelector` (`{}`) or **absent** `nodeSelector` means "no
  label-based exclusion" — only unschedulable nodes are excluded (the default).
- A non-empty selector excludes any node whose labels match the selector.
- The selector is evaluated on every reconciliation cycle; runtime changes via
  `kubectl patch` take effect on the next node event without restart.
- A structurally invalid selector (unknown operator, missing values) causes the
  controller to log a warning and fall back to unschedulable-only exclusion.

## Status Schema

### New fields (additive to existing status)

| Property | Type | Description |
|----------|------|-------------|
| `excludedNodeCount` | `integer (int32)` | Total nodes excluded from the aggregate. Equals `excludedByUnschedulable + excludedBySelector`. |
| `excludedByUnschedulable` | `integer (int32)` | Nodes excluded because `spec.unschedulable = true`. |
| `excludedBySelector` | `integer (int32)` | Nodes excluded because they matched the `spec.nodeSelector` label selector. |

**Counting semantics**: a node that is both unschedulable AND selector-matched is
counted under `excludedByUnschedulable` only (unschedulable is checked first).
This prevents double-counting.

### Existing fields (unchanged)

| Property | Type | Description |
|----------|------|-------------|
| `totalAllocatableCpuMilli` | `integer (int64)` | Total CPU from counted nodes (milli-CPUs). |
| `totalAllocatableMemoryBytes` | `integer (int64)` | Total memory from counted nodes (bytes). |
| `nodeCount` | `integer (int32)` | Number of counted nodes. |
| `lastUpdated` | `string` | RFC 3339 timestamp of the last recomputation. |

## Controller Behaviour

### Filtering (NEW)

The Node Capacity Controller applies the following filter before summing
allocatable:

1. **Default exclusion**: nodes with `spec.unschedulable = true` are always
   excluded. This cannot be disabled.
2. **Selector exclusion**: if `spec.nodeSelector` is present and non-empty,
   nodes matching the selector are excluded.
3. **Inclusion condition**: a node is counted only if it is schedulable AND does
   not match the selector (FR-004).

### Selector validation (NEW)

On each reconciliation, the controller validates the selector structurally
(operator validity, values consistency). If invalid, it logs a warning and
proceeds with unschedulable-only exclusion for that cycle. Validity is
re-checked on the next event — a corrected selector takes effect immediately.

### Status patching (MODIFIED)

The status patch now includes the three new exclusion-count fields alongside
the existing CPU/memory/count fields. All fields are patched atomically in a
single merge-patch (wrapped under the `"status"` key per the existing
`status_merge_patch` envelope).

## RBAC

**No changes.** The controller already has `get/list/watch` on `nodes`
(including `spec.unschedulable` and `metadata.labels`) and on `clustercapacities`
(to read the spec via the reflector cache). No new permissions are required.

## Configuration Examples

### Exclude control-plane nodes

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

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelector:
      matchExpressions:
        - key: node-role.kubernetes.io/control-plane
          operator: Exists
'
```

### Exclude by label value

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelector:
      matchLabels:
        dedicated: system
'
```

### Remove the selector (revert to unschedulable-only)

```sh
kubectl patch clustercapacity cluster-capacity --type=json -p '
  [{"op": "remove", "path": "/spec/nodeSelector"}]
'
```

### Inspect exclusion status

```sh
kubectl get clustercapacity cluster-capacity -o jsonpath='{.status}'
# {"totalAllocatableCpuMilli":32000,"totalAllocatableMemoryBytes":...,"nodeCount":4,
#  "excludedNodeCount":2,"excludedByUnschedulable":1,"excludedBySelector":1,...}
```
