# Node Exclusion

[← Back to README](../README.md)

The capacity aggregate does **not** count every node — it excludes nodes the
kube-scheduler cannot place workloads on, so the budget reflects capacity the
cluster can actually use. There are two exclusion layers (spec-006 / spec-007):

1. **Unschedulable nodes (default, always on).** Any node with
   `spec.unschedulable = true` (cordoned nodes, and typically control-plane
   masters) is excluded. This cannot be disabled. It fixes a phantom-capacity
   bug where cordoned nodes inflated the supply pool.
2. **Label-selector exclusion (optional).** Set `spec.nodeSelectors` to a list of
   label selectors. A node matching **any** selector in the list is excluded (OR
   semantics); within each selector, `matchLabels` and `matchExpressions` are
   ANDed (standard Kubernetes `LabelSelector` semantics). An empty selector is a
   no-op (it excludes nothing).

A node is counted only if it passes **both** layers. When all nodes are excluded,
capacity drops to zero and the webhook fails closed on every admission (correct —
no verifiable capacity).

The status reports the breakdown: `excludedNodeCount`, `excludedByUnschedulable`,
`excludedBySelector`. A node is counted once per layer it fails — a node both
unschedulable and selector-matched counts under `excludedByUnschedulable` only,
and a node matching multiple selectors counts under `excludedBySelector` once
(never double-counted).

## Exclude control-plane nodes by label

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchExpressions:
          - key: node-role.kubernetes.io/control-plane
            operator: Exists
'
```

## Exclude control-plane AND experimental nodes (OR — spec-007)

A list of selectors excludes the union of their matches. Here control-plane
nodes and experimental nodes are both excluded:

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchExpressions:
          - key: node-role.kubernetes.io/control-plane
            operator: Exists
      - matchExpressions:
          - key: node-type/experimental
            operator: Exists
'
```

## Exclude nodes by label value

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchLabels:
          dedicated: system
'
```

The selectors are read from the spec on every reconciliation, so a patch takes
effect on the next node event **without a restart**. Each selector is validated
independently: a structurally invalid entry (unknown operator, `In` without
values) is logged with a warning and skipped, while the remaining valid
selectors still apply — capacity tracking continues, and a corrected selector
takes effect immediately. If every selector is invalid, the controller falls
back to unschedulable-only exclusion for that cycle.

## Migrating from the singular `nodeSelector` (spec-006 → spec-007)

The `nodeSelector` field (a single `LabelSelector`) was renamed to
`nodeSelectors` (a list). Wrap your existing selector in a list — the field is
optional and defaults to no selectors, so no data-migration webhook is needed:

```sh
# Before (spec-006): spec.nodeSelector: { matchExpressions: [...] }
# After  (spec-007): spec.nodeSelectors: [ { matchExpressions: [...] } ]
kubectl patch clustercapacity cluster-capacity --type=json -p '
  [{"op": "move", "from": "/spec/nodeSelector", "path": "/spec/nodeSelectors/0"}]
'
```

## Remove all selectors (revert to unschedulable-only)

```sh
kubectl patch clustercapacity cluster-capacity --type=json -p '
  [{"op": "remove", "path": "/spec/nodeSelectors"}]
'
```

## Inspect the exclusion breakdown

```sh
kubectl get clustercapacity cluster-capacity -o jsonpath='{.status}'
# {"totalAllocatableCpuMilli":16000,...,"nodeCount":2,
#  "excludedNodeCount":2,"excludedByUnschedulable":0,"excludedBySelector":2,...}
```
