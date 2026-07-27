# Contract: Admission Exclusion Policy (spec-008)

## Allocation CRD Spec — New Fields

### `excludedNamespaces`

```yaml
spec:
  excludedNamespaces:
    - kube-system
    - monitoring
    - ci-cd
```

- **Type**: `array<string>`, nullable, optional (not in `required`).
- **Semantics**: a pod whose namespace (from the AdmissionRequest `.namespace`)
  matches ANY entry is exempt from capacity admission — `allowed: true` without
  a budget check.
- **Default**: absent → no namespace exclusions (backward-compatible).

### `excludedPriorityClasses`

```yaml
spec:
  excludedPriorityClasses:
    - system-node-critical
    - system-cluster-critical
```

- **Type**: `array<string>`, nullable, optional (not in `required`).
- **Semantics**: a pod whose `spec.priorityClassName` matches ANY entry is
  exempt. String match only — the webhook does NOT resolve PriorityClass
  resources.
- **Default**: absent → no priority class exclusions (backward-compatible).
- **Absent priority class**: a pod with no `priorityClassName` (or empty
  string) never matches.

### OR Semantics

A pod is exempt if it matches EITHER list. Matching both is equivalent to
matching one (exemption is boolean).

### Check Order

1. Webhook's own namespace (`--namespace`/`NAMESPACE` config) — FR-007 bootstrap
   self-exemption, checked first so the webhook never self-gates.
2. `excludedNamespaces` — operator-configured namespace list.
3. `excludedPriorityClasses` — operator-configured priority class list.

First match wins; subsequent checks are skipped. The winning reason is recorded
in logs and metrics.

## Admission Decision — New Outcome

### Exempt Verdict

When a pod is exempt by exclusion policy:

- **Response**: `AdmissionResponse { allowed: true }` — no warnings, no reason
  message on the response itself (the apiserver sees a clean allow).
- **Structured log**: INFO level, `decision = "exempt"`,
  `exemption_reason = "namespace" | "priority_class" | "webhook_namespace"`,
  plus the standard fields (workload, operation, latency_ms).
- **Prometheus metric**: `capacity_admission_exemptions_total{reason=...}` +1.

The exempt pod does NOT trigger `capacity_admission_verdicts_total` — that
counter tracks budget decisions (allow/deny/dry_run_deny/error) only.

## Metrics — New Counter

```
# HELP capacity_admission_exemptions_total Pods admitted by exclusion policy, by reason.
# TYPE capacity_admission_exemptions_total counter
capacity_admission_exemptions_total{reason="namespace"} 0
capacity_admission_exemptions_total{reason="priority_class"} 0
capacity_admission_exemptions_total{reason="webhook_namespace"} 0
```

All three `reason` label values are pre-created at zero at startup (same pattern
as the existing verdict counter).

## Webhook Config — Simplified namespaceSelector

The `ValidatingWebhookConfiguration` `namespaceSelector` is simplified. The
hardcoded system-namespace list (`kube-system`, `kube-public`) is removed —
those namespaces are now excluded dynamically via the CRD. Only the webhook's
own namespace remains at the apiserver level as defence-in-depth:

```yaml
namespaceSelector:
  matchExpressions:
    - key: kubernetes.io/metadata.name
      operator: NotIn
      values: ["capacity-admission"]
```

## Allocation Controller — Unchanged

The Allocation Controller does NOT change. It sums ALL non-terminal pods
regardless of exclusion status. Excluded pods appear in `status.allocatedCpuMilli`
and `status.allocatedMemoryBytes` — their resource consumption is visible. This
is by design: exclusion is an admission-gate bypass, not an accounting exclusion.
