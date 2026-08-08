# Metrics & Observability

[← Back to README](../README.md)

## HTTP Endpoints

The webhook serves three endpoints across two ports (source:
[`src/main.rs`](../src/main.rs), [`src/webhook/handler.rs`](../src/webhook/handler.rs),
[`deploy/deployment.yaml`](../deploy/deployment.yaml)):

| Endpoint | Protocol | Port | Path | Purpose |
|----------|----------|------|------|---------|
| Admission webhook | HTTPS (TLS) | 8443 | `/validate` | AdmissionReview processing (referenced by the ValidatingWebhookConfiguration) |
| Metrics | HTTP (plaintext) | 9090 | `/metrics` | Prometheus scrape |
| Health | HTTP (plaintext) | 9090 | `/healthz` | kubelet liveness/readiness probe |

The `/metrics` and `/healthz` endpoints share plaintext HTTP port 9090 so
Prometheus and the kubelet can reach them without TLS; the admission endpoint is
on a separate HTTPS port (8443).

> **Metrics port exposure**: port 9090 is plaintext HTTP with no authentication.
> Do not expose it externally without an additional network policy or auth layer
> — anyone who can reach it can read cluster capacity figures.

## Prometheus Metrics

All metrics are registered on a single registry and prefixed
`capacity_admission_` (source: [`src/metrics.rs`](../src/metrics.rs)). Every series
is pre-created at startup, so a scrape sees the full surface at zero before the
first decision. Label vocabularies: `resource ∈ {cpu, memory}`,
`verdict ∈ {allow, deny, dry_run_deny, error}`,
`reason ∈ {namespace, priority_class, webhook_namespace}` (spec-008).

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `capacity_admission_verdicts_total` | counter | `resource`, `verdict` | Admission decisions by resource and verdict (allow/deny/dry_run_deny/error). `dry_run_deny` is a dry-run mode would-be-rejection (spec-004); query `verdict=~"deny\|dry_run_deny"` for the combined view. Budget decisions only — an exempt decision does **not** increment this counter (see `capacity_admission_exemptions_total`). |
| `capacity_admission_exemptions_total` | counter | `reason` | Admissions bypassing the budget via the exclusion policy (spec-008). `reason ∈ {namespace, priority_class, webhook_namespace}`. An exempt decision increments this counter and **not** `capacity_admission_verdicts_total`, keeping the verdict counter semantically budget-only. |
| `capacity_admission_decision_duration_seconds` | histogram | — | Admission decision latency (seconds). Buckets: 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 1.0 |
| `capacity_admission_capacity_freshness_seconds` | gauge | — | Seconds since the Allocation CRD status was last refreshed |
| `capacity_admission_allocation_ratio` | gauge | `resource` | Allocated / ceiling ratio per resource (0.0–1.0+) |
| `capacity_admission_total_allocatable` | gauge | `resource` | Total allocatable capacity per resource |
| `capacity_admission_current_allocation` | gauge | `resource` | Currently allocated capacity per resource |
| `capacity_admission_ceiling` | gauge | `resource` | Budget ceiling per resource |

A scrape after deploying returns all eight families, each with `# HELP` and
`# TYPE` lines:

```sh
kubectl -n capacity-admission port-forward svc/capacity-admission-webhook 9090:metrics &
curl -s localhost:9090/metrics
```

## Structured Logging

The webhook uses [`tracing`](https://docs.rs/tracing) with structured fields
(target `capacity_admission`). The log level is taken from the `RUST_LOG`
environment variable and **defaults to `info`** (source: [`src/main.rs`](../src/main.rs);
the `Deployment` sets `RUST_LOG=info`).

Every admission decision emits one structured event carrying the workload
identity, the decision, and the capacity figures used. Key fields:

| Field | Meaning |
|-------|---------|
| `workload` | `<namespace>/<name>` of the triggering workload |
| `operation` | `CREATE`, `UPDATE`, `DELETE`, or `CONNECT` |
| `decision` | `allow`, `deny`, `dry_run_deny` (spec-004), `exempt` (spec-008), or `error` |
| `resource_type` | `cpu` or `memory` (one event per resource on allow/deny) |
| `allocated` / `requested` / `projected` / `ceiling` | Capacity figures for the resource |
| `budget_percent` | The `spec.budgetPercent` value — the legacy fallback budget, kept for back-compat in existing log consumers |
| `effective_cpu_budget_percent` | The effective CPU budget that governed this decision — `cpuBudgetPercent` if set, else `budgetPercent` (spec-012). `-1` on fail-closed / exempt paths. |
| `effective_memory_budget_percent` | The effective memory budget (spec-012). Symmetric to `effective_cpu_budget_percent`. |
| `enforcement_mode` | The active `enforcementMode` for the decision — `enforce` or `dry-run` (spec-004; present on every decision) |
| `exemption_reason` | On `exempt`: the criterion that triggered the bypass — `namespace`, `priority_class`, or `webhook_namespace` (spec-008) |
| `freshness_seconds` | Age of the cached Allocation status |
| `latency_ms` | Decision latency in milliseconds |
| `reason` | On `deny`/`dry_run_deny`: `<resource>_over_budget`; on `error`: the failure slug |

- An **allow** logs at INFO (one event per resource).
- A **deny** logs at WARN (one event per resource, `reason` names the violated resource).
- A **dry_run_deny** logs at WARN with `decision=dry_run_deny` — a dry-run mode
  would-be-rejection (the pod was admitted with a warning; spec-004).
- An **exempt** logs at INFO with `decision=exempt` and `exemption_reason` — the
  pod was admitted by the exclusion policy with no budget check (spec-008). One
  event (no per-resource figures).
- An **error** logs at ERROR with the failure `reason`.

Example (admission allowed, CPU line):

```text
2026-07-26T14:32:05.123Z  INFO capacity_admission: admission allowed \
  workload=default/api-server operation=CREATE decision=allow resource_type=cpu \
  allocated=70000 requested=5000 projected=75000 ceiling=80000 \
  budget_percent=80 effective_cpu_budget_percent=95 effective_memory_budget_percent=30 \
  freshness_seconds=2 latency_ms=3
```

## Rejection Messages

When a pod exceeds the budget, the AdmissionResponse carries a human-readable
`message` built from one violation per exceeded resource (source:
[`src/webhook/error.rs`](../src/webhook/error.rs)). Each line names the resource,
the current allocation, the requested increment, the projected total, and the
ceiling. When both CPU and RAM exceed, **both** lines are emitted, newline
separated, **CPU first**.

Format per resource:

```text
CPU budget exceeded: allocated <a>m, requested <r>m, projected <p>m, ceiling <c>m
memory budget exceeded: allocated <a> bytes, requested <r> bytes, projected <p> bytes, ceiling <c> bytes
```

Example — a pod whose projected CPU would reach 85000m against an 80000m ceiling:

```text
CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m
```

CPU is reported in milli-cores (`m`); memory in bytes. An over-budget rejection
returns HTTP `403`; a malformed request returns `400`; all other fail-closed
rejections return `500` (see [Failure Modes](./failure-modes.md)).
