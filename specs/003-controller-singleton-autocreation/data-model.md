# Data Model: Controller Singleton Autocreation

**Feature**: 003-controller-singleton-autocreation | **Date**: 2026-07-26

## 1. ensure_singleton Function (Node Capacity Controller)

**Location**: `src/controllers/node_capacity.rs`

```text
ensure_singleton(api: &Api<ClusterCapacity>) -> ()
  │
  ├─ api.get("cluster-capacity")
  │   ├─ Ok(_) → debug log "already exists"; return
  │   ├─ Err(404) → create:
  │   │   ├─ ClusterCapacity::new("cluster-capacity", ClusterCapacitySpec {})
  │   │   ├─ api.create(&PostParams::default(), &instance)
  │   │   │   ├─ Ok(_) → info log "created"; return
  │   │   │   ├─ Err(409) → debug log "race"; return
  │   │   │   └─ Err(_) → warn log; return (retry next cycle)
  │   └─ Err(_) → warn log; return (retry next cycle)
```

**Call site**: top of `run()`, before the reflector stream loop.

## 2. ensure_singleton Function (Allocation Controller)

**Location**: `src/controllers/allocation.rs`

```text
ensure_singleton(api: &Api<Allocation>) -> ()
  │
  ├─ api.get("cluster-allocation")
  │   ├─ Ok(_) → debug log "already exists, preserving operator budget"; return
  │   ├─ Err(404) → create:
  │   │   ├─ Allocation::new("cluster-allocation", AllocationSpec { budget_percent: 80 })
  │   │   ├─ api.create(&PostParams::default(), &instance)
  │   │   │   ├─ Ok(_) → info log "created with default budget 80%"; return
  │   │   │   ├─ Err(409) → debug log "race"; return
  │   │   │   └─ Err(_) → warn log; return (retry next cycle)
  │   └─ Err(_) → warn log; return (retry next cycle)
```

**Call site**: top of `run()`, before the ticker loop.

## 3. Singleton Lifecycle State Machine

```text
                    ┌─────────┐
     controller ───▶│ CHECK   │
     startup        │ get()   │
                    └────┬────┘
                         │
              ┌──────────┼──────────┐
              │          │          │
           Ok(_)      404        Err(_)
              │          │          │
              ▼          ▼          ▼
         EXISTS     CREATE      LOG+RETRY
              │          │
              │       ┌──┴──┐
              │       │     │
              │     Ok    409/Err
              │       │     │
              │       ▼     ▼
              │    CREATED  EXISTS/RETRY
              │       │
              ▼       ▼
         ┌────────────┐
         │ PATCH      │
         │ status     │
         └─────┬──────┘
               │
          ┌────┴────┐
          │         │
        Ok       404
          │         │
          ▼         ▼
        DONE   ENSURE + RETRY (next cycle)
```

## 4. CI Workflow Changes

Remove from `.github/workflows/ci.yml`, "Configure the budget" step:
- The `kubectl apply` that creates the `cluster-capacity` ClusterCapacity instance.

Keep (or modify):
- The Allocation creation step MAY remain to set a specific test budget, OR be
  removed entirely (the controller auto-creates with 80%). Decision: keep it to
  test with an explicit budget value (ensures the controller does NOT overwrite
  an operator-set value).

## 5. README Changes

Remove from README quick start:
- Any `kubectl apply` step that creates the `cluster-capacity` or
  `cluster-allocation` singleton instances.

Add:
- A note that the controllers auto-create both singletons (ClusterCapacity with
  empty spec, Allocation with budgetPercent=80 default).
- Instructions for changing the budget at runtime (kubectl patch).

## 6. Validation Rules

- **VR-001**: `ensure_singleton` is called at the start of each controller's `run` function.
- **VR-002**: `ensure_singleton` calls `get` first and only `create`s on 404 NotFound.
- **VR-003**: `ensure_singleton` treats 409 AlreadyExists as success (idempotent).
- **VR-004**: `ensure_singleton` never overwrites an existing instance's spec.
- **VR-005**: The Allocation default budgetPercent is 80.
- **VR-006**: The CI workflow does NOT manually create the ClusterCapacity instance.
- **VR-007**: The README does NOT instruct operators to create CRD instances manually.
- **VR-008**: Unit tests cover: create when absent, skip when present, 409 idempotent.
