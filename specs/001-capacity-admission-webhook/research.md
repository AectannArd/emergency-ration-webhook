# Research: Capacity Admission Webhook

**Phase**: 0 (Research) | **Date**: 2026-07-26

This document resolves every technical unknown left open by the constitution
and spec, with a Decision / Rationale / Alternatives format.

---

## R1. Rust toolchain version and edition

**Decision**: Rust 1.89 MSRV (edition 2024).

**Rationale**: `kube-rs` 4.2.0 declares MSRV 1.89 in its CI badge. Rust stable
as of 2026-07-26 is 1.97.1, so 1.89 is a conservative floor that covers the
widest range of contributor toolchains while remaining compatible with the
primary dependency. Edition 2024 is the current stable edition and is fully
supported by rustc ≥ 1.85.

**Alternatives considered**:
- Pin MSRV to latest stable (1.97): rejected — unnecessarily restrictive for
  contributors who may have slightly older toolchains in CI environments.
- Pin MSRV lower than 1.89: rejected — `kube-rs` 4.x would not compile.

---

## R2. kube-rs version and feature selection

**Decision**: `kube = "4.2.0"` with features `["runtime", "derive", "client",
"rustls-tls"]`.

**Rationale**: 4.2.0 is the current release (GitHub README, docs.rs
`kube-derive/4.2.0`). Features map directly to needs:
- `runtime` — `watcher`, `reflector`, `Controller` for the two controllers.
- `derive` — `#[derive(CustomResource)]` for the two CRDs.
- `client` — `kube::Client` for API-server communication.
- `rustls-tls` — TLS via `rustls` (no OpenSSL dependency, pure-Rust crypto,
  static linking with musl). Matches the constitution's `rustls` constraint.

**Alternatives considered**:
- `openssl-tls`: rejected — adds an OpenSSL system dependency, complicating
  the static container build.
- `kube` 0.99 (older): rejected — superseded by 4.x; the 0.x → 4.x renumber
  reflects the crate's graduation to a stable versioning scheme.

**Related dependencies** (version-locked to `kube` 4.2.0):
- `k8s-openapi = "0.28.0"` (features: `latest`, `schemars`) — kube 4.2.0
  requires k8s-openapi 0.28.
- `schemars = "1.0"` — kube 4.x requires schemars 1.0 (the `JsonSchema` derive
  changed between 0.8 and 1.0).

---

## R3. Kubernetes version support matrix (Principle VII)

**Decision**: Support Kubernetes 1.34, 1.35, 1.36 (the three most recent minor
releases as of 2026-07-26, per kubernetes.io/releases).

**Rationale**: The Kubernetes project maintains release branches for 1.34,
1.35, and 1.36. Principle VII mandates N-2 support. All APIs used are GA:
- `admissionregistration.k8s.io/v1` ValidatingWebhookConfiguration — GA since
  1.19.
- `apiextensions.k8s.io/v1` CustomResourceDefinition — GA since 1.16.
- core/v1 Node, Pod — GA since 1.0.

No alpha or beta APIs are used, so there is no version-skew risk within the
window.

**Alternatives considered**:
- Support only latest (1.36): rejected — violates Principle VII.
- Extend to N-3 (1.33): rejected — 1.33 reached EOL on 2026-06-28; supporting
  EOL versions creates a maintenance burden and security risk.

---

## R4. CRD design: ClusterCapacity and Allocation

**Decision**: Two cluster-scoped CRDs:

1. **ClusterCapacity** (`clustercapacity.emergency-ration.dev/v1`):
   - **Spec**: (empty — this is a singleton populated by the controller).
     Actually: the spec carries the `budgetPercent` configuration so the
     supply-side CRD also defines the policy ceiling. See correction below.
   - **Status**: `totalAllocatableCPU` (milli-CPUs), `totalAllocatableMemory`
     (bytes), `nodeCount`, `lastUpdated` (timestamp).

   *Correction after cross-checking with the constitution*: The constitution
   assigns the budget threshold to the **Allocation CRD** spec (Principle V:
   "holds the target allocation threshold (in `spec`)"). ClusterCapacity is
   supply-only:
   - **Spec**: empty (or a single field identifying this as the singleton).
   - **Status**: total allocatable CPU + RAM, node count, last-updated timestamp.

2. **Allocation** (`allocation.emergency-ration.dev/v1`):
   - **Spec**: `budgetPercent` (integer 0–100, per resource type — CPU and RAM
     share one percentage in v1; per-resource is a future concern).
   - **Status**: `allocatedCPU`, `allocatedMemory`, `ceilingCPU`, `ceilingMemory`,
     `utilizationPercentCPU`, `utilizationPercentMemory`, `lastUpdated`.

Both are cluster-scoped (the budget is cluster-wide per the spec's assumptions).

**Rationale**: The `kube::CustomResource` derive macro generates both the Rust
type and the CRD YAML from a single `Spec` struct + `#[kube(...)]` attributes
(verified in docs.rs/kube documentation). The `status` subresource is enabled
via `#[kube(status = "...")]`, which keeps controller-written status separate
from user-authored spec — preventing update conflicts.

**Alternatives considered**:
- One CRD combining supply + demand: rejected — violates Principle V's mandate
  to separate node lifecycle from pod lifecycle.
- Namespaced CRDs: rejected — the budget is cluster-wide in v1; namespaced
  would imply per-namespace budgets (explicitly deferred).
- ConfigMap/Secret instead of CRDs: rejected — CRDs provide typed status
  subresources, validation schemas, and watch semantics that ConfigMaps lack.

---

## R5. Controller runtime pattern (watcher/reflector/Controller)

**Decision**: Use `kube::runtime` primitives:
- **Node Capacity Controller**: a `reflector` on `Api::<Node>::all(client)` →
  on every Node event, re-sum `.status.allocatable` across all cached nodes →
  patch the `ClusterCapacity` CRD status.
- **Allocation Controller**: a `reflector` on `Api::<Pod>::all(client)` +
  watches the `ClusterCapacity` CRD → on every Pod or capacity event, re-sum
  resource requests across non-terminal pods → patch the `Allocation` CRD
  status (allocation + recomputed ceiling).
- **Admission Webhook**: a `reflector` on the `Allocation` CRD → the webhook
  handler reads the cached allocation state for each AdmissionReview, avoiding
  any API-server call on the hot path.

**Rationale**: `kube::runtime::reflector` maintains an eventually-consistent
in-memory `Store` backed by a `watcher` (which handles relists, bookmarks, and
reconnection with backoff automatically). This is the standard `kube-rs` pattern
(documented in the GitHub README's Watchers/Reflectors/Controllers sections).
The admission webhook's hot path reads from the `Store` — O(1), no network I/O,
satisfying the p99 < 100 ms performance target.

**Alternatives considered**:
- `kube::runtime::Controller` (full reconcile loop): suitable for the
  controllers but heavier than needed for the webhook's read-only cache. The
  webhook uses a bare reflector (no reconcile loop — it reads, doesn't write).
- Raw `watch` API + manual `resourceVersion` management: rejected — the
  reflector already handles this correctly; manual management is error-prone.
- Polling (`Api::list` on a timer): rejected — generates excessive
  apiserver load and higher latency than watch-based reflectors.

---

## R6. Admission webhook HTTP server

**Decision**: `axum` 0.7+ on `hyper` 1.x, served over HTTPS via `rustls`.

**Rationale**: `axum` is the idiomatic async web framework in the Tokio
ecosystem, built directly on `hyper` and `tower`. It integrates natively with
`tokio::runtime`. The admission webhook endpoint is a single `POST /validate`
route that accepts an `AdmissionReview<CoreV1>` and returns an
`AdmissionReview` with a `response` field. `rustls` handles TLS termination
(Kubernetes requires HTTPS for admission webhooks).

`kube-rs` does not ship an admission server framework — it provides the types
(`AdmissionReview`, `AdmissionRequest`, `AdmissionResponse`) via
`k8s-openapi`, but the HTTP server is the application's responsibility. `axum`
is the minimal, well-supported choice.

**Alternatives considered**:
- `actix-web`: rejected — heavier, its own runtime, less idiomatic with
  `tower` middleware.
- Raw `hyper`: rejected — too low-level for a route handler; `axum` adds
  negligible overhead.
- `warp`: rejected — less actively maintained than `axum` in 2026.

---

## R7. Resource quantity parsing and arithmetic

**Decision**: Implement a `Quantity` parser that normalises Kubernetes resource
strings to integer units (milli-CPUs for CPU, bytes for memory) for exact
arithmetic. Use `k8s-openapi`'s `apimachinery::Quantity` type for
deserialisation, then convert to internal integer types.

**Rationale**: Kubernetes resource quantities (`"500m"` = 500 millicores,
`"2Gi"` = 2 × 2³⁰ bytes) must be summed and compared exactly. Floating-point
arithmetic would introduce drift at the budget boundary (Principle II requires
deterministic enforcement). The `k8s-openapi` `Quantity` type wraps the raw
string; we parse it into `i64` (bytes) and `i64` (milli-CPUs) for lossless
arithmetic.

CPU values in Kubernetes are always expressible as integer milli-CPUs (the
smallest unit is `1m`). Memory is always expressible as integer bytes. Both fit
comfortably in `i64` (max ~9.2 × 10¹⁸) even for exabyte-scale clusters.

**Alternatives considered**:
- Use `rust_decimal` or fixed-point: rejected — unnecessary; milli-CPU and
  bytes are already integer domains.
- Compare as strings: rejected — impossible to sum or compare magnitudes
  correctly across suffixes (`"1Gi"` vs `"1073741824"`).

---

## R8. Deployment topology: Deployment vs DaemonSet

**Decision**: `Deployment` (replica count 2 with leader-election via the
  Allocation CRD status update).

**Rationale**: The webhook is a cluster-level service, not a per-node agent. A
`Deployment` with 2 replicas provides redundancy. Both controllers and the
webhook can run in each replica, but only one should write CRD status at a time
to avoid write conflicts. Leader election can be implemented via a lease on the
Allocation CRD (or `coordination.k8s.io/Lease`).

For the webhook specifically: Kubernetes routes admission calls to the webhook
Service, which load-balances across pods. Both replicas serve admission
requests from their local reflector caches. The caches are eventually
consistent (reflector sync), so both replicas will converge to the same
allocation figures. In the brief window where they diverge (one has seen a pod
event the other hasn't), the worst case is a slightly different but still
valid budget check — both reject if over budget.

**Alternatives considered**:
- `DaemonSet`: rejected — the webhook is not a per-node component; it doesn't
  need to run on every node. DaemonSet would waste resources on large clusters.
- Single replica: rejected — no redundancy; a pod restart causes admission
  failures (fail-closed, so safe, but disruptive).

---

## R9. TLS certificate provisioning

**Decision**: Support both (a) cert-manager `Certificate` resource and (b) a
  manually-provided TLS Secret. Default deployment YAML references cert-manager;
  a fallback Secret path is documented for clusters without cert-manager.

**Rationale**: Kubernetes admission webhooks require HTTPS. The webhook's
`ValidatingWebhookConfiguration` must reference a CA bundle that trusts the
webhook's serving certificate. cert-manager is the de facto standard for
automated cert rotation in Kubernetes. Providing a manual Secret path supports
air-gapped or minimal clusters.

The webhook reads the cert and key from a mounted `Secret` (paths configurable
via flags: `--tls-cert-file`, `--tls-key-file`). The CA bundle in the
`ValidatingWebhookConfiguration` is injected by cert-manager's `ca-injector`
or provided manually in the webhook config YAML.

**Alternatives considered**:
- Generate self-signed certs at startup: rejected — the apiserver needs a
  stable CA bundle; self-signed certs that rotate break the trust chain.
- HTTP (no TLS): rejected — Kubernetes does not allow HTTP admission webhooks.

---

## R10. Metrics exposition

**Decision**: `prometheus` crate 0.14, exposition endpoint at `GET /metrics`
  on the same `axum` server (different port optional).

**Rationale**: The `prometheus` crate is the standard Rust Prometheus client
library. It provides `IntCounter`, `Histogram`, `IntGauge` types that are
thread-safe and integrate with the `tokio` runtime. The metrics endpoint is
scraped by Prometheus/Grafana — standard Kubernetes observability.

Metrics to expose (mapping to constitution Principle IV):
- `capacity_admission_verdicts_total{resource,verdict}` (counter: allow/deny/error)
- `capacity_admission_decision_duration_seconds` (histogram)
- `capacity_admission_capacity_freshness_seconds` (gauge: seconds since last
  CRD status update)
- `capacity_admission_allocation_ratio{resource}` (gauge: 0.0–1.0+,
  allocated/ceiling)
- `capacity_admission_total_allocatable{resource}` (gauge)
- `capacity_admission_current_allocation{resource}` (gauge)
- `capacity_admission_ceiling{resource}` (gauge)

**Alternatives considered**:
- `opentelemetry`: rejected — adds abstraction layer; Prometheus is the
  standard Kubernetes metrics backend and the constitution specifies Prometheus.

---

## R11. Integration test strategy: tower-test mocked apiserver

**Decision**: Use `tower-test` to mock the kube-apiserver as a
  `tower::Service<http::Request<Body>>`. The webhook's admission handler is
  tested by feeding scripted `AdmissionReview` requests and asserting the
  response. The reflector cache is pre-populated with test fixture state
  (ClusterCapacity + Allocation CRD objects) to simulate various cluster
  states.

**Rationale**: The constitution (Principle VI) mandates `tower-test` as the
default integration test path. `tower-test::mock::Mock` allows building a
scripted service that responds to HTTP requests with predetermined responses —
perfect for simulating the admission webhook's HTTP interface. The webhook
handler is a pure function of (request, cached_state) → response, so tests can
inject fixture state directly without a real apiserver.

BDD tests via `cucumber-rs` build on top of the same mocked infrastructure,
with `.feature` files providing Given/When/Then scenarios readable by
non-Rust reviewers.

**Alternatives considered**:
- `kube-rs` envtest: explicitly rejected by the constitution (Principle VI)
  — requires a Go toolchain to run `etcd` + `kube-apiserver` binaries.
- Real `kind`/`k3d` cluster for all integration tests: rejected — too slow for
  the default `cargo test` path; reserved for E2E (`#[ignore]`).

---

## R12. Fail-closed implementation details

**Decision**: The webhook handler wraps the entire decision logic in a
  fallible function returning `Result<AdmissionResponse, AdmissionError>`.
  The `?` operator propagates errors; the top-level handler catches any
  `Err` and converts it to an `AdmissionResponse` with `allowed: false` and a
  descriptive `message`. Additionally, a `catch_unwind` guard converts panics
  to rejections.

**Rationale**: Rust's `Result` type makes the error path explicit and
exhaustive. By having a single error-to-response conversion point, we guarantee
(Principle I) that no error path can accidentally admit. The `catch_unwind`
guard ensures even a panic (e.g., from a malformed quantity parser) results in
a rejection rather than a crashed handler thread (which would cause the
apiserver to see a connection failure and apply `failurePolicy: Fail`).

Stale-data detection: the webhook checks the `lastUpdated` timestamp on the
cached Allocation CRD status. If the data is older than a configurable
threshold (`--capacity-freshness-timeout`, default 30s), the admission is
rejected with reason "capacity data stale".

**Alternatives considered**:
- Return `allowed: true` on error with a warning: rejected — violates Principle
  I (NON-NEGOTIABLE).
- Use `Option<AdmissionResponse>`: rejected — loses the error message that
  Principle IV requires in every rejection.
