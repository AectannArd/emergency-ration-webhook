<!--
=== Sync Impact Report ===
Version change: 1.0.0 → 1.2.0 (supersedes remote v1.0.0; incorporates prior
  amendments ratifying Principles VI and VII)
  Prior: (untracked template) → 1.0.0 (initial ratification, 2026-07-25)
  Prior: 1.0.0 → 1.1.0 (Principle VI added, 2026-07-25)
  Prior: 1.1.0 → 1.2.0 (Principle VII added, 2026-07-25)
Modified principles (vs remote v1.0.0):
  - II. Fail-Safe by Design → I. Fail-Closed by Default (NON-NEGOTIABLE).
    Remote v1.0.0 set a provisional fail-open default pending /speckit-clarify.
    The owner has now decided: fail-closed is the non-negotiable default.
    This is a MAJOR-in-spirit governance change (principle redefinition) but is
    folded into this consolidation version because the v1.0.0 default was
    explicitly marked provisional and never confirmed. Numbered 1.x, not 2.0.0,
    on that basis.
Added principles:
  - v1.0.0: Principles I–V (initial ratification; renamed/reordered here)
  - v1.1.0: VI — Integration Test Coverage of Main and Exceptional Workflows
  - v1.2.0: VII — Kubernetes Version Support Window (N-2)
Added sections (cumulative):
  - Core Principles I–VII
  - Technology Constraints
  - Development Workflow
  - Governance
Removed sections: none
Templates requiring updates:
  - .specify/templates/plan-template.md — ✅ no change needed (Constitution Check gate is generic; plan author derives gates from this file)
  - .specify/templates/spec-template.md — ✅ no change needed (no constitution-coupled sections)
  - .specify/templates/tasks-template.md — ✅ no change needed (no constitution-coupled sections)
Follow-up TODOs:
  - Port concrete SLO targets, dependency list (kube-rs/axum/hyper/rustls), and
    security model (TLS/RBAC) from the superseded v1.0.0 into a follow-up
    amendment (v1.3.0) so that detail is not lost.
=== Sync Impact Report End ===
-->

# Emergency Ration Webhook Constitution

## Core Principles

### I. Fail-Closed by Default (NON-NEGOTIABLE)

The webhook exists to prevent cluster overcommit. When it cannot authoritatively
verify that a workload fits within the configured capacity budget — for any
reason (webhook process down, metrics/capacity API unreachable, TLS failure,
timeout exceeded, deserialization error) — it MUST reject the admission request.

- `failurePolicy: Fail` is the only supported default for the ValidatingWebhookConfiguration.
- A denial is always a safe outcome; an admission under degraded knowledge is never safe.
- The admission response MUST set `allowed: false` on every non-verifiable path.
- Rationale: a capacity guardian that admits when it cannot measure has failed
  its only job. Cluster stability outranks deploy throughput.

### II. Capacity as a Hard Budget (NON-NEGOTIABLE)

CPU and RAM are tracked against a configurable percentage ceiling of cluster
capacity. Admission decisions are deterministic budget checks, not heuristics
or "best effort" estimates.

- The configured capacity percentage is the source of truth — there is no soft
  limit, no override by annotation, no per-workload exception in v1.
- Scheduled (not yet running) workloads MUST be counted against the budget so
  the webhook prevents overcommit before it happens, not after.
- The canonical source of capacity truth MUST be the Kubernetes API server
  (node `.status.allocatable` and pod resource requests/limits). The webhook
  MUST NOT rely on out-of-band or human-fed capacity data.
- Rationale: predictable, auditable admission is the product. Fuzzy limits
  defeat the purpose.

### III. Explicit Failure Mode Configuration

The failure mode is not emergent behaviour — it is declared, tested, and
documented. Every code path that could cause a non-verifiable admission MUST
map to one of:

1. Reject (the default, per Principle I), or
2. A narrow, explicitly-configured exception with a recorded justification.

There is no third "undefined" category. Unknown error types reject by default.

- Rationale: in a control-plane component, implicit/undocumented failure
  behaviour is a latent incident. The decision tree MUST be enumerable from the
  source and the tests.

### IV. Observability Before Optimisation

The webhook MUST emit structured logs and metrics sufficient to answer, for any
admission request: what was requested, what capacity was seen, what was
decided, and why. Capacity state changes and every rejection reason are
first-class observability events.

- Structured logging (`tracing`) MUST accompany every allow/deny with the
  decision, the triggering workload, and the capacity figures used.
- Prometheus metrics MUST be exposed: admission verdicts (allow/deny/error),
  decision latency histogram, cache freshness, and current capacity utilisation
  per resource type.
- Denials MUST carry a clear, human-readable `message` and, where applicable, a
  machine-readable `reason` on the AdmissionResponse.
- Metrics and structured logging are required for the v1 admission path; they
  are not a "polish phase" task.
- Rationale: a capacity controller that cannot explain its own decisions cannot
  be trusted in production or debugged during an incident.

### V. Simplicity and YAGNI

Start with the minimum that enforces the budget correctly: a single
ValidatingWebhook, synchronous capacity check, one config source. Add
complexity (mutating webhooks, multiple budgets, caching layers, custom
exceptions) only when a concrete, evidence-backed need appears.

- One resource-accounting model, one enforcement policy, one webhook type
  (Validating) unless a requirement forces otherwise.
- Configuration via ConfigMap and/or flags; no database, no CRDs for v1.
- Prefer standard Kubernetes types and stable APIs over alpha/custom resources
  unless the stable surface is provably insufficient.
- Complexity MUST be justified in the plan's Complexity Tracking table.
- Rationale: admission webhooks sit on the critical path of every deploy;
  unnecessary surface area is unnecessary risk.

### VI. Integration Test Coverage of Main and Exceptional Workflows

The webhook's main (happy-path) workflow AND its exceptional (error/edge)
workflows MUST be covered by integration tests — not only unit tests of the
decision logic.

- Main workflow: a valid admission request that fits within the capacity budget
  is admitted, with capacity state observed end-to-end through the real
  admission path (AdmissionReview in → response out).
- Exceptional workflows: every enumerated failure path from Principle III
  (over-budget rejection, capacity source unreachable, timeout, malformed
  request) MUST have a corresponding integration test asserting the reject /
  fail-closed outcome.
- Integration tests exercise the webhook against a realistic admission request
  flow (fake/in-memory API surface, and where feasible a local control plane
  such as `kwok`/`kind`), not isolated function calls.
- Rationale: unit tests prove the budget arithmetic; integration tests prove the
  webhook actually rejects/admits when wired into the admission path. A
  fail-closed guardian that only passes unit tests is unverified at the
  boundary that matters.

### VII. Kubernetes Version Support Window (N-2)

The webhook MUST support the three most recent major Kubernetes releases (the
current release plus the two preceding — i.e. N, N-1, N-2).

- The ValidatingWebhookConfiguration and all Kubernetes API types used MUST be
  available and stable across the supported window. Prefer APIs that are GA/stable
  in the oldest supported release.
- Deprecation of support for an older release MUST be a documented, deliberate
  decision (tracked as a constitution-relevant change), not drift.
- As Kubernetes releases roughly three minor versions per year, the window is
  effectively the current plus ~8 months of prior history; the webhook's CI MUST
  test against each release in the window.
- Rationale: cluster operators cannot always upgrade immediately, and an
  admission webhook that only runs on the latest version is a forced upgrade
  dependency. N-2 is the standard community support window.

## Technology Constraints

- **Language**: Rust (current stable edition; MSRV recorded in `Cargo.toml`).
  The webhook and its capacity-tracking logic are implemented in Rust for
  latency, memory footprint, and correctness on the admission critical path.
- **Runtime target**: Linux container, deployed as a Kubernetes workload
  (`Deployment` behind a `Service`, served over HTTPS; DaemonSet is an
  alternative to be settled in the plan).
- **Kubernetes surface**: ValidatingWebhookConfiguration (v1). No MutatingWebhook
  in scope for v1.
- **Capacity inputs**: cluster node capacity (`.status.allocatable`) and
  resource requests/limits from the Kubernetes API. Source of capacity *usage*
  (live metrics vs. declared requests) to be resolved in clarification/plan.
- **Configuration**: the capacity percentage ceiling and webhook settings MUST
  be configurable via standard Kubernetes mechanisms (ConfigMap / flags / env),
  not compiled in.
- **No host paths or machine-specific paths in tracked files.** The repository
  is portable across the dev setup.

## Development Workflow

- **Spec-driven**: features are specified (`/speckit-specify`) and planned
  (`/speckit-plan`) before implementation. Implementation MUST cite the plan.
- **Dual-agent split**: planning (constitution, clarify, specify, plan) happens
  on the planning host; implementation (tasks, implement, test) is delegated to
  the coding agent on the build host. The git repository is the sync mechanism —
  planning commits are pulled before implementation begins.
- **Tests required**: the admission decision logic MUST have unit tests covering
  admit, reject, and every enumerated failure-mode path from Principle III
  before it is considered done. Capacity budget arithmetic MUST be tested at
  boundaries (exactly at ceiling, one unit over, zero remaining).
- **Quality gate**: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo
  test` (unit + integration) all green before merge. No admission-core change
  lands without a covering test.
- **Verification gate**: a feature is not complete until its tests pass against
  the real code path, not a stub.

## Governance

- This constitution supersedes all other project practices when they conflict.
- Amendments require: (a) a documented change with rationale, (b) a version bump
  following semantic versioning (MAJOR for principle removal/redefinition,
  MINOR for a new principle or material expansion, PATCH for clarification),
  (c) propagation through the dependent spec/plan/tasks templates, and (d) a
  commit recording the ratification date.
- Every spec's Constitution Check gate MUST be evaluated against this file
  before the plan advances past design.
- Use `.specify/memory/constitution.md` as the single source of truth for these
  principles; if a doc disagrees, the constitution wins.

**Version**: 1.2.0 | **Ratified**: 2026-07-25 | **Last Amended**: 2026-07-25
