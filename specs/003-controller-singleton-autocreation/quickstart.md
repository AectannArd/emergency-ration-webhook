# Quickstart: Controller Singleton Autocreation

**Feature**: 003-controller-singleton-autocreation | **Date**: 2026-07-26

Validation guide for the controller singleton autocreation fix.

---

## Validation Scenario 1: Fresh-Cluster Autocreation (FR-001, FR-002, SC-001)

**Goal**: Both singletons are auto-created by controllers in a fresh cluster.

### Steps

1. Deploy the webhook into a kind cluster (apply RBAC, CRDs, TLS Secret,
   Deployment, webhook-config).
2. Do NOT create any CRD instances manually.
3. Wait for webhook pods to be Ready.
4. Check for CRD instances:
   - `kubectl get clustercapacities.emergency-ration.dev cluster-capacity -o yaml`
   - `kubectl get allocations.emergency-ration.dev cluster-allocation -o yaml`

### Expected

- `cluster-capacity` exists with empty spec and populated status (total
  allocatable, node count, last updated).
- `cluster-allocation` exists with `spec.budgetPercent: 80` and populated status
  (allocated, ceiling, utilization, last updated).

### Pass criteria

- [ ] Both singletons exist without manual creation.
- [ ] ClusterCapacity status has non-zero figures (node count > 0).
- [ ] Allocation status has non-zero ceilings (computed from ClusterCapacity).

---

## Validation Scenario 2: No Overwrite of Existing Instance (FR-003, SC-003)

**Goal**: Controllers do not overwrite an operator-set instance.

### Steps

1. Before deploying the webhook, create the Allocation with a custom budget:
   `kubectl apply -f` an Allocation with `spec.budgetPercent: 50`.
2. Deploy the webhook.
3. Wait for controllers to reconcile.
4. Check the Allocation spec: `kubectl get allocation cluster-allocation -o jsonpath='{.spec.budgetPercent}'`.

### Expected

- The budgetPercent is still 50 (the operator's value), NOT overwritten to 80.

### Pass criteria

- [ ] budgetPercent is 50 after controller startup.
- [ ] The Allocation status is computed using budget 50 (ceiling reflects 50%).

---

## Validation Scenario 3: E2E CI Passes (FR-006, SC-002)

**Goal**: E2E CI passes without manual ClusterCapacity creation.

### Steps

1. The CI workflow must NOT have a `kubectl apply` for the ClusterCapacity instance.
2. Push a commit and observe E2E CI.
3. Verify the smoke test passes (small pod admitted, over-budget pod denied).

### Pass criteria

- [ ] No manual ClusterCapacity creation in the workflow.
- [ ] All three K8s versions (1.34, 1.35, 1.36) pass E2E.

---

## Validation Scenario 4: README Accuracy (FR-007, SC-004)

**Goal**: README does not instruct operators to create singletons manually.

### Steps

1. Read the README quick start.
2. Verify no `kubectl apply` step creates CRD instances.

### Pass criteria

- [ ] README documents that controllers auto-create singletons.
- [ ] No manual singleton-creation commands in the quick start.
