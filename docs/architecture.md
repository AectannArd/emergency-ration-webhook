# Architecture

[← Back to README](../README.md)

`emergency-ration-webhook` is a 3-component operator in a single process, linked
by two cluster-scoped CRDs as shared state (Constitution Principle V). Full design
detail is in
[`specs/001-capacity-admission-webhook/data-model.md`](../specs/001-capacity-admission-webhook/data-model.md);
this is the operator-facing summary.

```text
  Node API  ──▶  Node Capacity Controller   ──▶  ClusterCapacity (status)
                  (sum of .status.allocatable)     cluster-capacity  [supply]
                                                          │
                                                          ▼
  Pod API   ──▶  Allocation Controller      ◀──  Allocation (spec.budgetPercent)
                  (sum of pod requests              cluster-allocation
                   + ceiling(supply, budget))  ──▶  Allocation (status)
                                                          │  [allocated, ceiling]
                                                          ▼
  API server ──▶ Admission Webhook          ──▶  AdmissionResponse (allow/deny)
                  POST /validate (HTTPS)
                  reads Allocation status only (in-process cache, no network)
```

- **Node Capacity Controller** — watches nodes and writes their summed
  `.status.allocatable` into `ClusterCapacity` `status` (the supply side).
  Read-only on nodes; it never interrupts node lifecycle.
- **Allocation Controller** — sums pod resource requests (non-terminal pods) and
  the budget from `Allocation.spec.budgetPercent`, computes the per-resource
  ceilings, and writes the result into `Allocation` `status` (the demand side).
- **Admission Webhook** — reads `Allocation` `status` from an in-process cache
  (no network on the hot path) and admits or denies pod `CREATE`/`UPDATE` against
  the budget.
