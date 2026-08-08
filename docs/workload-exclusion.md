# Workload Exclusion

[← Back to README](../README.md)

The Allocation singleton also carries two **optional exclusion lists** (spec-008):
`spec.excludedNamespaces` and `spec.excludedPriorityClasses`. A pod matching
**either** list (OR semantics) is admitted **without a budget check** — an
explicit, operator-configured bypass of the capacity gate, for workloads that
must never be gated (system components, the webhook's own namespace, etc.).

- **Check order** (first match wins, subsequent checks skipped): the webhook's
  own namespace (`capacity-admission`, FR-007) → `excludedNamespaces` →
  `excludedPriorityClasses`.
- **Priority class is a string match** on `pod.spec.priorityClassName`. The
  webhook does **not** resolve `PriorityClass` resources or their preemption
  values — it only compares the name. An absent or empty-string
  `priorityClassName` never matches.
- **Excluded pods are still counted.** The Allocation Controller is unchanged:
  excluded pods still contribute to `allocatedCpuMilli` /
  `allocatedMemoryBytes`. Exclusion is an admission-gate bypass, **not** an
  accounting exclusion — the excluded consumption stays visible in the gauges.
- **Backward compatible.** Both fields are optional; an absent or empty list
  exempts nothing, so a pre-spec-008 Allocation behaves exactly as before.
- **Fail-closed paths still reject.** The exemption check runs only **after**
  the Allocation singleton and its status are found. Missing allocation, missing
  status, stale data, timeout, and panic all reject before the exemption check
  — exclusion never weakens a fail-closed path. An exempt decision is an
  **explicit allow**, not a fail-open path (see [Failure Modes](./failure-modes.md)).

Patch the lists at runtime — they take effect on the next decision **without a
restart** (read from the webhook's in-process cache, like `budgetPercent`):

```sh
# Exclude a namespace (e.g. monitoring stack, never budget-gated).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedNamespaces":["monitoring"]}}'

# Exclude a priority class (e.g. critical system pods).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedPriorityClasses":["system-node-critical"]}}'

# Set both at once (OR semantics — either match exempts).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedNamespaces":["kube-system"],"excludedPriorityClasses":["system-node-critical"]}}'

# Remove all exclusions (revert to budget-gating everything except the webhook's own ns).
kubectl patch allocation cluster-allocation --type=json \
  -p '[{"op":"remove","path":"/spec/excludedNamespaces"},{"op":"remove","path":"/spec/excludedPriorityClasses"}]'
```

An exempt decision is logged as `decision=exempt` with `exemption_reason` set to
`namespace`, `priority_class`, or `webhook_namespace`, and counted under
`capacity_admission_exemptions_total{reason="..."}` — **not** the verdicts
counter (see [Structured Logging](./observability.md#structured-logging) and
[Prometheus Metrics](./observability.md#prometheus-metrics)).

> **Cold start / self-admission.** Before the webhook's Allocation cache is
> populated it cannot read the CRD exclusion lists, so the
> `ValidatingWebhookConfiguration` keeps a `namespaceSelector` that skips the
> webhook's **own** namespace as apiserver-level defence-in-depth (FR-009). Once
> the Allocation is cached, the webhook also self-exempts its own namespace at
> runtime (FR-007). All other namespace/priority-class exclusions are CRD-based
> and operator-configured.
