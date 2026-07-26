# Contract: Admission Webhook Endpoint (Dry-Run Amendment)

**Phase**: 1 (Design) | **Date**: 2026-07-27 | **Amends**: spec-001
`contracts/admission-webhook.md`

This contract amendment documents the changes to the admission webhook HTTP
interface when the enforcement mode is `dry-run`. All behaviour from the
original contract remains unchanged in `enforce` mode.

---

## Response Body: Dry-Run Mode

When `enforcementMode: dry-run` and the pod's projected allocation exceeds the
budget, the response admits the pod with a warning instead of rejecting it:

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "response": {
    "uid": "<echoed from request>",
    "allowed": true,
    "warnings": [
      "Budget violations (dry-run): CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m"
    ]
  }
}
```

**Fields**:
- `uid`: echoed from `request.uid` (unchanged).
- `allowed`: `true` — the pod is admitted.
- `warnings`: array of warning strings. Each warning carries the same message
  format as a real rejection (see Error Path Matrix below), prefixed with
  `"Budget violations (dry-run): "`. When both CPU and RAM are over budget,
  both violation lines are included, newline-separated within a single warning
  string.
- `status`: **omitted** — the pod is admitted, not rejected. No status code or
  message is set.

**Multiple violations**: when both CPU and memory exceed the budget, the warning
contains both lines:

```json
"warnings": [
  "Budget violations (dry-run): CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m\nmemory budget exceeded: allocated 70000 bytes, requested 15000 bytes, projected 85000 bytes, ceiling 80000 bytes"
]
```

---

## Error Path Matrix (Dry-Run Mode)

Every fail-closed path rejects **regardless of enforcement mode**. Dry-run only
converts over-budget denials; it does not convert error rejections.

| Condition | Mode | `allowed` | `status.code` | Warnings | Log level |
|-----------|------|-----------|--------------|----------|-----------|
| Pod fits within budget | any | `true` | (omitted) | (none) | INFO |
| CPU/memory over budget | **enforce** | `false` | 403 | (none) | WARN |
| CPU/memory over budget | **dry-run** | **`true`** | (omitted) | Budget violation message(s) | WARN (`dry_run_deny`) |
| Capacity data stale | **dry-run** | `false` | 500 | (none) | ERROR |
| Allocation/ClusterCapacity missing | **dry-run** | `false` | 500 | (none) | ERROR |
| Deserialisation failure | **dry-run** | `false` | 400 | (none) | ERROR |
| Quantity parse failure | **dry-run** | `false` | 400 | (none) | ERROR |
| Timeout | **dry-run** | `false` | 500 | (none) | ERROR |
| Internal panic | **dry-run** | `false` | 500 | (none) | ERROR |
| Unknown error | **dry-run** | `false` | 500 | (none) | ERROR |

**Key invariant (unchanged)**: there is no path that returns `allowed: true`
under error conditions, in either enforcement mode.

---

## Logging Contract (Dry-Run Amendment)

Every log entry gains an `enforcement_mode` field. A new `decision` value
`dry_run_deny` distinguishes dry-run would-be-rejections.

| Field | Type | Present on | Example |
|-------|------|-----------|---------|
| `enforcement_mode` | string | **all** (NEW) | `enforce` / `dry_run` |
| `decision` | string | all | `allow` / `deny` / **`dry_run_deny`** (NEW) / `error` |

All other fields (`workload`, `operation`, `reason`, `resource_type`,
`allocated`, `requested`, `projected`, `ceiling`, `budget_percent`,
`freshness_seconds`, `latency_ms`, `error`) are unchanged.

Dry-run deny logs at **WARN** level (same as enforce deny), with:
- `decision = "dry_run_deny"`
- `reason` naming the violated resource (e.g. `cpu_over_budget`)
- `enforcement_mode = "dry_run"`
- All capacity figures populated (same as a real deny)

---

## Metrics (Dry-Run Amendment)

The `capacity_admission_verdicts_total` counter gains a new verdict label value:

| Label | Values |
|-------|--------|
| `verdict` | `allow`, `deny`, **`dry_run_deny`** (NEW), `error` |

New pre-created series (at zero from startup):
- `capacity_admission_verdicts_total{resource="cpu",verdict="dry_run_deny"}`
- `capacity_admission_verdicts_total{resource="memory",verdict="dry_run_deny"}`

An operator can query combined denies with
`verdict=~"deny|dry_run_deny"`, or filter to dry-run-only with
`verdict="dry_run_deny"`.

---

## Webhook Configuration Contract (No Change)

The `ValidatingWebhookConfiguration` is unchanged. Dry-run mode is a
webhook-internal toggle read from the Allocation CRD spec, not a registration
change. The webhook still uses `failurePolicy: Fail`, still processes
CREATE/UPDATE on pods, and the apiserver handles the `warnings` field natively.
