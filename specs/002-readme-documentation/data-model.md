# Data Model: README Documentation

**Feature**: 002-readme-documentation | **Date**: 2026-07-26

This is a documentation deliverable, not a data system. The "data model" here
defines the README's **section structure** and the **reference tables** the
implementation agent must fill. Each table's values are locked in research.md;
the README must reproduce them verbatim.

---

## 1. README Section Tree

The README is a single file with the following top-level structure. Each
section maps to a spec user story and a set of FRs.

```
README.md
├── # emergency-ration-webhook          (title + one-liner)
├── Badges / status line                (optional, CI + license)
├── ## Overview                         (what + why; 2–3 paragraphs)
├── ## Quick Start                      (US1 / P1 / FR-001)
│   ├── ### Prerequisites
│   ├── ### Build the Image
│   ├── ### Deploy to Kubernetes
│   │   ├── CRDs + RBAC
│   │   ├── TLS Certificate
│   │   ├── Webhook Deployment + Service
│   │   └── ValidatingWebhookConfiguration
│   ├── ### Verify
│   └── ### TLS Provisioning            (cert-manager + manual Secret)
├── ## Configuration                    (US2 / P2 / FR-002,003,008,009)
│   ├── ### CLI Flags & Environment Variables  (7-row table)
│   ├── ### Precedence                   (flag > env > default)
│   ├── ### Allocation CRD              (spec + status field tables)
│   ├── ### ClusterCapacity CRD         (status field table)
│   └── ### Adjusting the Budget at Runtime
├── ## Metrics & Observability          (US3 / P3 / FR-004,005)
│   ├── ### HTTP Endpoints              (3-row table)
│   ├── ### Prometheus Metrics          (7-row table)
│   ├── ### Structured Logging          (fields + example)
│   └── ### Rejection Messages          (format + example)
├── ## Failure Modes                    (US3 / FR-006)
│   └── fail-closed table (6 rows)
├── ## Kubernetes Compatibility         (FR-007)
│   └── N-2 window + CI matrix
├── ## Architecture                     (brief, links to specs/001)
│   └── 3-component diagram (text/ASCII or table)
├── ## Development                      (build + test + contribute)
│   ├── ### Build
│   ├── ### Tests
│   └── ### Project Structure
└── ## License                          (Apache-2.0)
```

---

## 2. Configuration Reference Table (FR-002)

**Source of truth**: research.md §R1. Reproduce exactly.

| Flag | Env Var | Type | Default | Description |
|------|---------|------|---------|-------------|
| `--port` | `PORT` | u16 | 8443 | HTTPS port for the admission server |
| `--tls-cert-file` | `TLS_CERT_FILE` | path | `/tls/tls.crt` | TLS certificate path (PEM) |
| `--tls-key-file` | `TLS_KEY_FILE` | path | `/tls/tls.key` | TLS private key path (PEM) |
| `--decision-timeout-ms` | `DECISION_TIMEOUT_MS` | u64 | 100 | Admission decision timeout (ms); fails closed on expiry |
| `--capacity-freshness-timeout-secs` | `CAPACITY_FRESHNESS_TIMEOUT_SECS` | u64 | 30 | Max age (s) of capacity data before treated as stale |
| `--namespace` | `NAMESPACE` | string | `capacity-admission` | Namespace for the webhook and its CRDs |
| `--metrics-port` | `METRICS_PORT` | u16 | 9090 | HTTP port for `/metrics` and `/healthz` |

**Precedence** (FR-008): CLI flag → environment variable → compiled default.

---

## 3. CRD Reference Tables (FR-003)

### Allocation CRD

**Identity**: `allocations.emergency-ration.dev` (short: `alloc`), cluster-scoped,
singleton name: `cluster-allocation`.

**Spec fields**:

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `budgetPercent` | integer | 0–100 | Max allocation as % of total allocatable. Applied to CPU and RAM independently. |

**Status fields** (read-only — controller-computed):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `allocatedCpuMilli` | integer | milli-CPUs | Sum of pod CPU requests |
| `allocatedMemoryBytes` | integer | bytes | Sum of pod memory requests |
| `ceilingCpuMilli` | integer | milli-CPUs | `floor(totalCpuMilli × budgetPercent / 100)` |
| `ceilingMemoryBytes` | integer | bytes | Budget ceiling for memory |
| `utilizationPercentCpu` | number | ratio 0–1+ | `allocated / ceiling` for CPU |
| `utilizationPercentMemory` | number | ratio 0–1+ | `allocated / ceiling` for memory |
| `lastUpdated` | string | RFC 3339 | Last recomputation timestamp |

### ClusterCapacity CRD

**Identity**: `clustercapacities.emergency-ration.dev` (short: `cc`),
cluster-scoped, singleton name: `cluster-capacity`.

**Spec**: empty (no user-configurable fields).

**Status fields**:

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `totalAllocatableCpuMilli` | integer | milli-CPUs | Total allocatable CPU across all nodes |
| `totalAllocatableMemoryBytes` | integer | bytes | Total allocatable memory across all nodes |
| `nodeCount` | integer | count | Number of nodes counted |
| `lastUpdated` | string | RFC 3339 | Last recomputation timestamp |

---

## 4. HTTP Endpoints Table (FR-004)

| Endpoint | Protocol | Port | Path | Purpose |
|----------|----------|------|------|---------|
| Admission webhook | HTTPS | 8443 | `/validate` | AdmissionReview processing |
| Metrics | HTTP | 9090 | `/metrics` | Prometheus scrape |
| Health | HTTP | 9090 | `/healthz` | kubelet liveness/readiness probe |

---

## 5. Metrics Reference Table (FR-005)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `capacity_admission_verdicts_total` | counter | `resource`, `verdict` | Admission verdicts (allow/deny/error) by resource |
| `capacity_admission_decision_duration_seconds` | histogram | — | Decision latency. Buckets: .005,.01,.025,.05,.075,.1,.25,.5,1.0 |
| `capacity_admission_capacity_freshness_seconds` | gauge | — | Seconds since Allocation CRD status last refreshed |
| `capacity_admission_allocation_ratio` | gauge | `resource` | Allocated / ceiling ratio (0.0–1.0+) |
| `capacity_admission_total_allocatable` | gauge | `resource` | Total allocatable capacity per resource |
| `capacity_admission_current_allocation` | gauge | `resource` | Currently allocated capacity per resource |
| `capacity_admission_ceiling` | gauge | `resource` | Budget ceiling per resource |

**Labels**: `resource ∈ {cpu, memory}`, `verdict ∈ {allow, deny, error}`.

---

## 6. Failure Modes Table (FR-006)

| Condition | Outcome | Logged Reason |
|-----------|---------|---------------|
| Capacity data stale (age > freshness timeout) | Reject | capacity data unavailable/stale |
| ClusterCapacity or Allocation CRD missing | Reject | capacity data unavailable |
| Component unreachable / controller down | Reject | capacity data unavailable |
| Malformed / undiserisable admission request | Reject | deserialisation failure |
| Decision timeout exceeded | Reject | decision timeout |
| Internal panic / unknown error | Reject | unknown error (catch-all) |

`failurePolicy: Fail` ensures the apiserver rejects if the webhook itself is
unreachable.

---

## 7. Validation Rules

The README content MUST satisfy these constraints (checked in quickstart.md):

- **VR-001**: Every flag in §2 exists in `src/config.rs` with the exact name,
  env-var, type, and default.
- **VR-002**: Every CRD field in §3 exists in `src/crd/*.rs` with the exact
  name, type, and serde casing (`camelCase` in JSON).
- **VR-003**: Every metric name in §5 exists in `src/metrics.rs` with the
  exact type and labels.
- **VR-004**: Every endpoint/port in §4 matches `src/main.rs` and
  `deploy/deployment.yaml`.
- **VR-005**: The budget formula and inclusive-ceiling semantics in §3 match
  `src/webhook/admission.rs` (`check_budget`, `ceiling`).
- **VR-006**: The failure modes in §6 match `src/webhook/error.rs` and the
  handler's error-path mapping.
- **VR-007**: The Kubernetes version matrix matches `.github/workflows/ci.yml`.
- **VR-008**: The namespace exclusion list matches
  `deploy/webhook-config.yaml`.
