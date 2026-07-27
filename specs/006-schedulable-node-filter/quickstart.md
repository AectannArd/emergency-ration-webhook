# Quickstart Validation — Schedulable Node Filter

> Validation guide for the 3 user stories in [spec.md](./spec.md). Each scenario
> maps to a runnable test. Implementation details belong in `tasks.md` and the
> implementation phase — this document defines only the commands and assertions
> that prove the feature works.

## Prerequisites

- Rust toolchain (MSRV 1.89), `cargo`
- The project builds: `cargo build`
- For E2E: a `kind`/`k3d` cluster with the webhook deployed (per
  [README Quick Start](../../README.md#quick-start))

## US1 — Cordoned Nodes Excluded by Default (P1)

### Unit test

```sh
cargo test --lib node_capacity::tests::
```

Covers:
- `sum_node_allocatable` with a mix of schedulable + unschedulable nodes →
  unschedulable nodes contribute zero, counted_node_count excludes them.
- All-unschedulable cluster → `(0, 0, 0)` (Principle I interaction).
- `is_node_counted(unschedulable=true, ...)` → `false`.

### Integration test (mock apiserver)

```sh
cargo test --test node_filter -- node_filter::cordon
```

**Scenario**: the mock apiserver serves a node list with one node having
`spec.unschedulable: true`. The controller reconciles. Assert:
- Status PATCH body contains `nodeCount` excluding the cordoned node.
- `excludedByUnschedulable: 1`, `excludedNodeCount: 1`.
- CPU/memory sum excludes the cordoned node's allocatable.

### BDD scenario

```sh
cargo test --test node_filter_bdd -- --tags @cordon
```

Feature: `tests/bdd/features/node_filter.feature` — `@cordon` tag.

```gherkin
Scenario: Cordoned node is excluded from capacity
  Given a cluster with 3 schedulable nodes each with 16 CPU and 32Gi memory
  When one node is cordoned
  Then the ClusterCapacity status reports 2 nodes
  And the excludedByUnschedulable count is 1
```

## US2 — Label-Selector Exclusion (P2)

### Unit test

```sh
cargo test --lib node_filter::tests::
```

Covers:
- `labels_match_selector` with `matchLabels` hit/miss.
- `labels_match_selector` with each `matchExpressions` operator (In, NotIn,
  Exists, DoesNotExist).
- `is_node_counted` with a selector that matches → `false`.
- Empty selector → matches all (node counted if schedulable).
- `validate_selector` — valid and invalid cases.

### Integration test (mock apiserver)

```sh
cargo test --test node_filter -- node_filter::selector
```

**Scenario**: the mock apiserver serves a node list with a control-plane node
carrying `node-role.kubernetes.io/control-plane` label. The ClusterCapacity
CRD spec has `nodeSelector.matchExpressions[key: node-role.kubernetes.io/
control-plane, operator: Exists]`. Assert:
- Status PATCH excludes the control-plane node.
- `excludedBySelector: 1`.

### BDD scenario

```sh
cargo test --test node_filter_bdd -- --tags @selector
```

```gherkin
Scenario: Control-plane nodes excluded by label selector
  Given a cluster with 2 worker nodes and 1 control-plane node
  And the ClusterCapacity nodeSelector excludes nodes labeled node-role.kubernetes.io/control-plane
  Then the ClusterCapacity status reports 2 nodes
  And the excludedBySelector count is 1
```

## US3 — Observability of Excluded Nodes (P3)

### Unit test

```sh
cargo test --lib node_capacity::tests::exclusion_breakdown
```

Covers:
- Mixed cluster (1 unschedulable, 1 selector-matched, 3 counted) →
  `excludedByUnschedulable: 1`, `excludedBySelector: 1`, `nodeCount: 3`,
  `excludedNodeCount: 2`.
- Node that is both unschedulable + selector-matched → counted under
  `excludedByUnschedulable` only (no double-count).

### BDD scenario

```sh
cargo test --test node_filter_bdd -- --tags @observability
```

```gherkin
Scenario: Status shows excluded node breakdown
  Given a cluster with 5 nodes where 1 is cordoned and 1 matches the nodeSelector
  When the controller reconciles
  Then the status shows nodeCount 3
  And excludedNodeCount is 2
  And excludedByUnschedulable is 1
  And excludedBySelector is 1
```

## Full Test Suite

Run the complete quality gate (unit + integration + BDD):

```sh
cargo fmt --check
 cargo clippy -- -D warnings
cargo test --all-targets
```

All must pass. The new tests are part of the existing CI matrix; no special CI
configuration is needed.

## E2E (against a real kind cluster)

This feature's E2E is covered by the existing CI smoke test once the CRD is
updated. No separate `#[ignore]` E2E test file is needed for the filter itself
— the unit + integration + BDD tests prove the logic, and the CRD schema
change is validated by the existing E2E deployment.

Manual E2E (if desired):

```sh
# Deploy the webhook (per README Quick Start)
# Then:
kubectl cordon <some-node>
kubectl get cc cluster-capacity -o jsonpath='{.status.nodeCount}'
# Should drop by 1

kubectl patch cc cluster-capacity --type=merge -p '{"spec":{"nodeSelector":{"matchExpressions":[{"key":"node-role.kubernetes.io/control-plane","operator":"Exists"}]}}}'
kubectl get cc cluster-capacity -o jsonpath='{.status}'
# Should show excludedBySelector > 0
```
