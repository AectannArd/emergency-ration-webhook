# Quickstart Validation — Workload Exclusion Policy

## Prerequisites

- Rust toolchain (MSRV 1.89), `cargo`
- The project builds: `cargo build`

## US1 — Namespace List Exclusion

### Unit tests

```sh
cargo test --lib webhook::handler -- exclusion
```

Covers: `check_exemption` with namespace match, priority class match, OR
semantics, empty lists, absent priority class, duplicate entries.

### Integration test

```sh
cargo test --test admission -- namespace_exclusion
```

**Scenario**: Allocation CRD spec carries `excludedNamespaces: ["monitoring"]`.
Submit an over-budget pod in `monitoring` → admitted (exempt). Submit the same
pod in `app-team-a` → denied (over budget).

### BDD scenario

```sh
cargo test --test admission_bdd -- namespace_exclusion
```

```gherkin
Scenario: Pod in excluded namespace is admitted regardless of budget
  Given the cluster budget is at 100% utilization
  And the Allocation CRD excludes namespace "monitoring"
  When a pod is submitted in namespace "monitoring"
  Then the admission response is allowed
  And the exemption reason is "namespace"
```

## US2 — Priority Class Exclusion

### BDD scenario

```sh
cargo test --test admission_bdd -- priority_class_exclusion
```

```gherkin
Scenario: Pod with excluded priority class is admitted regardless of budget
  Given the cluster budget is at 100% utilization
  And the Allocation CRD excludes priority class "system-node-critical"
  When a pod with priorityClassName "system-node-critical" is submitted
  Then the admission response is allowed
  And the exemption reason is "priority_class"
```

## US3 — Combined OR Semantics

### BDD scenario

```sh
cargo test --test admission_bdd -- combined_exclusion
```

```gherkin
Scenario: Pod matching either namespace or priority class is exempt (OR)
  Given the Allocation CRD excludes namespace "kube-system"
  And the Allocation CRD excludes priority class "system-node-critical"
  When a pod with priorityClassName "system-node-critical" is submitted in "app-team-a"
  Then the admission response is allowed
  And the exemption reason is "priority_class"

  When a pod with no priority class is submitted in "kube-system"
  Then the admission response is allowed
  And the exemption reason is "namespace"

  When a pod with no priority class is submitted in "app-team-a"
  Then the admission response is denied (over budget)
```

## Full Quality Gate

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## Manual E2E

```sh
# Patch the Allocation CRD with exclusion config
kubectl patch alloc cluster-allocation --type=merge -p '
  spec:
    excludedNamespaces:
      - kube-system
      - monitoring
    excludedPriorityClasses:
      - system-node-critical
'

# Verify a pod in an excluded namespace is admitted even at full budget
kubectl apply -f - <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: test-exempt
  namespace: monitoring
spec:
  containers:
    - name: test
      image: busybox
      resources:
        requests:
          cpu: "999"
          memory: "999Gi"
EOF
# Should be admitted (exempt) — no budget check

# Verify a pod in a normal namespace IS budget-checked
kubectl apply -f - <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: test-gated
  namespace: default
spec:
  containers:
    - name: test
      image: busybox
      resources:
        requests:
          cpu: "999"
          memory: "999Gi"
EOF
# Should be denied (over budget)

# Check the exemption counter
kubectl port-forward -n capacity-admission svc/capacity-admission-webhook 9090:metrics &
curl -s localhost:9090/metrics | grep capacity_admission_exemptions_total
# capacity_admission_exemptions_total{reason="namespace"} 1
```
