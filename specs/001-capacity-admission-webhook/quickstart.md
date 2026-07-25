# Quickstart: Capacity Admission Webhook

**Phase**: 1 (Design) | **Date**: 2026-07-26

This is a validation guide — it documents the runnable scenarios that prove
the feature works end-to-end. It is not an implementation guide; code details
belong in `tasks.md` (Phase 2).

---

## Prerequisites

- Rust toolchain ≥ 1.89 (`rustup default stable`).
- A Kubernetes cluster (1.34+) for E2E tests, or just `cargo test` for
  integration tests (no cluster needed).
- For E2E: `kubectl`, `k3d` or `kind`, and `docker`.

---

## Build

```sh
cargo build
```

Expected: clean compilation, no warnings (`cargo clippy -- -D warnings` must
also pass).

---

## Scenario 1: Budget Enforcement (User Story 1)

**What it proves**: a pod that fits is admitted; a pod that exceeds the budget
is rejected with figures.

### Integration test (no cluster needed)

```sh
cargo test --test integration budget_enforcement
```

**Expected**: tests pass, covering:
1. Pod with requests under ceiling → admitted.
2. Pod with requests over ceiling → denied with message citing the resource,
   current, requested, projected, and ceiling.
3. Pod requesting exactly the remaining budget → admitted (inclusive ceiling).
4. Pod with zero requests → admitted.
5. Pod update (increase request) → evaluated as delta.

### BDD test

```sh
cargo test --test bdd budget_enforcement
```

Runs `tests/bdd/features/budget_enforcement.feature`.

### E2E test (requires k3d/kind cluster)

```sh
# Start a test cluster
k3d cluster create capacity-test

# Install CRDs, RBAC, deployment
kubectl apply -f deploy/crds.yaml
kubectl apply -f deploy/rbac.yaml
kubectl apply -f deploy/cert-setup.yaml
kubectl apply -f deploy/deployment.yaml
kubectl apply -f deploy/webhook-config.yaml

# Wait for webhook to be ready
kubectl wait --for=condition=Ready pod -l app=capacity-admission-webhook -n capacity-admission --timeout=60s

# Set budget to 80%
cat <<EOF | kubectl apply -f -
apiVersion: emergency-ration.dev/v1
kind: Allocation
metadata:
  name: cluster-allocation
spec:
  budgetPercent: 80
EOF

# Try to create a pod that exceeds budget
kubectl run test-pod --image=nginx --requests='cpu=999,memory=999Gi'

# Expected: error from server, admission denied, message includes budget figures
# Example: "CPU budget exceeded: allocated Xm, requested Ym, projected Zm, ceiling Wm"

# Tear down
k3d cluster delete capacity-test
```

---

## Scenario 2: Capacity Awareness (User Story 2)

**What it proves**: every decision is observable with capacity figures, and
metrics are exposed.

### Integration test

```sh
cargo test --test integration capacity_awareness
```

**Expected**: tests verify that:
1. Structured log entries contain all required fields (workload, decision,
   resource, allocated, requested, projected, ceiling).
2. Rejection messages contain actionable figures.
3. Denials include a machine-readable reason.

### Metrics verification (E2E)

```sh
# Port-forward the metrics endpoint
kubectl port-forward -n capacity-admission svc/capacity-admission-webhook 9090:metrics &

# Scrape metrics
curl -s http://localhost:9090/metrics | grep capacity_admission

# Expected output includes:
# capacity_admission_verdicts_total{resource="cpu",verdict="allow"} <count>
# capacity_admission_verdicts_total{resource="cpu",verdict="deny"} <count>
# capacity_admission_decision_duration_seconds_bucket{le="0.05"} <count>
# capacity_admission_allocation_ratio{resource="cpu"} 0.75
# capacity_admission_total_allocatable{resource="cpu"} 320000
# capacity_admission_current_allocation{resource="cpu"} 240000
# capacity_admission_ceiling{resource="cpu"} 256000
```

### Capacity state updates (E2E)

```sh
# Check current capacity
kubectl get clustercapacity cluster-capacity -o yaml
# Expected: status.totalAllocatableCPUMilli, totalAllocatableMemoryBytes populated

# Check current allocation
kubectl get allocation cluster-allocation -o yaml
# Expected: status.allocatedCPUMilli, ceilingCPUMilli, utilizationPercentCPU populated

# Add a node (if using k3d: k3d node create)
# Expected: ClusterCapacity status updates within seconds
```

---

## Scenario 3: Fail-Safe Operation (User Story 3)

**What it proves**: the webhook rejects under every failure condition.

### Integration test

```sh
cargo test --test integration fail_safe
```

**Expected**: tests verify each failure path produces `allowed: false`:
1. Capacity data stale (lastUpdated beyond threshold) → deny.
2. Allocation CRD missing/not populated → deny.
3. ClusterCapacity CRD missing → deny.
4. Malformed AdmissionReview (deserialisation failure) → deny.
5. Decision timeout exceeded → deny.
6. Unknown error type → deny (catch-all).

### BDD test

```sh
cargo test --test bdd fail_safe
```

Runs `tests/bdd/features/fail_safe.feature`.

### E2E: webhook down

```sh
# Scale webhook to 0
kubectl scale -n capacity-admission deploy/capacity-admission-webhook --replicas=0

# Try to create a pod
kubectl run test-pod --image=nginx

# Expected: admission denied (failurePolicy: Fail)
# Error: "failed calling webhook ... no endpoints available"

# Scale back up
kubectl scale -n capacity-admission deploy/capacity-admission-webhook --replicas=2
```

---

## Scenario 4: Performance (Success Criteria SC-005/SC-006)

**What it proves**: the webhook meets latency and footprint targets.

### Latency (integration benchmark)

```sh
cargo test --test integration -- --nocapture performance
```

**Expected**: p99 decision time < 100ms, p50 < 50ms (the hot path is an
in-memory read + arithmetic, so this should be well under 1ms in practice).

### Resource footprint (E2E)

```sh
kubectl top pod -n capacity-admission -l app=capacity-admission-webhook
# Expected: memory < 256 MiB, CPU < 500m under load
```

---

## Full Test Suite

```sh
# Unit + integration (default, no cluster needed)
cargo test

# Clippy (quality gate)
cargo clippy -- -D warnings

# Format check (quality gate)
cargo fmt --check

# E2E (requires cluster, marked #[ignore])
cargo test -- --ignored
```

---

## CI Matrix

E2E tests run against the N-2 Kubernetes matrix:

| Version | Tool |
|---------|------|
| 1.34 | `kind` or `k3d` with k8s 1.34 node image |
| 1.35 | `kind` or `k3d` with k8s 1.35 node image |
| 1.36 | `kind` or `k3d` with k8s 1.36 node image |

CI workflow: for each version, create cluster → apply manifests → run
`kubectl wait` → run E2E tests → tear down. All three must pass.
