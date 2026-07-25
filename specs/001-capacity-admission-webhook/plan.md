# Implementation Plan: Capacity Admission Webhook

**Branch**: `spec/capacity-admission-webhook` | **Date**: 2026-07-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-capacity-admission-webhook/spec.md`

## Summary

A Kubernetes validating admission webhook that prevents cluster overcommit by
enforcing a configurable capacity budget (percentage of total allocatable CPU
and RAM) against declared pod resource requests. The system follows a
3-component operator architecture (constitution Principle V): a Node Capacity
Controller that aggregates cluster supply, an Allocation Controller that tracks
consumption against the budget, and an Admission Webhook that gates pod
creation/updates. Components are linked by two CRDs (`ClusterCapacity`,
`Allocation`) as shared state. The webhook reads cached CRD state (via
`kube-rs` reflectors) so the admission critical path makes no API-server calls
— it is an O(1) budget check against locally cached figures. Implementation is
in Rust using `kube-rs`, `axum`/`hyper`, `rustls`, `tokio`, and `tracing`.

## Technical Context

**Language/Version**: Rust stable edition 2024. MSRV 1.89 (minimum required by
`kube-rs` 4.x). Recorded as `rust-version` in `Cargo.toml`.

**Primary Dependencies**:
- `kube` 4.2.0 (features: `runtime`, `derive`, `client`, `rustls-tls`) —
  Kubernetes client, watcher/reflector/Controller runtime, CRD derive macro.
- `k8s-openapi` 0.28.0 (features: `latest`, `schemars`) — built-in Kubernetes
  types (Pod, Node, AdmissionReview, Quantity).
- `schemars` 1.0 — JSON Schema generation for CRD validation schemas (required
  by `kube` 4.x derive).
- `tokio` 1.x (features: `full`) — async runtime.
- `axum` + `hyper` 1.x — HTTP server for the admission webhook endpoint.
- `rustls` 0.23 — TLS termination for the webhook HTTPS endpoint.
- `serde` / `serde_json` 1.x — serialisation of AdmissionReview and CRD objects.
- `tracing` + `tracing-subscriber` 0.1/0.3 — structured logging.
- `prometheus` 0.14 — metrics exposition (admission verdicts, latency, capacity
  utilisation).

**Storage**: N/A — no database. State lives in Kubernetes CRDs
(`ClusterCapacity`, `Allocation`) managed by the controllers. The webhook
process holds an in-memory cache (reflector `Store`) of the CRD state for
hot-path reads.

**Testing**:
- Unit: standard `#[test]` — budget arithmetic, resource-quantity parsing,
  boundary conditions (at ceiling, one over, zero remaining).
- Integration: `tower-test` mocking the kube-apiserver as a `tower::Service`,
  feeding scripted AdmissionReview scenarios through the webhook.
- BDD: `cucumber-rs` with `.feature` files (Given/When/Then against a mocked
  apiserver `World`).
- E2E: `k3d`/`kind` cluster on CI, `#[ignore]` by default, across the N-2
  Kubernetes version matrix (1.34, 1.35, 1.36).

**Target Platform**: Linux container (`x86_64-unknown-linux-gnu`, static
musl build), deployed as a Kubernetes `Deployment` behind a `Service` (see
research.md for Deployment-vs-DaemonSet rationale).

**Project Type**: Web-service / Kubernetes operator (single binary, multiple
internal roles).

**Performance Goals**:
- Admission decision: p99 < 100 ms (excluding kube-apiserver overhead), p50 <
  50 ms. Achievable because the hot path reads from an in-process cache — no
  network calls, no I/O, pure arithmetic + serialisation.
- Webhook resource footprint: < 256 MiB memory request, < 500 m CPU request.

**Constraints**:
- Fail-closed: every non-verifiable path returns `allowed: false` (Principle I).
- Validating-only: the webhook never mutates the pod object.
- N-2 Kubernetes support: APIs must be stable (GA) across 1.34–1.36.
- Least-privilege RBAC: read on Nodes + Pods; write only on the two CRDs'
  status subresources.
- No secrets stored or logged.

**Scale/Scope**: Target clusters up to ~5,000 nodes / ~50,000 pods. Reflector
cache holds a snapshot of Node and Pod objects; the controllers aggregate to
two single-cluster CRD instances, so the admission decision is O(1) regardless
of cluster size.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | Webhook returns `allowed: false` on every error path: deserialisation failure, CRD cache miss/stale, timeout, internal panic catch. `failurePolicy: Fail` on the ValidatingWebhookConfiguration. Documented in contracts/admission-webhook.md §Error Paths. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | Budget is a percentage of `.status.allocatable`, enforced as a hard ceiling (allocation == ceiling allowed; allocation > ceiling rejected). Source of truth is the kube-apiserver (node `.status.allocatable` + pod requests). No soft limits, no annotations. |
| III | Explicit Failure Mode Configuration | ✅ PASS | Every error path maps to Reject or a documented exception. Enumerated in data-model.md §Admission Decision States and contracts/admission-webhook.md §Error Path Matrix. No undefined category. |
| IV | Observability Before Optimisation | ✅ PASS | Structured `tracing` logs on every decision; Prometheus metrics endpoint exposing verdicts, latency histogram, capacity freshness, utilisation. Rejection messages carry figures. All in v1 scope. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | 3-component architecture: Node Capacity Controller, Allocation Controller, Admission Webhook — linked by 2 CRDs. Each has one job. No mutating webhook, no caching layers beyond the standard reflector store, no per-node partitioning. |
| VI | Integration Test Coverage | ✅ PASS | Integration tests via `tower-test` (mocked apiserver); BDD via `cucumber-rs`. Covers happy-path admit, over-budget deny, and every failure path from Principle III. E2E on `k3d`/`kind` in CI. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | CI matrix tests against 1.34, 1.35, 1.36. All APIs used (ValidatingWebhookConfiguration v1, CRDs v1, Node, Pod) are GA across the window. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | Plan mandates Red-Green-Refactor for all implementation. tasks.md (Phase 2) will sequence every task as test-first. |
| IX | Editor Configuration as Code | ✅ PASS | `.editorconfig` at repo root governs mechanical formatting; `rustfmt`/`taplo`/`shfmt`/`prettier` are authoritative for their languages, `.editorconfig` mirrors them. |

**Post-design re-check**: see §Constitution Check (Post-Design) at the end of
this file.

## Project Structure

### Documentation (this feature)

```text
specs/001-capacity-admission-webhook/
├── plan.md              # This file
├── research.md          # Phase 0 output — resolved technical decisions
├── data-model.md        # Phase 1 output — entities, CRD schemas, state machines
├── quickstart.md        # Phase 1 output — validation/run guide
├── contracts/           # Phase 1 output — interface contracts
│   ├── admission-webhook.md  # AdmissionReview request/response contract
│   ├── clustercapacity-crd.md  # ClusterCapacity CRD spec
│   └── allocation-crd.md  # Allocation CRD spec
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
src/
├── main.rs                      # Entry point: starts controllers + webhook server
├── lib.rs                       # Re-exports for integration tests
├── crd/
│   ├── mod.rs                   # CRD type re-exports
│   ├── cluster_capacity.rs      # ClusterCapacity CRD (spec + status structs)
│   └── allocation.rs            # Allocation CRD (spec + status structs)
├── controllers/
│   ├── mod.rs
│   ├── node_capacity.rs         # Node Capacity Controller (watch nodes → CRD status)
│   └── allocation.rs            # Allocation Controller (watch pods + CC CRD → Alloc status)
├── webhook/
│   ├── mod.rs
│   ├── handler.rs               # axum handler: AdmissionReview in → AdmissionReview out
│   ├── admission.rs             # Core decision logic: extract requests, check budget
│   └── error.rs                 # Error types → fail-closed mapping
├── resources/
│   ├── mod.rs
│   └── quantity.rs              # Kubernetes resource.Quantity parsing & arithmetic
├── metrics.rs                   # Prometheus metric definitions + exposition
└── config.rs                    # CLI flags / env vars (port, cert paths, namespace)

deploy/
├── deployment.yaml              # Deployment + Service for the webhook
├── webhook-config.yaml          # ValidatingWebhookConfiguration
├── rbac.yaml                    # ClusterRole + Binding (read nodes/pods, write CRDs)
├── crds.yaml                    # ClusterCapacity + Allocation CRD manifests
└── cert-setup.yaml              # cert-manager Certificate or Secret reference

tests/
├── unit/                        # Standard #[test] modules (budget math, quantity parsing)
├── integration/                 # tower-test based (mocked apiserver AdmissionReview flow)
├── bdd/                         # cucumber-rs .feature files + step definitions
│   ├── features/
│   │   ├── budget_enforcement.feature
│   │   ├── capacity_awareness.feature
│   │   └── fail_safe.feature
│   └── steps/
└── e2e/                         # #[ignore] k3d/kind tests (CI only)

.editorconfig                    # Mechanical formatting rules (all file types)
Cargo.toml                       # Workspace manifest, dependencies, rust-version (MSRV)
rustfmt.toml                     # rustfmt configuration (canonical for *.rs)
```

**Structure Decision**: Single-binary operator (src/ layout, not a workspace).
All three components run in one process — `main.rs` spawns the two controllers
as `kube::runtime::Controller` tasks and the webhook as an `axum` server on the
same `tokio` runtime. The `tests/` directory mirrors the constitution's test
strategy: unit, integration (tower-test), BDD (cucumber-rs), and E2E (k3d/kind).

## Complexity Tracking

> No constitution violations requiring justification. The 3-component split is
> mandated by Principle V (NON-NEGOTIABLE), not a violation of it.

---

## Constitution Check (Post-Design)

*Re-evaluated after Phase 1 design artifacts (data-model.md, contracts/) were
produced.*

| # | Principle | Post-Design Status | Notes |
|---|-----------|-------------------|-------|
| I | Fail-Closed by Default | ✅ CONFIRMED | data-model.md §Admission Decision States enumerates every path to `Deny`. contracts/admission-webhook.md §Error Path Matrix confirms every error maps to `allowed: false`. The reflector cache miss (CRD not yet populated at startup) is handled as a stale-data rejection. |
| II | Capacity as a Hard Budget | ✅ CONFIRMED | data-model.md §Budget Calculation defines the ceiling as `floor(allocatable × percent)`, inclusive at equality. Resource quantities are normalised to integer milli-CPUs and bytes for exact comparison — no floating-point drift. |
| III | Explicit Failure Mode Configuration | ✅ CONFIRMED | All failure paths are enumerable from contracts/admission-webhook.md §Error Path Matrix: stale capacity, CRD missing, deserialisation failure, timeout, internal error → all Reject. No exceptions configured for v1. |
| IV | Observability Before Optimisation | ✅ CONFIRMED | data-model.md §Metrics defines the full metric set. contracts/admission-webhook.md §Logging defines the structured log fields. quickstart.md includes metric verification steps. |
| V | Separated Concerns, Minimal Surface | ✅ CONFIRMED | Three components with clear boundaries (Project Structure). CRDs carry controller-computed status only. No additional complexity beyond the mandated split. |
| VI | Integration Test Coverage | ✅ CONFIRMED | quickstart.md maps each user story to its test scenario. BDD `.feature` files cover all acceptance scenarios from the spec. |
| VII | N-2 Support Window | ✅ CONFIRMED | All APIs used are GA since Kubernetes 1.16 (ValidatingWebhookConfiguration v1) or earlier. No beta/alpha APIs. |
| VIII | Test-First Development | ✅ CONFIRMED | Project Structure includes dedicated test directories. tasks.md (Phase 2) will enforce the test-first ordering. |
| IX | Editor Configuration as Code | ✅ CONFIRMED | `.editorconfig` included in the source tree. `rustfmt.toml` alongside it. |

**Gate result**: PASS — no violations. Design advances to Phase 2 (tasks).
