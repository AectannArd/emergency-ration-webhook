# Clarifications — Session 2026-07-25

> Produced by `/speckit-clarify` ahead of `/speckit-specify`. No spec file
> existed yet (clarify precedes specify in this repo's workflow), so answers
> are captured here for the specify phase to encode into `spec.md`.

## Session 2026-07-25

- Q: When calculating consumed capacity, does the webhook count declared pod
  `resources.requests` or live usage via metrics-server?
  → A: **Declared requests** (pod `resources.requests`). Deterministic, consistent with kube-scheduler, no metrics-server dependency.
- Q: Is the capacity percentage ceiling applied cluster-wide, per-node, or per-resource-pool?
  → A: **Cluster-wide** (Option A). One capacity percentage for total cluster allocatable CPU and RAM. Single budget, simplest correct model. "For now" — per-node/pool partitioning is a deferred future concern, not v1.
- Q: Which admission verbs does the webhook need to gate (CREATE / UPDATE / DELETE)?
  → A: Reframed into a full **3-component operator architecture** (see Architecture
     Vision below). Admission verbs fall out of the component split, not decided
     in isolation.

## Architecture Vision (2026-07-25)

Two independent processes drive capacity; the webhook owns one, not both:

1. **Node lifecycle** — drives the top of the budget (available capacity).
   We **watch** it but do **not interrupt** it (not draining a node for
   maintenance carries heavy risk; that's an operator decision, not the
   webhook's).
2. **Pod lifecycle** — drives consumption. The webhook **controls** this.

Between the two processes, the data link is **CRDs** (shared state). Three
components:

### Component 1 — Node Capacity Controller
- Watches nodes.
- Owns a CRD whose **status** holds the cumulative cluster capacity
  (sum of `.status.allocatable` across all nodes).
- Read-only on nodes; never interrupts node lifecycle.

### Component 2 — Allocation Controller
- Watches the Node Capacity CRD (from Component 1) + resources allocated to
  scheduled pods.
- Calculates current allocation percentage (stored in **status**).
- Holds the **target allocation threshold** in its **spec** (the configurable
  capacity ceiling).
- Tracks pod **CREATE + UPDATE + DELETE** to keep allocation accurate.

### Component 3 — Admission Webhook
- Reads Component 2's CRD **spec** (threshold) + **status** (current allocation)
  for the admission decision.
- Validates a new Pod against the remaining budget.
- Tracks pod **CREATE + UPDATE**.

Data flow:

```
  nodes ──watch──▶ [Node Capacity Controller] ──status──▶ ClusterCapacity CRD
                                                                      │
  pods ──watch───▶ [Allocation Controller] ──reads──▶ ClusterCapacity CRD
                         │ writes status (allocation %) + reads spec (threshold)
                         ▼
                  Allocation CRD ◀──reads── [Admission Webhook]
                                       │ CREATE+UPDATE on pods
                                       ▼
                                 AdmissionReview response
```
