# Contract: Admission Webhook Endpoint

**Phase**: 1 (Design) | **Date**: 2026-07-26

This contract defines the HTTP interface exposed by the admission webhook to
the Kubernetes API server.

---

## Endpoint

| Method | Path | Content-Type | Purpose |
|--------|------|-------------|---------|
| `POST` | `/validate` | `application/json` | Receive an AdmissionReview, return an AdmissionReview |
| `GET` | `/metrics` | `text/plain` | Prometheus metrics exposition |
| `GET` | `/healthz` | `text/plain` | Liveness/readiness probe |

The server listens on HTTPS (TLS 1.2+ via `rustls`). Port is configurable
(`--port`, default `8443`). Certificate and key are read from mounted files
(`--tls-cert-file` `/tls/tls.crt`, `--tls-key-file` `/tls/tls.key`).

---

## AdmissionReview Request/Response

### Request Body

The apiserver sends a `POST /validate` with an `AdmissionReview` object
(core/v1 serialisation). The relevant fields:

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "request": {
    "uid": "705ab4f5-639d-11e2-8a6a-9e4c1d4fc3b1",
    "kind": { "group": "", "version": "v1", "kind": "Pod" },
    "resource": { "group": "", "version": "v1", "resource": "pods" },
    "name": "example-pod",
    "namespace": "default",
    "operation": "CREATE",
    "userInfo": { "username": "system:..." },
    "object": { /* serialised Pod spec */ },
    "oldObject": null,   /* present on UPDATE */
    "dryRun": false,
    "options": { "kind": "CreateOptions", "apiVersion": "meta.k8s.io/v1" }
  }
}
```

The webhook only processes `operation` ∈ `{CREATE, UPDATE}` for `kind: Pod`.
All other operations and kinds are admitted without evaluation (the
ValidatingWebhookConfiguration `rules` filter ensures only CREATE/UPDATE of
pods reaches the webhook).

### Response Body

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "response": {
    "uid": "<echoed from request>",
    "allowed": false,
    "status": {
      "code": 403,
      "message": "capacity budget exceeded: CPU projected 85000m > ceiling 80000m ..."
    }
  }
}
```

**Fields**:
- `uid`: echoed from `request.uid`.
- `allowed`: `true` (admit) or `false` (deny). **Always `false` on any error
  path** (Principle I).
- `status.message`: human-readable explanation. For denials, includes the
  violated resource(s), current allocation, requested increment, projected
  total, and ceiling (SC-002). For errors, identifies the failure mode.
- `status.code`: HTTP status code. `403` for policy denial, `500` for internal
  error (but `allowed` remains `false`).

---

## Error Path Matrix

Every path that is not a clean budget-check admission results in `allowed:
false`. There is no path that returns `allowed: true` under error conditions.

| Condition | `allowed` | `status.code` | `status.message` format | Log level |
|-----------|-----------|--------------|------------------------|-----------|
| Pod fits within budget | `true` | (omitted) | (omitted) | INFO |
| CPU over budget | `false` | 403 | `CPU budget exceeded: allocated {A}m, requested {R}m, projected {P}m, ceiling {C}m` | WARN |
| Memory over budget | `false` | 403 | `memory budget exceeded: allocated {A} bytes, requested {R} bytes, projected {P} bytes, ceiling {C} bytes` | WARN |
| Both over budget | `false` | 403 | Both messages, newline-separated | WARN |
| Capacity data stale (lastUpdated > threshold) | `false` | 500 | `capacity data unavailable: last refresh {T}s ago exceeds {threshold}s threshold` | ERROR |
| Allocation CRD not found / not yet populated | `false` | 500 | `capacity data unavailable: allocation state not initialised` | ERROR |
| ClusterCapacity CRD not found | `false` | 500 | `capacity data unavailable: cluster capacity state not initialised` | ERROR |
| AdmissionReview deserialisation failure | `false` | 400 | `admission request malformed: {parse error}` | ERROR |
| Resource quantity parse failure (in pod spec) | `false` | 400 | `cannot parse resource quantity in pod spec: {field}={value}` | ERROR |
| Request timeout exceeded | `false` | 500 | `admission decision timed out after {timeout}ms` | ERROR |
| Internal panic (catch_unwind) | `false` | 500 | `internal error: panic in admission handler` | ERROR |
| Any other unhandled error | `false` | 500 | `internal error: {error description}` | ERROR |

**Key invariant**: the "Any other unhandled error" row is the catch-all that
guarantees Principle III's "no third category" — unknown error types reject by
default.

---

## Logging Contract

Every admission decision emits a structured log entry (`tracing` span).
Fields:

| Field | Type | Present on | Example |
|-------|------|-----------|---------|
| `workload` | string | all | `default/example-pod` |
| `operation` | string | all | `CREATE` |
| `decision` | string | all | `allow` / `deny` |
| `reason` | string | deny/error | `cpu_over_budget` |
| `resource_type` | string | all | `cpu` / `memory` |
| `allocated` | integer | all | `70000` (milli-CPUs) |
| `requested` | integer | all | `15000` |
| `projected` | integer | all | `85000` |
| `ceiling` | integer | all | `80000` |
| `budget_percent` | integer | all | `80` |
| `freshness_seconds` | integer | all | `12` (seconds since last CRD update) |
| `latency_ms` | integer | all | `3` |
| `error` | string | error only | `capacity data stale` |

---

## Webhook Configuration Contract

The `ValidatingWebhookConfiguration` that registers this webhook:

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingWebhookConfiguration
metadata:
  name: capacity-admission.emergency-ration.dev
webhooks:
  - name: capacity-admission.emergency-ration.dev
    admissionReviewVersions: ["v1"]
    sideEffects: None
    failurePolicy: Fail           # Principle I: fail-closed
    matchPolicy: Exact
    timeoutSeconds: 5             # apiserver-level timeout; webhook internal timeout is tighter
    clientConfig:
      service:
        name: capacity-admission-webhook
        namespace: capacity-admission
        path: "/validate"
        port: 8443
      caBundle: <base64-encoded CA cert>   # injected by cert-manager or manual
    rules:
      - apiGroups: [""]
        apiVersions: ["v1"]
        resources: ["pods"]
        operations: ["CREATE", "UPDATE"]
        scope: "*"
    namespaceSelector:
      matchExpressions:
        # Skip namespaces where the webhook itself runs (bootstrap problem)
        - key: kubernetes.io/metadata.name
          operator: NotIn
          values: ["capacity-admission", "kube-system", "kube-public"]
```

**Key fields**:
- `failurePolicy: Fail` — if the webhook is unreachable, the apiserver rejects
  the pod (Principle I).
- `sideEffects: None` — the webhook is validating-only, no side effects
  (required for `v1` admission webhooks).
- `timeoutSeconds: 5` — the apiserver times out after 5s; the webhook's
  internal timeout (`--decision-timeout`, default 100ms) is much tighter to
  fail fast.
- `namespaceSelector` excludes the webhook's own namespace and system
  namespaces (handles the bootstrap problem from spec edge cases).
