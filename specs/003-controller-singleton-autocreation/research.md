# Research: Controller Singleton Autocreation

**Feature**: 003-controller-singleton-autocreation | **Date**: 2026-07-26

## R1: The Bug (Root Cause Analysis)

**Source**: E2E CI debug output from run 30214916166.

The Node Capacity Controller's `patch_status` function calls
`api.patch_status(CLUSTER_CAPACITY_NAME, &params, &patch)` without ensuring the
instance exists. When `cluster-capacity` is absent, the apiserver returns:

```
404 NotFound: clustercapacities.emergency-ration.dev "cluster-capacity" not found
```

The controller logs this as a `warn!` and moves on — the status is never
written. The Allocation Controller's `recompute` function does a `get` on
`cluster-allocation` and silently returns if not found. Neither controller
creates its singleton.

**Impact**: Without ClusterCapacity status, the Allocation Controller has no
supply figures → cannot compute ceilings → writes no Allocation status → the
webhook has no capacity data → rejects ALL pods (Principle I: fail-closed).

## R2: kube-rs Singleton Lifecycle Pattern

**Decision**: Use a get-or-create pattern at controller startup.

```rust
async fn ensure_singleton(api: &Api<T>) {
    match api.get(SINGLETON_NAME).await {
        Ok(_) => debug!("singleton already exists"),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            let instance = T::new(SINGLETON_NAME, DEFAULT_SPEC);
            match api.create(&PostParams::default(), &instance).await {
                Ok(_) => info!("created singleton"),
                Err(kube::Error::Api(e)) if e.code == 409 => {
                    debug!("singleton already exists (race with another replica)");
                }
                Err(e) => warn!(%e, "failed to create singleton"),
            }
        }
        Err(e) => warn!(%e, "failed to check singleton existence"),
    }
}
```

**Rationale**: This is the standard Kubernetes operator pattern. The controller
owns its singleton lifecycle. The get-first approach preserves any existing
operator-set fields (e.g. a custom budgetPercent). The 409-handling makes it
safe for multi-replica deployments.

**Alternatives considered**:
- Server-side apply (`Patch::Apply`): would create-or-patch in one call, but
  kube-rs SSA support for CRDs is finicky and would overwrite the spec. Rejected
  because the Allocation Controller must NOT overwrite an operator-set
  budgetPercent.
- Kubernetes finalizers: overkill for a singleton that the controller recreates
  on the next cycle if deleted.

## R3: Default budgetPercent for Allocation Autocreation

**Decision**: 80%.

**Rationale**: 80% leaves 20% headroom for system daemons, node-level overhead,
and unexpected spikes. This matches the test value used in the existing CI
workflow and the examples in spec-001. It is the same default the constitution's
Technology Constraints section implies (the budget is a "configurable percentage"
and 80% is the canonical example throughout the specs).

## R4: Where to Call ensure_singleton

**Decision**: At the start of each controller's `run` function, before the
reconcile loop. Additionally, if `patch_status`/`recompute` encounters a 404
during operation (singleton deleted mid-run), it re-creates on the next cycle.

**Node Capacity Controller**: call `ensure_singleton(&capacity_api)` before the
`stream.for_each(...)` loop starts. The existing `patch_status` already handles
404 by logging a warning — after the fix, the next node event will trigger
another `patch_status`, and if it 404s again, the controller should call
`ensure_singleton` and retry.

**Allocation Controller**: call `ensure_singleton(&allocation_api)` before the
`ticker.tick()` loop starts. The existing `recompute` already does a `get` and
returns if not found — after the fix, `ensure_singleton` runs first, so the
instance exists by the time `recompute` reads it.
