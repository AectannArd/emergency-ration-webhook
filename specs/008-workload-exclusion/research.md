# Research — Workload Exclusion Policy

## R1 — Where to put the exclusion fields: Allocation CRD vs ClusterCapacity CRD

**Decision**: `Allocation.spec.excludedNamespaces` and
`Allocation.spec.excludedPriorityClasses`.

**Rationale**: The Allocation CRD is the admission-policy singleton. It already
holds `budgetPercent` (the threshold) and `enforcementMode` (how to react to a
violation). Exclusion policy — "which workloads skip the check entirely" — is
the third admission-policy axis and belongs in the same place. The webhook
already reads the Allocation spec from its reflector cache on every request, so
the new fields are available at zero additional cost.

The ClusterCapacity CRD governs supply-side node counting (nodeSelectors,
unschedulable exclusion). Mixing admission-policy fields there would violate
Principle V (Separated Concerns).

**Alternatives**:
- A new `AdmissionPolicy` CRD — rejected: adds a new CRD + controller + reflector
  for two optional fields that are naturally at home on Allocation (Principle V:
  minimal surface).
- A ConfigMap — rejected: ConfigMaps are not typed, not schema-validated, and
  require a separate watch. CRD spec is the established pattern in this codebase.

## R2 — OR semantics: namespace ∪ priorityClass

**Decision**: a pod is exempt if it matches EITHER list. Matching both does not
change the outcome (exemption is boolean).

**Rationale**: operators think in terms of "exclude these workloads" regardless
of whether the criterion is namespace-based or priority-based. AND semantics
(requiring both a matching namespace AND a matching priority class) would be
surprising and nearly useless — no real-world exclusion policy is structured
that way.

**Alternatives**:
- AND semantics — rejected (surprising, not what operators expect).
- Separate admission paths (namespace checked first, priority class only if
  namespace didn't match) — rejected: the order shouldn't matter, and
  observability benefits from knowing which criterion triggered the exemption.

## R3 — Priority class is a string match, not a resource reference

**Decision**: the webhook matches `pod.spec.priorityClassName` as a string
against the `excludedPriorityClasses` list. It does NOT resolve or validate
against actual `PriorityClass` resources.

**Rationale**: resolving PriorityClass resources would require a new watch (or
an API call on the hot path), violating the "reflector-only hot path" invariant.
The string match is sufficient: the operator knows which priority class names
exist in their cluster, and the match is exact (no glob/regex needed for v1).

**Alternatives**:
- Watch PriorityClass resources and validate list entries — rejected: extra
  reflector, extra RBAC, extra complexity for no functional gain.
- Glob/regex matching — rejected: YAGNI. Exact match is sufficient for v1.

## R4 — Webhook's own namespace: dual-layer (apiserver + webhook)

**Decision**: the webhook's own namespace (`--namespace`/`NAMESPACE` config) is
excluded at TWO layers:
1. **Apiserver layer**: the `namespaceSelector` in
   `ValidatingWebhookConfiguration` keeps the webhook's own namespace (FR-009).
   The apiserver never sends these requests — defence-in-depth during cold
   start before the CRD cache is populated.
2. **Webhook layer**: the webhook also checks its own namespace against the
   exclusion config in `evaluate()` (FR-007). This is the CRD-based path that
   applies at runtime.

**Rationale**: the cold-start window (webhook process up, reflector cache not
yet populated) is the only time the CRD-based exclusion cannot work. The
apiserver-level `namespaceSelector` covers that gap. Once the cache is warm, the
webhook also checks its own namespace via the CRD path — so even if an operator
removes the webhook's namespace from the CRD exclusion list, the apiserver
filter still prevents self-gating.

The system-namespace list (`kube-system`, `kube-public`) that was hardcoded in
the static `namespaceSelector` MOVES to the CRD — it becomes the operator's
choice. The `namespaceSelector` is simplified to only the webhook's own
namespace.

**Alternatives**:
- Webhook-only (no apiserver filter) — rejected: cold-start deadlock risk.
- Apiserver-only (no webhook check) — rejected: can't express priority class
  exclusion at the apiserver layer, and moving namespace exclusion to the CRD
  is the whole point of this feature.

## R5 — Excluded pods are still counted in allocation accounting

**Decision**: exclusion affects the admission gate only. The Allocation
Controller still sums ALL non-terminal pods, including excluded ones.

**Rationale**: excluded pods consume real cluster resources. If they were
excluded from accounting, the `status.allocated*` figures would underreport
actual usage, and non-excluded workloads would see inflated headroom —
defeating the budget's purpose. By keeping them counted, operators see the true
utilization; the exclusion means "don't GATE this pod," not "pretend it doesn't
exist."

This also keeps the Allocation Controller untouched (Principle V: separated
concerns) — it doesn't need to know about exclusion policy at all.

**Alternatives**:
- Exclude from accounting too — rejected: hides real resource consumption,
  makes the budget unreliable for non-excluded workloads.

## R6 — New `Exempt` verdict + Prometheus counter

**Decision**: add `DecisionVerdict::Exempt` and a new Prometheus counter
`capacity_admission_exemptions_total{reason}` where `reason ∈ {namespace,
priority_class, webhook_namespace}`.

**Rationale**: Principle IV requires observability for every decision outcome.
An exempted admission is a distinct outcome — it's an `allowed: true` response,
but for a fundamentally different reason than a budget-passing allow. A
dedicated verdict + counter lets dashboards distinguish "admitted within budget"
from "admitted by exclusion policy."

The `reason` label identifies which criterion triggered the exemption:
- `namespace` — matched `excludedNamespaces`
- `priority_class` — matched `excludedPriorityClasses`
- `webhook_namespace` — the webhook's own namespace (FR-007 bootstrap fallback)

When both namespace and priority class match, `namespace` takes precedence
(first-checked). This is deterministic and avoids a `both` label that adds no
operational value.

**Alternatives**:
- Reuse `Allow` verdict — rejected: can't distinguish budget-pass from
  exclusion-pass in logs/metrics.
- A single counter without a reason label — rejected: operators need to know
  WHICH exclusion criterion fired (Principle IV: "for any admission request…
  what was decided, and why").
