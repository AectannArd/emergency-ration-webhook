# Failure Modes

[← Back to README](../README.md)

The webhook is **fail-closed**: every degradation path rejects (`allowed: false`)
rather than admitting under uncertainty (Constitution Principle I). The
`ValidatingWebhookConfiguration` uses `failurePolicy: Fail`, so the API server
itself rejects if the webhook is unreachable. There is no best-effort or
silent-admit path. The fail-closed contract holds across every release in the
[Kubernetes compatibility](./kubernetes-compatibility.md) window.

**Every fail-closed path below rejects in both enforcement modes** (FR-006 /
spec-004): dry-run converts **only** over-budget denials, never error rejections.
The one mode-sensitive row is the budget over-commit itself — see the last row.

| Condition | Outcome | Logged reason (`reason` slug) · HTTP |
|-----------|---------|--------------------------------------|
| Capacity data stale (age > freshness timeout) | Reject (both modes) | `capacity_data_stale` · 500 |
| `ClusterCapacity` or `Allocation` CRD not populated | Reject (both modes) | `capacity_data_missing` · 500 |
| Capacity state not initialised (controller cold start / empty caches) | Reject (both modes) | `capacity_data_missing` · 500 |
| Malformed / undeserialisable admission request | Reject (both modes) | `deserialisation_failure` · 400 |
| Unparseable resource quantity in the pod spec | Reject (both modes) | `quantity_parse_failure` · 400 |
| Decision timeout exceeded | Reject (both modes) | `timeout` · 500 |
| Internal panic / unknown error (catch-all) | Reject (both modes) | `internal_error` / `unknown` · 500 |
| Pod projected allocation over the budget — `enforce` | Reject | `over_budget` · 403 |
| Pod projected allocation over the budget — `dry-run` | **Admit** with a warning (`dry_run_deny` · `verdict="dry_run_deny"`) | `over_budget` (WARN) |

> **Exempt decisions are an explicit allow, not a fail-closed path** (spec-008). A
> pod admitted by the [Workload Exclusion](./workload-exclusion.md) policy (`decision=exempt`)
> is not a degradation outcome — it is an operator-configured bypass that runs
> only *after* the Allocation singleton and its status are found. None of the
> fail-closed rows above is weakened by exclusion config; the exemption check
> never fires on a missing allocation, missing status, stale data, timeout, or
> panic.

Source: [`src/webhook/error.rs`](../src/webhook/error.rs) (variant → message/code
mapping) and [`src/webhook/handler.rs`](../src/webhook/handler.rs) (the
panic/timeout/unknown guards). The catch-all guarantees there is no third,
undefined category: every error maps to a rejection (Constitution Principle III).

## Webhook Self-Admission (Bootstrap)

The webhook's own pods run in the namespace `capacity-admission`. The
`ValidatingWebhookConfiguration` carries a `namespaceSelector` that skips the
webhook's **own** namespace only (key `kubernetes.io/metadata.name`, operator
`NotIn`, value `capacity-admission`) as apiserver-level defence-in-depth, so the
webhook never blocks its own deployment — even during cold start before its
Allocation cache is populated (FR-009).

Once the Allocation cache is populated, the webhook also self-exempts its own
namespace at runtime via `check_exemption` (FR-007). All other namespace and
priority-class exclusions are **operator-configured on the Allocation CRD**
(`spec.excludedNamespaces` / `spec.excludedPriorityClasses`, spec-008) and take
effect without re-deploying the `ValidatingWebhookConfiguration` — see
[Workload Exclusion](./workload-exclusion.md).
