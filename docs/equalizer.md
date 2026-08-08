# Multi-Cluster Capacity Equalizer

[← Back to README](../README.md)

The `capacity-equalizer` is a **separate binary** (own Docker image) deployed in
one cluster to manage a **fleet** of N clusters. It reads an `EqualizerConfig`
CRD specifying per-resource cumulative budget targets and a list of target
clusters (each via a kubeconfig Secret), then dynamically adjusts each cluster's
per-resource budget to bring the fleet's cumulative allocation to the target.

## How it works

Every 10 seconds (configurable), the equalizer:
1. Reads each target cluster's `Allocation.status` (current utilization %) and
   `ClusterCapacity.status` (total allocatable).
2. Computes equalized per-resource budgets using the overflow-distribution
   algorithm (per resource, independently).
3. Patches each target's `Allocation.spec.cpuBudgetPercent` /
   `memoryBudgetPercent` (the spec-012 override fields).

**Algorithm**: clusters over the target are frozen at their current utilization;
the total absolute overflow (in CPU milli / RAM bytes) is divided equally among
the good-state clusters; each good cluster's budget is lowered to compensate.

**Worked example** — 3 clusters × 100 CPU, target 80%, utilization 65%/55%/90%:
- Over-cluster (90%): frozen at 90% (overflow = 10 CPU).
- Good clusters (65%, 55%): each gets `80 − 10/2 = 75%`.
- Fleet average = (90+75+75)/3 = **exactly 80%**.

When the over-cluster drops to 86%, budgets recalculate to 77%/77%/86%.

## Deployment

```bash
# 1. Apply the EqualizerConfig CRD + RBAC + Deployment in the home cluster.
kubectl apply -f deploy/equalizer/

# 2. Create a kubeconfig Secret for each target cluster (including the home cluster).
kubectl create secret generic cluster-a-kubeconfig \
  --from-file=kubeconfig=/path/to/cluster-a.kubeconfig -n default

# 3. Create the EqualizerConfig singleton.
kubectl apply -f deploy/equalizer/equalizer-config.example.yaml
```

See `deploy/equalizer/equalizer-config.example.yaml` for a full example with
target cluster definitions and Secret references.

## EqualizerConfig CRD reference

| Field | Type | Description |
|-------|------|-------------|
| `spec.cpuTargetBudgetPercent` | int (0–100) | Cumulative CPU budget target |
| `spec.memoryTargetBudgetPercent` | int (0–100) | Cumulative memory budget target |
| `spec.targets[].name` | string | Human-readable cluster name |
| `spec.targets[].kubeconfigSecretRef.name` | string | Secret containing kubeconfig |
| `spec.targets[].kubeconfigSecretRef.key` | string | Key in Secret (default: `kubeconfig`) |
| `spec.targets[].kubeconfigSecretRef.namespace` | string | Secret's namespace |

Status reports per-cluster observations (utilization, allocatable, computed
budget, state: `healthy`/`over`/`unreachable`/`config-error`) and the overall
fleet condition (`healthy`/`compensating`/`degraded`).

> The equalizer is **not** on the admission critical path. If it is down, each
> cluster's webhook continues enforcing its last-known budget independently.
