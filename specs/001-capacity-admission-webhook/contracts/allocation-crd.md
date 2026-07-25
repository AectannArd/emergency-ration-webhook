# Contract: Allocation CRD

**Phase**: 1 (Design) | **Date**: 2026-07-26

Defines the `Allocation` custom resource — the demand-side state that carries
the user-configurable budget threshold (in `spec`) and the controller-computed
allocation figures (in `status`).

---

## Summary

| Property | Value |
|----------|-------|
| Group | `emergency-ration.dev` |
| Version | `v1` |
| Kind | `Allocation` |
| Scope | Cluster (not namespaced) |
| Plural | `allocations` |
| Singular | `allocation` |
| Short name | `alloc` |
| Singleton | Yes — instance name: `cluster-allocation` |

Users create one instance named `cluster-allocation` to configure the budget.
The Allocation Controller then populates its `status` with the current
allocation figures.

---

## Spec

```yaml
spec:
  budgetPercent: 80
```

| Field | Type | Required | Validation | Description |
|-------|------|----------|------------|-------------|
| `budgetPercent` | int32 | Yes | `minimum: 0`, `maximum: 100` | Maximum allowed allocation as a percentage of total cluster allocatable capacity. Applied to both CPU and RAM independently. `0` = reject all resource requests; `100` = only reject genuine overcommit. |

This is the **only** user-configurable field in the entire system. It is
adjustable at runtime without restart (FR-011) — patching this field causes
the Allocation Controller to recompute the ceiling and the webhook to use the
new value on the next admission.

---

## Status

Written by the Allocation Controller by summing pod resource requests across
all non-terminal pods.

```yaml
status:
  allocatedCPUMilli: 250000         # 250 cores allocated
  allocatedMemoryBytes: 386547056640 # 360 GiB allocated
  ceilingCPUMilli: 256000           # 80% of 320 cores
  ceilingMemoryBytes: 412316860416  # 80% of 480 GiB
  utilizationPercentCPU: 0.9766     # 250/256
  utilizationPercentMemory: 0.9375  # 360/384
  lastUpdated: "2026-07-26T14:32:05Z"
```

| Field | Type | Description |
|-------|------|-------------|
| `allocatedCPUMilli` | int64 | Sum of `pod.spec.containers[].resources.requests.cpu` across all non-terminal pods, in milli-CPUs. |
| `allocatedMemoryBytes` | int64 | Sum of `pod.spec.containers[].resources.requests.memory` across all non-terminal pods, in bytes. |
| `ceilingCPUMilli` | int64 | `floor(totalAllocatableCPUMilli * budgetPercent / 100)`. Recomputed when either supply or budget changes. |
| `ceilingMemoryBytes` | int64 | `floor(totalAllocatableMemoryBytes * budgetPercent / 100)`. |
| `utilizationPercentCPU` | double | `allocatedCPUMilli / ceilingCPUMilli`. May exceed 1.0 if pods were admitted before a node was removed. |
| `utilizationPercentMemory` | double | `allocatedMemoryBytes / ceilingMemoryBytes`. |
| `lastUpdated` | string (RFC 3339) | Timestamp of last allocation recomputation. Used by the webhook for freshness checks. |

---

## Controller Behaviour

The Allocation Controller:
1. Runs a `kube::runtime::reflector` on `Api::<Pod>::all(client)`.
2. Watches the `cluster-capacity` ClusterCapacity CRD (for supply changes).
3. Reads `budgetPercent` from the `cluster-allocation` Allocation CRD spec.
4. On any relevant event (pod change, capacity change, budget change):
   a. Sums resource requests across all pods not in a terminal phase
      (`Failed`, `Succeeded`).
   b. Applies Kubernetes defaulting: containers with limits but no requests
      use `requests = limits`.
   c. Computes the ceiling from current supply and budget.
   d. Patches the `cluster-allocation` CRD `.status` subresource.

**RBAC**: The controller's service account requires:
- `get`, `list`, `watch` on `pods` (core/v1).
- `get`, `list`, `watch` on `clustercapacities` (the controller reads
  ClusterCapacity status to compute the ceiling).
- `get`, `list`, `watch`, `update`, `patch` on `allocations`
  (the `.status` subresource).

---

## Pod Phase Filtering

The Allocation Controller counts requests from pods in phases:

| Phase | Counted? | Rationale |
|-------|----------|-----------|
| `Pending` | Yes | The pod is scheduled (or will be); its requests are reserved. |
| `Running` | Yes | Active workload. |
| `Unknown` | Yes | Conservative — the pod may still be running. |
| `Succeeded` | No | Terminal — the pod has completed; resources are freed. |
| `Failed` | No | Terminal — the pod has failed; resources are freed. |

This matches the kube-scheduler's reservation model (spec: "scheduled
workloads" are counted).

---

## Defaulting Convention

Per FR-005 and the spec's edge cases, resource requests are resolved as:

```
for each container in pod.spec.containers + pod.spec.initContainers:
    if container.resources.requests.cpu exists:
        request.cpu = parse(requests.cpu)
    elif container.resources.limits.cpu exists:
        request.cpu = parse(limits.cpu)   # requests = limits defaulting
    else:
        request.cpu = 0

    (same for memory)
```

Init containers run sequentially, so the effective pod request for scheduling
is `max(sum(regular container requests), max(init container requests))`.
The Allocation Controller uses the same formula for consistency with the
kube-scheduler.

---

## OpenAPI Validation Schema

Generated by the `kube::CustomResource` derive. See data-model.md for the
full CRD YAML.
