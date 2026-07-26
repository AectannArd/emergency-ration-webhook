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

### Deployment Topology
- **Single binary, three roles.** All three components (Node Capacity Controller,
  Allocation Controller, Admission Webhook) run as async tasks within one
  process, deployed as one `Deployment`. CRDs are the internal data contract.
  Horizontal scaling via stateless replicas. Splitting into separate binaries
  is a future concern if a component needs independent scaling.

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

## Session 2026-07-27 (spec-004: dry-run mode)

> Produced by `/speckit-clarify` ahead of `/speckit-specify` for the dry-run
> (audit/shadow) enforcement mode feature.

- Q: How should dry-run mode be toggled?
  → A: **Allocation CRD spec field** (`spec.enforcementMode: enforce | dry-run`).
     Runtime-adjustable via `kubectl patch` — no restart required to switch
     modes. Consistent with how `spec.budgetPercent` already works: the webhook
     reads the Allocation spec from its in-process cache, so a spec change takes
     effect on the next admission decision. No CLI flag or env var for this.

- Q: What should the AdmissionResponse look like when dry-run mode admits a pod
  that WOULD have been rejected?
  → A: **`allowed: true` with the would-be rejection reason surfaced via the
     admission `warnings` field** (available since Kubernetes 1.19). The pod is
     cleanly admitted (no modification to `allowed` or `message`), but the
     operator sees the would-be rejection surfaced as a Warning — visible in
     `kubectl` output (`Warning: ...`) and in cluster events. Structured logs
     and metrics also reflect the dry-run decision so dashboards/alerts can
     track what *would* be blocked. This avoids polluting the rejection
     `message` (which is the contract for real rejections) while still
     surfacing the information at the point of action.

### Design consequences (carried into specify)

- The webhook evaluates every admission request normally (budget check,
  capacity freshness, fail-closed paths) — it just flips the final verdict from
  deny to allow when `enforcementMode == dry-run` and the only reason for
  denial is an over-budget condition.
- **Fail-closed paths stay fail-closed even in dry-run mode.** If capacity data
  is missing/stale, the webhook cannot evaluate the request at all — it rejects
  regardless of `enforcementMode`. Dry-run only converts *over-budget* denials
  to admits; it does not convert *error* rejections to admits. This preserves
  Constitution Principle I: the webhook never admits under degraded knowledge,
  even in audit mode.
- The `enforcementMode` field defaults to `enforce`. The auto-created singleton
  (`cluster-allocation`) includes this default.
- Metrics and structured logging must distinguish a dry-run would-deny from a
  real deny and a real allow, so operators can build dashboards that answer
  "what would dry-run block?" without conflating it with enforced denials.
