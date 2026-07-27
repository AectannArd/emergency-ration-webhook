# Quickstart Validation — Multi-Selector Node Exclusion

## Prerequisites

- Rust toolchain (MSRV 1.89), `cargo`
- The project builds: `cargo build`

## US1 — Multi-Selector OR Exclusion

### Unit tests

```sh
cargo test --lib controllers::node_filter
```

Covers: `labels_match_any_selector` (match any, match none, empty list),
`is_node_counted` with `Option<&[LabelSelector]>`, `sum_node_allocatable` with
multiple selectors, `effective_selectors` skip-invalid logic.

### Integration test

```sh
cargo test --test node_filter -- multi_selector
```

**Scenario**: mock apiserver serves nodes with different labels; ClusterCapacity
CRD spec carries `nodeSelectors` with 2 selectors. Assert both label-groups are
excluded.

### BDD scenario

```sh
cargo test --test node_filter_bdd
```

```gherkin
Scenario: Nodes matching any of multiple selectors are excluded
  Given a cluster with 2 worker nodes, 1 control-plane node, and 1 experimental node
  And the ClusterCapacity nodeSelectors excludes control-plane and experimental nodes
  When the controller reconciles
  Then the status reports 2 counted nodes
  And excludedBySelector is 2
```

## Full Quality Gate

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## Manual E2E

```sh
kubectl patch cc cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchExpressions:
          - key: node-role.kubernetes.io/control-plane
            operator: Exists
      - matchExpressions:
          - key: node-type/experimental
            operator: Exists
'
kubectl get cc cluster-capacity -o jsonpath='{.status}'
# excludedBySelector should reflect both groups
```
