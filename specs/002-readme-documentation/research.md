# Research: README Documentation

**Feature**: 002-readme-documentation | **Date**: 2026-07-26

This is a documentation-only feature. The "research" is therefore not
technology selection — it is the authoritative enumeration of the user-facing
surface the README MUST document, derived from the shipped code on `main`.
Every value below is the source of truth; the README must match it exactly.

---

## R1: CLI Flags and Environment Variables

**Source**: `src/config.rs` — `impl Default for Config` (defaults) +
`Config::from_args_and_env` (flag/env names) + `resolve` (precedence).

**Precedence rule**: CLI flag (`--flag value`) → environment variable →
compiled default. Unparseable values fall back to the default silently.

| Flag | Env Var | Type | Default | Description |
|------|---------|------|---------|-------------|
| `--port` | `PORT` | `u16` | `8443` | HTTPS port the admission server listens on |
| `--tls-cert-file` | `TLS_CERT_FILE` | `PathBuf` | `/tls/tls.crt` | Path to the TLS certificate (PEM) |
| `--tls-key-file` | `TLS_KEY_FILE` | `PathBuf` | `/tls/tls.key` | Path to the TLS private key (PEM) |
| `--decision-timeout-ms` | `DECISION_TIMEOUT_MS` | `u64` | `100` | Per-request admission decision timeout (ms); webhook fails closed on elapsed time |
| `--capacity-freshness-timeout-secs` | `CAPACITY_FRESHNESS_TIMEOUT_SECS` | `u64` | `30` | Maximum age (seconds) of cached capacity data before treated as stale |
| `--namespace` | `NAMESPACE` | `String` | `capacity-admission` | Namespace the webhook and its CRDs live in |
| `--metrics-port` | `METRICS_PORT` | `u16` | `9090` | HTTP port for `/metrics` and `/healthz` |

**Decision**: Document all seven in a single table with flag, env-var, type,
default, and description columns. State the precedence rule explicitly above
the table.

---

## R2: Allocation CRD

**Source**: `src/crd/allocation.rs`.

- **Group/Version/Kind**: `emergency-ration.dev/v1`, `Allocation`
- **Short name**: `alloc`
- **Scope**: `Cluster` (singleton, convention name: `cluster-allocation`)
- **Spec** (user-configurable):

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `budgetPercent` | `i32` | `minimum: 0, maximum: 100` | Maximum allowed allocation as a percentage of total allocatable capacity. Applied to both CPU and RAM independently. |

- **Status** (controller-computed, read-only for operators):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `allocatedCpuMilli` | `i64` | milli-CPUs | Currently allocated CPU (sum of pod requests) |
| `allocatedMemoryBytes` | `i64` | bytes | Currently allocated memory (sum of pod requests) |
| `ceilingCpuMilli` | `i64` | milli-CPUs | Budget ceiling for CPU = `floor(totalAllocatableCpuMilli × budgetPercent / 100)` |
| `ceilingMemoryBytes` | `i64` | bytes | Budget ceiling for memory |
| `utilizationPercentCpu` | `f64` | ratio 0.0–1.0+ | Utilisation ratio for CPU (allocated / ceiling) |
| `utilizationPercentMemory` | `f64` | ratio 0.0–1.0+ | Utilisation ratio for memory |
| `lastUpdated` | `String` | RFC 3339 timestamp | Timestamp of the last allocation recomputation |

---

## R3: ClusterCapacity CRD

**Source**: `src/crd/cluster_capacity.rs`.

- **Group/Version/Kind**: `emergency-ration.dev/v1`, `ClusterCapacity`
- **Short name**: `cc`
- **Scope**: `Cluster` (singleton, convention name: `cluster-capacity`)
- **Spec**: empty (`{}`) — supply-side, controller-written, no user-configurable
  fields.
- **Status** (controller-computed):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `totalAllocatableCpuMilli` | `i64` | milli-CPUs | Total allocatable CPU across all nodes |
| `totalAllocatableMemoryBytes` | `i64` | bytes | Total allocatable memory across all nodes |
| `nodeCount` | `i32` | count | Number of nodes counted |
| `lastUpdated` | `String` | RFC 3339 timestamp | Timestamp of the last capacity recomputation |

---

## R4: HTTP Endpoints

**Source**: `src/main.rs` (bind + routes) + `src/webhook/handler.rs` (handlers).

| Endpoint | Protocol | Port | Path | Purpose |
|----------|----------|------|------|---------|
| Admission webhook | HTTPS (TLS) | `8443` | `/validate` | AdmissionReview in → AdmissionReview out. Referenced by ValidatingWebhookConfiguration. |
| Metrics | HTTP (plaintext) | `9090` | `/metrics` | Prometheus scrape endpoint |
| Health | HTTP (plaintext) | `9090` | `/healthz` | kubelet liveness/readiness probe |

**Note**: The metrics and health endpoints share port 9090 (plaintext HTTP) so
Prometheus and kubelet probes can reach them without TLS. The admission
endpoint is on a separate HTTPS port (8443).

---

## R5: Prometheus Metrics

**Source**: `src/metrics.rs` — `Metrics::new()` registrations.

All metrics are prefixed `capacity_admission_`. Labels: `resource ∈ {cpu,
memory}`, `verdict ∈ {allow, deny, error}`.

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `capacity_admission_verdicts_total` | counter | `resource`, `verdict` | Admission decisions by resource and verdict (allow/deny/error) |
| `capacity_admission_decision_duration_seconds` | histogram | — | Admission decision latency in seconds. Buckets: 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 1.0 |
| `capacity_admission_capacity_freshness_seconds` | gauge | — | Seconds since the Allocation CRD status was last refreshed |
| `capacity_admission_allocation_ratio` | gauge | `resource` | Allocated / ceiling ratio per resource (0.0–1.0+) |
| `capacity_admission_total_allocatable` | gauge | `resource` | Total allocatable capacity per resource |
| `capacity_admission_current_allocation` | gauge | `resource` | Currently allocated capacity per resource |
| `capacity_admission_ceiling` | gauge | `resource` | Budget ceiling per resource |

---

## R6: Budget Calculation and Rejection Message Format

**Source**: `src/webhook/admission.rs` (`check_budget`, `ceiling`) +
`src/webhook/error.rs` (`BudgetViolation`).

**Budget formula**: `ceiling = floor(totalAllocatable × budgetPercent / 100)`
per resource. Ceiling is **inclusive**: `projected == ceiling` admits;
`projected == ceiling + 1` denies.

**Rejection message format**: each violation names the resource, current
allocation, requested increment, projected total, and ceiling. When both CPU
and RAM are exceeded, both violations are reported (CPU first). Example from
`BudgetViolation`:

```
CPU would reach 85000m / 80000m (allocated 70000m + requested 15000m)
```

---

## R7: Deployment Manifests

**Source**: `deploy/*.yaml`.

| File | Contains |
|------|----------|
| `deploy/namespace.yaml` | (merged into deployment.yaml) Namespace `capacity-admission` |
| `deploy/deployment.yaml` | Namespace + Deployment (2 replicas, resource limits, probes, TLS volume) + Service (ports 8443, 9090) |
| `deploy/rbac.yaml` | ServiceAccount + ClusterRole (read nodes/pods, write CRD status) + ClusterRoleBinding |
| `deploy/crds.yaml` | ClusterCapacity + Allocation CRD definitions |
| `deploy/webhook-config.yaml` | ValidatingWebhookConfiguration (failurePolicy: Fail, namespaceSelector exclusion) |
| `deploy/cert-setup.yaml` | cert-manager Certificate (or manual Secret reference) |

**Key deployment details** (from `deployment.yaml`):
- Replicas: 2 (redundancy)
- Resource requests: 100m CPU / 128Mi memory; limits: 500m / 256Mi
- Probes: `/healthz` on port 9090 (liveness + readiness)
- Security: runAsNonRoot, runAsUser 65532, readOnlyRootFilesystem,
  seccomp RuntimeDefault, drop ALL capabilities
- TLS: mounted from Secret `capacity-admission-webhook-tls` at `/tls`
- `namespaceSelector` excludes: `capacity-admission`, `kube-system`,
  `kube-public`

---

## R8: Kubernetes Version Support Window

**Source**: CI workflow `.github/workflows/ci.yml` (version matrix).

**Support window**: N-2 — the three most recent Kubernetes releases. As of the
implementation date, CI tests against **1.34, 1.35, 1.36**.

All APIs used are GA/stable:
- `admissionregistration.k8s.io/v1` (ValidatingWebhookConfiguration)
- `apiextensions.k8s.io/v1` (CustomResourceDefinition)
- Core `v1` (Pod, Node)

---

## R9: Fail-Closed Behaviour

**Source**: `src/webhook/handler.rs` + `src/webhook/error.rs` +
`deploy/webhook-config.yaml`.

Every non-verifiable admission path rejects (`allowed: false`):

| Condition | Outcome | Reason |
|-----------|---------|--------|
| Capacity data stale (age > freshness timeout) | Reject | capacity data unavailable/stale |
| ClusterCapacity or Allocation CRD missing | Reject | capacity data unavailable |
| Component unreachable / controller down | Reject | capacity data unavailable |
| Admission request malformed / deserialisation failure | Reject | deserialisation failure |
| Decision timeout exceeded | Reject | decision timeout |
| Internal panic / unknown error | Reject | unknown error (catch-all) |

`failurePolicy: Fail` on the ValidatingWebhookConfiguration ensures the
apiserver itself rejects if the webhook is unreachable.

---

## R10: README Best Practices for Kubernetes Operators

**Decision**: Follow the established structure of well-documented Kubernetes
admission webhooks and operators (e.g. Kyverno, OPA Gatekeeper, kubectl
plugins). The README should be structured for an operator who scans top-to-
bottom: what it is → why use it → install → configure → operate → troubleshoot.

**Structure** (see data-model.md for the full section tree):

1. Title + one-line description + badges
2. Overview (what / why)
3. Quick Start (clone → running)
4. Configuration (flags + CRDs)
5. Metrics & Observability
6. Architecture (brief, links to specs/)
7. Kubernetes Compatibility
8. Development (build, test, contribute)

**Rationale**: This ordering front-loads the operator's primary journey
(install) and defers architecture/internals. It mirrors the constitution's
priority ordering: the webhook exists to enforce budgets (quick start gets
them there), then observability (metrics), then understanding (architecture).
