<!--
Sync Impact Report
==================
Version change: (unratified template) → 1.0.0
  - Rationale: initial ratification. First adoption of governance is treated as
    a MAJOR (1.0.0) release per SemVer-for-governance.
Modified principles: all PRINCIPLE_N placeholders → concrete principles below.
  - I. Safety & Correctness First
  - II. Fail-Safe by Design
  - III. Stateless, Fast, Horizontally Scalable
  - IV. Observability
  - V. Test Discipline (NON-NEGOTIABLE)
  - VI. Simplicity (YAGNI)
Added sections:
  - Technology Stack & Constraints
  - Development Workflow & Quality Gates
  - Governance
Removed sections: none (template placeholders resolved).
Templates requiring updates:
  - .specify/templates/plan-template.md        — ✅ no change (generic; "Constitution Check" gate is filled per-feature from this file)
  - .specify/templates/spec-template.md        — ✅ no change (generic)
  - .specify/templates/tasks-template.md       — ✅ no change (generic)
Follow-up TODOs:
  - The default failure mode (fail-open vs fail-closed) under Principle II is an
    opinionated first stance; confirm or override during /speckit-clarify.
  - Exact latency budget numbers under Principle III are provisional targets;
    ratify concrete SLOs during /speckit-plan.
-->

# emergency-ration-webhook Constitution

A Kubernetes admission webhook that tracks cluster capacity (CPU and RAM) and
enforces that scheduled workloads do not exceed a configurable capacity
percentage, preserving headroom ("emergency ration") for surges and failures.

## Core Principles

### I. Safety & Correctness First

The webhook is critical cluster control-plane infrastructure: a wrong admission
decision can either break scheduling for the whole cluster or silently defeat
the capacity guarantee the webhook exists to provide.

- Admission decisions MUST be deterministic: the same (cluster state, pod
  request) pair MUST always yield the same allow/deny verdict.
- The canonical source of capacity truth MUST be the Kubernetes API server
  (node `.status.allocatable` and pod resource requests/limits). The webhook
  MUST NOT rely on out-of-band or human-fed capacity data.
- Capacity accounting MUST be conservative: when resource requests are absent
  (e.g. limits-only containers), a documented fallback MUST be applied and MUST
  not silently under-count.

### II. Fail-Safe by Design

A webhook in the admission path can fail (crash, timeout, TLS error). Its
failure mode is a first-class, documented, configurable decision — never an
accident.

- The webhook's `failurePolicy` (fail-open = `Ignore` to preserve cluster
  availability, fail-closed = `Fail` to preserve the capacity guarantee) MUST be
  explicit in every deployment manifest and MUST be documented.
- **Provisional default: fail-open** (`failurePolicy: Ignore`, `timeoutSeconds:
  3`). Rationale: a broken admission webhook MUST NOT wedge cluster-wide pod
  creation; availability trumps the capacity guarantee when the webhook itself
  is the thing that is broken. Operators who need a hard capacity ceiling set
  fail-closed deliberately. *(Confirm during /speckit-clarify.)*
- The webhook MUST degrade gracefully when it cannot reach the API server
  (cache-stale behaviour documented; never panic the request path).

### III. Stateless, Fast, Horizontally Scalable

The webhook sits in the hot path of pod admission. Performance is a feature and
a reliability concern.

- The webhook MUST be horizontally scalable: any replica MUST be able to answer
  any AdmissionReview. No sticky state, no leader-elected admission authority.
- The webhook MUST NOT block admission on a synchronous full-cluster API list.
  Capacity state MUST come from a local informer/watch cache refreshed in the
  background; admission latency MUST stay within a strict budget.
- Provisional SLO: p99 admission decision < 100 ms excluding kube-apiserver
  overhead, < 50 ms p50. *(Ratify concrete SLOs during /speckit-plan.)*
- Resource footprint MUST be modest (target < 256 Mi request, < 500 m CPU) so
  the guard does not itself consume the ration it protects.

### IV. Observability

Every admission decision MUST be explainable after the fact. Capacity
enforcement that cannot be inspected will not be trusted by operators.

- Structured logging (`tracing`) MUST accompany every allow/deny with the
  decision, the triggering workload, and the capacity figures used.
- Prometheus metrics MUST be exposed: admission verdicts (allow/deny/error),
  decision latency histogram, cache freshness, and current capacity utilisation
  per resource type.
- Denials MUST carry a clear, human-readable `message` and, where applicable, a
  machine-readable `reason` on the AdmissionResponse.

### V. Test Discipline (NON-NEGOTIABLE)

Admission logic is pure given (cluster state, AdmissionReview). That purity is
exploited ruthlessly.

- TDD is mandatory: tests written → reviewed → fail → then implement.
  Red-Green-Refactor is strictly enforced for the admission core.
- The admission decision function MUST be unit-testable in isolation, taking
  capacity state and a parsed request as plain values, returning a verdict.
- Property-based tests MUST cover the core invariant: total admitted requests
  never exceeds the configured capacity percentage when the policy denies.
- Integration tests MUST exercise the webhook against a fake/in-memory API
  surface (and, where feasible, a local control plane such as `kwok`/`kind`).

### VI. Simplicity (YAGNI)

Start with the minimal enforcement that delivers the guarantee. Every addition
(mutation, multi-resource, namespace scoping, priority classes) MUST justify
itself against the core invariant.

- One resource-accounting model, one enforcement policy, one webhook type
  (Validating) unless a requirement forces otherwise.
- Configuration via ConfigMap and/or flags; no database, no CRDs for v1.
- Complexity MUST be justified in the plan's Complexity Tracking table.

## Technology Stack & Constraints

- **Language**: Rust (current stable edition; MSRV recorded in `Cargo.toml`).
- **Role**: HTTP admission server returning Kubernetes `AdmissionReview`
  responses; a Validating webhook for v1.
- **Primary dependencies**: an async runtime (`tokio`), an HTTP/TLS server
  (`axum`/`hyper` + `rustls`), a Kubernetes client/informer (`kube-rs`),
  `serde` for (de)serialising admission objects, `tracing` for logs,
  a Prometheus metrics crate.
- **Deployment**: container image, `Deployment` behind a `Service`, served over
  HTTPS (kube-apiserver → webhook is TLS); a `ValidatingWebhookConfiguration`
  scoped to the resources/namespaces being protected.
- **Security**: TLS for the webhook endpoint (cert mounted from a Secret or
  issued by cert-manager); least-privilege RBAC for the service account (read on
  nodes + pods; no writes). No secrets stored or logged by the webhook.
- **Configuration**: capacity percentage(s), resource scopes, and fail-mode via
  ConfigMap/flags; hot-reload is a v2 concern.

## Development Workflow & Quality Gates

- Spec-Kit workflow is authoritative: constitution → (clarify) → specify → plan
  → tasks → implement, with review gates between phases.
- Two-agent split: planning (Hermes) and implementation (Claude Code), synced
  via `git`. `AGENTS.md` and `CLAUDE.md` are kept in sync by the
  `agent-context` extension and MUST NOT be hand-edited inside their SPECKIT
  markers.
- Quality gate before merge: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test` (unit + integration) all green. No admission-core change lands
  without a covering test.
- Every plan MUST pass a Constitution Check (see plan template) before research
  proceeds; re-check after design.

## Governance

- This constitution is the highest-authority document in the repo. Where any
  spec, plan, or implementation choice conflicts with it, the constitution wins
  unless the conflict is resolved via a documented amendment.
- Amendments require: (a) a written proposal stating the change and rationale,
  (b) a version bump per SemVer — MAJOR for principle removal/redefinition,
  MINOR for added principles/sections, PATCH for clarification — and (c) an
  updated `Last Amended` date.
- All code review and planning MUST verify constitution compliance; unjustified
  complexity MUST be rejected at the plan's Complexity Tracking gate.
- Use `.specify/memory/constitution.md` (this file) as the single source of
  governance; do not duplicate its principles elsewhere.

**Version**: 1.0.0 | **Ratified**: 2026-07-25 | **Last Amended**: 2026-07-25
