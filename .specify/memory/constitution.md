<!--
=== Sync Impact Report ===
Version change: 2.8.0 → 2.9.0 (MINOR — Principle X redefined: README as Documentation Hub with docs/ Articles)
  Prior: 2.7.0 → 2.8.0 (MINOR — Principle XV added: Build and Publish Procedure for Every Docker Artifact)
  Prior: 2.6.0 → 2.7.0 (MINOR — Principle XIV added: Artifact Inventory)
  Prior: 2.5.0 → 2.6.0 (MINOR — Principle XIII added: Separation of Usage and Contribution Documentation)
  Prior: 2.4.0 → 2.5.0 (MINOR — Principle XII added: Scratch Space for Agent Intercommunication)
  Prior: 2.3.0 → 2.4.0 (MINOR — Principle XI added: CI-Green Completion Gate)
  Prior: 2.2.0 → 2.3.0 (MINOR — Principle X added: User-Facing Functionality Documented in README.md)
  Prior: 2.1.0 → 2.2.0 (MINOR — Development Workflow expanded: branch-and-PR workflow rule)
  Prior: 2.0.0 → 2.1.0 (MINOR — Principle IX added: Editor Configuration as Code)
  Prior: 1.4.0 → 2.0.0 (MAJOR — Principle V redefined: single-webhook → 3-component operator, 2026-07-25)
  Prior: (untracked template) → 1.0.0 (initial ratification, 2026-07-25)
  Prior: 1.0.0 → 1.1.0 (Principle VI added, 2026-07-25)
  Prior: 1.1.0 → 1.2.0 (Principle VII added, 2026-07-25)
  Prior: 1.2.0 → 1.3.0 (integration test framework locked + deferred detail ported)
  Prior: 1.3.0 → 1.4.0 (Principle VIII: Test-First Development)
Modified principles (vs remote v1.0.0):
  - II. Fail-Safe by Design → I. Fail-Closed by Default.
  - V. Simplicity and YAGNI → V. Separated Concerns, Minimal Surface.
    The single-webhook model is replaced by a 3-component operator architecture
    (Node Capacity Controller + Allocation Controller + Admission Webhook),
    linked by CRDs as shared state. The 'no CRDs for v1' constraint is lifted.
    Rationale: node lifecycle and pod lifecycle are independent processes with
    different risk profiles; conflating them couples what should be separated.
    This is a MAJOR bump because Principle V was a core founding principle and its core
    stance (single component, no CRDs) is redefined.
Added principles:
  - v1.0.0: Principles I–IV (initial ratification; renamed/reordered here)
  - v1.1.0: VI — Integration Test Coverage of Main and Exceptional Workflows
  - v1.2.0: VII — Kubernetes Version Support Window (N-2)
  - v1.4.0: VIII — Test-First Development
  - v2.1.0: IX — Editor Configuration as Code
  - v2.3.0: X — User-Facing Functionality Documented in README.md
  - v2.4.0: XI — CI-Green Completion Gate
  - v2.5.0: XII — Scratch Space for Agent Intercommunication
  - v2.6.0: XIII — Separation of Usage and Contribution Documentation
  - v2.7.0: XIV — Artifact Inventory
  - v2.8.0: XV — Build and Publish Procedure for Every Docker Artifact
Modified in v1.3.0:
  - Principle VI: integration test framework selection locked.
  - Technology Constraints: added Primary Dependencies, Testing, SLO targets,
    Security/RBAC model.
Modified in v2.0.0:
  - Principle V: rewritten — 3-component operator architecture, CRDs allowed.
  - Technology Constraints: Kubernetes surface now includes CRDs; Configuration
    via CRD spec; Capacity inputs sourced via controllers.
  - Principle VI: envtest rejection rationale updated (was 'Principle V grounds',
    now just 'Go toolchain cost').
Modified in v2.9.0:
  - Principle X: rewritten — README as Documentation Hub with docs/ Articles.
    The README is now the navigational hub (project description, quick-start, TOC,
    and brief per-capability summaries that link to docs/). Detailed configuration,
    deployment, operational, and architecture reference lives in separate
    docs/ articles. The previous model (README MUST "itself cover the essentials"
    with docs/ only for "deeper reference material") is replaced — the README
    summarizes and links; docs/ holds the depth. MINOR bump because the principle's
    mandate is materially expanded (new structural split requirement), not removed.
  - Principle XIII: README bullet updated to reflect the hub model (quickstart +
    TOC + summaries linking to docs/, detailed reference in docs/ per Principle X).
  - Development Workflow: documentation-as-deliverable bullet updated —
    "discoverable from README.md, with detailed reference in docs/ articles."
Modified in v2.1.0:
  - Development Workflow: quality gate now requires .editorconfig compliance.
Modified in v2.2.0:
  - Development Workflow: added branch-and-PR rule — every spec implemented on a
    dedicated feature branch, merged to `main` only via pull request.
Modified in v2.3.0:
  - Development Workflow: added documentation-as-deliverable rule — user-facing
    changes MUST update README.md in the same change.
Modified in v2.4.0:
  - Development Workflow: quality gate and verification gate now require CI
    green (all jobs) before a task/feature is declared complete or a PR is
    merged. A failing CI run is an incomplete deliverable, regardless of
    whether the failure is in the changed code or pre-existing
    infrastructure.
Added sections (cumulative):
  - Core Principles I–XIV
  - Core Principles I–XV
  - Technology Constraints
  - Development Workflow
  - Governance
Removed sections: none
Templates requiring updates:
  - .specify/templates/plan-template.md — ✅ no change needed
  - .specify/templates/spec-template.md — ✅ no change needed
  - .specify/templates/tasks-template.md — ✅ no change needed
Follow-up TODOs: none.
=== Sync Impact Report End ===
-->

# Emergency Ration Webhook Constitution

## Core Principles

### I. Fail-Closed by Default

The webhook exists to prevent cluster overcommit. When it cannot authoritatively
verify that a workload fits within the configured capacity budget — for any
reason (webhook process down, metrics/capacity API unreachable, TLS failure,
timeout exceeded, deserialization error) — it MUST reject the admission request.

- `failurePolicy: Fail` is the only supported default for the ValidatingWebhookConfiguration.
- A denial is always a safe outcome; an admission under degraded knowledge is never safe.
- The admission response MUST set `allowed: false` on every non-verifiable path.
- Rationale: a capacity guardian that admits when it cannot measure has failed
  its only job. Cluster stability outranks deploy throughput.

### II. Capacity as a Hard Budget

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

### V. Separated Concerns, Minimal Surface

The capacity guardian separates two independent cluster processes — node
lifecycle (capacity supply) and pod lifecycle (capacity consumption) — into
distinct components linked by CRDs as shared state. Each component has a single
responsibility; complexity is only added where it separates a real concern.

- **Three components, each with one job:**
  1. **Node Capacity Controller** — watches nodes, publishes cumulative cluster
     capacity (sum of `.status.allocatable`) in a CRD `status`. Read-only on
     nodes; never interrupts the node lifecycle (draining is an operator
     decision, not the webhook's).
  2. **Allocation Controller** — watches the Node Capacity CRD + pod resource
     requests, computes current allocation percentage (in CRD `status`), holds
     the target allocation threshold (in CRD `spec`). Tracks pod
     CREATE + UPDATE + DELETE to keep allocation accurate.
  3. **Admission Webhook** — reads the Allocation CRD (`spec` threshold +
     `status` allocation) to admit/deny new pods against remaining budget.
     Tracks pod CREATE + UPDATE.
- **CRDs are the data link between components**, not a database — they carry
  controller-computed status, not user-facing CRUD.
- **Within each component, apply YAGNI ruthlessly**: one resource-accounting
  model, one enforcement policy, one webhook type (Validating). Configuration
  via the Allocation CRD `spec` and/or flags; no external database.
- Prefer standard Kubernetes types and stable APIs over alpha/custom resources
  unless the stable surface is provably insufficient.
- Complexity beyond this 3-component split (mutating webhooks, multiple budgets,
  caching layers, per-node partitioning) MUST be justified in the plan's
  Complexity Tracking table.
- Rationale: conflating node lifecycle and pod lifecycle in one component
  couples two processes with different risk profiles. Separating them via CRDs
  makes each independently testable (Principle VI) and independently failureable
  (Principle I), while the minimal-surface discipline prevents scope creep within
  each component.

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
  flow. The default integration test path uses `tower-test` to mock the
  kube-apiserver as a `tower::Service`, feeding scripted AdmissionReview
  request/response scenarios through the webhook. This avoids a Go toolchain
  dependency (Go toolchain cost) while keeping
  tests fast and isolated. E2E coverage on CI uses a `k3d`/`kind` cluster.
- BDD structure: integration tests SHOULD be organised as Gherkin `.feature`
  files executed via `cucumber-rs` (Given/When/Then against a mocked apiserver
  `World`), so failure paths are readable by non-Rust reviewers.
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

### VIII. Test-First Development

Development is test-first (TDD), not merely test-required. Tests are written
BEFORE implementation and watched to fail; only then is the minimal code
written to pass them. Red-Green-Refactor is strictly enforced.

- **RED**: write one minimal test describing the next behaviour. Run it and
  WATCH it fail — for the right reason (feature missing), not a typo or compile
  error. A test that passes immediately tests nothing.
- **GREEN**: write the minimal code to pass the test. Nothing more — no extra
  features, no refactors, no "improvements." Hardcoded returns and duplication
  are acceptable here.
- **REFACTOR**: only after green, clean up — remove duplication, improve names,
  simplify — while keeping tests green.
- **Iron Law**: no production code without a failing test first. Code written
  before its test MUST be deleted and reimplemented from the test, not
  "adapted" or kept "as reference."
- **Vertical slices, not horizontal**: one RED→GREEN→REFACTOR cycle per
  behaviour, end-to-end. Do NOT write a pile of tests then a pile of
  implementation — tests designed before the implementation teaches the
  interface become brittle.
- This applies to integration tests (Principle VI) too: the integration test
  for a workflow is written first and watched to fail, then the workflow is
  implemented to pass it.
- Rationale: tests written after code pass immediately and prove nothing — they
  are biased by the implementation and miss the edge cases you forgot. Seeing
  the test fail is the only proof it actually tests something.

### IX. Editor Configuration as Code

Mechanical file-formatting rules — indentation, line endings, final newline,
trailing whitespace, character encoding — are declared once in a versioned
`.editorconfig` at the repository root and enforced by editor tooling and CI,
not by ad-hoc convention or review nitpicks.

- The `.editorconfig` is the single source of truth for mechanical formatting
  across every file type in the repo (Rust, TOML, YAML, JSON, Markdown, shell,
  PowerShell, Python, Gherkin `.feature`, Dockerfile, Makefile).
- Where a language has its own canonical formatter (`rustfmt`, `taplo`, `shfmt`,
  `prettier`), that formatter is AUTHORITATIVE and the `.editorconfig` MUST
  mirror it — they are not independent sources. `rustfmt` governs `*.rs`; the
  `.editorconfig` governs everything `rustfmt` does not reach.
- Adding a new file type to the repo MUST add (or confirm) a matching section in
  `.editorconfig` in the same change. An unconfigured file type is a formatting
  debt.
- Formatting changes are real changes: they require a commit and the same review
  as any other. "My editor did it" is not a valid unexplained diff, and silent
  reformatting of unrelated lines in a functional commit is review-blocking.
- Rationale: formatting churn from inconsistent editor settings pollutes diffs,
  obscures real changes in `git blame`, and wastes review cycles on noise.
  Declaring the rules in a machine-readable, editor-agnostic file removes the
  entire class of disagreement at the source.

### X. README as Documentation Hub with docs/ Articles

Every user-facing capability of the webhook — installation, configuration,
deployment, and operational behaviour — MUST be documented and discoverable from
the repository's `README.md`. The README is the navigational hub and entry point;
detailed reference material lives in separate articles under `docs/`.

**README.md scope (the hub):**
- A project description: what the software is and does.
- A quick-start section: the minimal path from clone to a running, verified
  deployment (install, deploy, verify — concise, not exhaustive).
- A table of contents linking to every article in `docs/`.
- For each major capability, a brief (1–3 sentence) summary that links to its
  detailed article in `docs/`.

**docs/ scope (the detail):**
- Full configuration reference (CLI flags, environment variables, CRD
  spec/status fields with examples).
- Deployment manifests and procedures (TLS provisioning, RBAC, multi-component
  topology, multi-cluster setup).
- Operational reference (metrics catalog, structured-log keys, failure-mode
  catalog, enforcement modes, workload exclusion, budget overrides).
- Architecture and component design.
- Tooling reference (verification CLI usage, scenario inventory).
- Any capability whose documentation exceeds a few paragraphs belongs in
  `docs/`, not in the README.

**The split is structural, not cosmetic.** When the README grows to the point
that a reader must scroll past unrelated sections to reach the one they need, the
overflowing section MUST be extracted into a `docs/` article and replaced with a
summary + link. A monolithic README that mixes quick-start with deep reference
serves neither the first-time operator (overwhelmed) nor the experienced operator
(hunting through walls of text).

**Same-change rule (unchanged):** a change that adds or alters user-facing
functionality MUST update documentation in the same change (same commit / PR).
For a new capability this means: create the `docs/` article AND add a TOC entry
+ summary in README. For a changed capability: update the `docs/` article AND the
README summary if the one-liner changed. A PR that ships user-facing behaviour
without the matching doc delta is incomplete and MUST be blocked at review — the
same standard as the test-first rule (Principle VIII) and the `.editorconfig`
rule (Principle IX).

**README must not duplicate docs/ content verbatim.** It summarizes and links,
not copies. When a detail appears in both places, the `docs/` article is the
source of truth and the README summary defers to it.

**Discoverability guarantee:** a human operator MUST be able to discover, from
the README alone, that a capability exists and where to read about it in depth.
A capability documented only in `docs/` without a README link is undocumented.
Conversely, a brief or trivial capability MAY live entirely in the README without
a `docs/` article — the split is triggered by depth, not by mandate on every
topic.

- Rationale: as the webhook grew from a single admission webhook to a
  multi-component operator with per-resource budgets, workload exclusion,
  enforcement modes, an on-demand verification CLI, and a cross-cluster equalizer,
  the README grew to over a thousand lines. A README of that length is no longer
  scannable — operators cannot find what they need, and contributors hesitate to
  add content to an already-heavy file. Making the README a navigational hub with
  detail in `docs/` keeps each audience served: first-time operators get a short
  quick-start, experienced operators link directly to the reference they need,
  and contributors add new capabilities as focused articles without bloating the
  hub.

### XI. CI-Green Completion Gate

A task, feature, or spec is not complete until the continuous integration
pipeline passes on the branch that will be merged. Implementation work whose CI
is failing — for any reason — is an incomplete deliverable, not a finished one.

- "CI passes" means **all** CI jobs on the pull request are green: not just the
  Rust quality gate (`fmt`, `clippy`, `test`), but also E2E, `.editorconfig`
  compliance, and any other check the repository runs. A single failing job
  fails the gate; there is no "unrelated failure" exemption.
- A pre-existing infrastructure failure (e.g. a CI bug in `main`) does NOT
  exempt a change from this gate. If CI is red on `main`, the first obligation
  is to fix the infrastructure — then the change can be validated and merged.
  Shipping a change on top of broken CI is shipping unverified work.
- A task list is not fully checked off while any task's covering CI is red.
  The implementation agent MUST report CI failures and either fix them or
  escalate them — not declare success and leave them for the reviewer to find.
- The pull request is not mergeable until CI is green. Branch-protection
  required-status-checks SHOULD enforce this automatically; where they are
  not configured, the reviewer MUST treat a red PR as blocked, not mergeable.
- This principle is the verification analogue of Principle VIII (test-first):
  VIII requires a failing test before the code; XI requires a passing pipeline
  after the code. Code without a passing CI gate is unverified — the same
  standard the webhook applies to cluster capacity (Principle I).
- Rationale: a capacity admission webhook is a safety-critical control-plane
  component. A PR whose CI is red is a PR whose correctness has not been
  demonstrated in the environment that matters. "It works on my machine" or
  "the failure is pre-existing" are not sufficient — the pipeline is the
  evidence, and red evidence is no evidence. Declaring a task complete while
  CI fails teaches the team to tolerate unverified work, which is the exact
  anti-pattern the webhook exists to prevent at the cluster level.

### XII. Scratch Space for Agent Intercommunication

The repository MUST provide a single git-ignored scratch directory (`.temp/`)
for transient artifacts that are produced during a task, consumed within the
same task or by the downstream agent, and deleted or abandoned afterward.
Scratch files are never committed and never shipped.

- **Purpose**: agents and scripts produce intermediate output that exists to be
  read by the next step, not to be preserved — validation reports, test-run
  logs, rendered manifests, extracted snippets, checkpoint dumps, captured
  command output. These belong in `.temp/`, NOT in the repository root or any
  tracked directory.
- **Never tracked**: `.temp/` is git-ignored. Files inside it are ephemeral by
  definition — if an artifact needs to survive a task, it must be promoted to a
  tracked location (specs, README, source) with an explicit justification.
- **Agent intercommunication**: when the planning agent and the implementation
  agent (or any two automated steps) need to exchange a file (e.g. a
  pre-rendered plan excerpt, a verification checklist), they write it to
  `.temp/` and reference it by relative path. The receiver reads it from there.
  This replaces the anti-pattern of writing such files to the repo root (which
  risks accidental commits like `VALIDATION.md`).
- **Naming**: no convention enforced — the writer picks a descriptive filename.
  Collision avoidance is the writer's responsibility; overwriting is acceptable.
- **No cleanup obligation**: files MAY linger in `.temp/` across sessions; they
  are disposable. `.temp/` being non-empty is not a defect.
- **Rationale**: a tracked file in the repository root that was intended as a
  one-time validation report (`VALIDATION.md`) revealed the gap: agents need a
  write target for transient files, and absent a designated scratch space they
  default to the repo root, where the files get committed and pollute history.
  `.temp/` closes that gap by making the scratch space explicit, git-ignored,
  and discoverable to every agent that touches the repo.

### XIII. Separation of Usage and Contribution Documentation

The repository maintains two distinct documentation surfaces with
non-overlapping scope: `README.md` for **usage** (what the software does and
how to operate it) and `CONTRIBUTING.md` for **contribution** (how to work on
the repository — building, testing, running real-infrastructure verification,
and development workflow). A change to one surface MUST NOT silently absorb
the other's content.

- **`README.md`** — the operator's entry point. Contains: project description,
  quick-start, a table of contents linking to every `docs/` article, and brief
  per-capability summaries that link to `docs/` for detail. This is Principle X's
  scope, updated for the hub model.
- **`CONTRIBUTING.md`** — the contributor's entry point. Contains: how to clone
  and set up the development environment, how to build (including container
  images), how to run the test suite (unit, integration, BDD, E2E), how to run
  the on-demand verification tool (`erw-verify`) against a real cluster,
  development workflow (spec-driven, dual-agent split, branch-and-PR), code
  style and formatting rules, and how to submit changes.
- **Non-overlap rule**: if a piece of documentation is about *using* the
  deployed software (an operator reading it to run/configure the webhook), it
  belongs in `README.md`. If it is about *working on the repository* (a
  developer reading it to build, test, or contribute), it belongs in
  `CONTRIBUTING.md`. The README MAY link to `CONTRIBUTING.md` for contributor
  concerns, but MUST NOT duplicate contributor content inline.
- **Mandatory `CONTRIBUTING.md`**: every repository MUST have a
  `CONTRIBUTING.md` at the root. A change that adds or alters contribution
  workflow (build steps, test commands, verification tooling, development
  process) MUST update `CONTRIBUTING.md` in the same change — the same
  standard as the README rule (Principle X) and the test-first rule (Principle
  VIII).
- Rationale: mixing usage and contribution docs in a single README creates a
  wall of text that serves neither audience — operators wade through build
  commands they don't need, while contributors hunt for test instructions
  buried after deployment sections. Separating the surfaces makes each
  document focused and scannable. This became necessary when the spec-009
  image-build automation added substantial build/test/verify documentation that
  belongs in the contributor's guide, not in the operator's README.

### XIV. Artifact Inventory

Every binary the repository produces MUST be enumerated in a dedicated
`ARTIFACTS.md` file at the repository root, and each entry MUST explicitly
state whether the binary is published as a Docker image or not.

- **Scope**: a "binary" is any `[[bin]]` target declared in `Cargo.toml`
  (or equivalent build manifest for non-Rust components). The library crate
  (`[lib]`) is not a binary and is out of scope.
- **Per-binary disclosure**: each entry MUST record:
  1. The binary name and source path.
  2. Whether it produces a Docker image (yes/no).
  3. If yes: the `Dockerfile`, the image repository, and the publishing
     mechanism (e.g. GitHub Actions workflow).
  4. If no: a one-line rationale (e.g. "CLI tool, not a deployed workload").
- **Same-change rule**: adding, renaming, or removing a `[[bin]]` target, or
  adding/removing a Docker image for an existing binary, MUST update
  `ARTIFACTS.md` in the same change (same commit / PR). A `[[bin]]` landing
  without a matching `ARTIFACTS.md` delta is incomplete and MUST be blocked at
  review — the same standard as the README rule (Principle X) and the
  test-first rule (Principle VIII).
- **`ARTIFACTS.md` is the single source of truth**: the README, CONTRIBUTING,
  and specs MAY link to it but MUST NOT duplicate the inventory. When a doc
  and `ARTIFACTS.md` disagree, `ARTIFACTS.md` wins.
- **Rationale**: the repository produces multiple binaries with different
  deployment profiles (long-running server, separate controller, CLI tool).
  Without a single explicit inventory, it is unclear which binaries are
  containerised, which Dockerfiles exist, and which images are published —
  exactly the gap that left the equalizer binary without a publish workflow
  after it was added. Making the inventory a first-class, mandated document
  closes this gap by construction: every new binary is declared, and its
  containerisation status is explicit, never implicit.

### XV. Build and Publish Procedure for Every Docker Artifact

Every binary marked as producing a Docker image in `ARTIFACTS.md` MUST have an
explicitly defined, reproducible build and publish procedure. A Dockerfile
without a publish mechanism is an incomplete deliverable.

- **Scope**: this principle applies to every artifact whose "Docker image"
  column in `ARTIFACTS.md` is "Yes". Artifacts marked "No" (e.g. CLI tools)
  are out of scope.
- **Required elements**: for each containerised artifact, the repository MUST
  define:
  1. **Build**: the Dockerfile (or equivalent) that produces the image.
  2. **Publish**: the mechanism that pushes the image to a registry — a CI
     workflow, a script, or a documented manual procedure.
  3. **Registry and repository**: the destination image repository (e.g.
     `aectann/emergency-ration-webhook`) recorded in `ARTIFACTS.md`.
  4. **Tagging strategy**: how image tags are derived (semver git tag,
     commit SHA, `latest`), documented alongside the publish mechanism.
- **Same-change rule**: adding a Dockerfile or flipping an artifact's image
  status to "Yes" in `ARTIFACTS.md` MUST land with a publish procedure in the
  same change (same commit / PR). A Dockerfile without a corresponding publish
  step is incomplete and MUST be blocked at review — the same standard as the
  artifact inventory rule (Principle XIV) and the README rule (Principle X).
- **No orphan Dockerfiles**: a `Dockerfile.*` in the repository root that has
  no publish procedure is a defect. Either wire it into a publish mechanism or
  remove it; leaving it unmentioned is not acceptable.
- **Rationale**: the equalizer binary received a `Dockerfile.equalizer` during
  its initial implementation but no publish workflow was added — the image was
  buildable locally but never reachable from CI or a registry. An operator
  pulling the image list from `ARTIFACTS.md` would see "Yes" next to the
  equalizer but find no way to obtain the published image. Requiring the
  publish procedure to land with the Dockerfile eliminates this class of gap:
  if the image is declared, the procedure to build and ship it is declared too.

## Technology Constraints

- **Language**: Rust (current stable edition; MSRV recorded in `Cargo.toml`).
  The webhook and its capacity-tracking logic are implemented in Rust for
  latency, memory footprint, and correctness on the admission critical path.
- **Runtime target**: Linux container, deployed as a Kubernetes workload
  (`Deployment` behind a `Service`, served over HTTPS; DaemonSet is an
  alternative to be settled in the plan).
- **Kubernetes surface**: ValidatingWebhookConfiguration (v1) + two CRDs
  (ClusterCapacity, Allocation). No MutatingWebhook in scope.
- **Architecture**: 3-component operator — Node Capacity Controller,
  Allocation Controller, Admission Webhook — linked by CRDs as shared state
  (see Principle V for the data-flow diagram and component responsibilities).
- **Capacity inputs**: cluster node capacity (`.status.allocatable`) aggregated
  by the Node Capacity Controller into a CRD; pod resource requests summed by
  the Allocation Controller. Source of capacity *usage* = declared pod
  `resources.requests` (resolved in clarification — deterministic, consistent
  with kube-scheduler, no metrics-server dependency).
- **Configuration**: the target allocation threshold lives in the Allocation
  CRD `spec`; webhook settings via flags/env. Not compiled in.
- **Primary dependencies**: async runtime (`tokio`), HTTP/TLS server
  (`axum`/`hyper` + `rustls`), Kubernetes client/informer (`kube-rs`),
  `serde` for serialising admission objects, `tracing` for structured logs,
  a Prometheus metrics crate.
- **Testing**: unit tests via standard `#[test]`; integration tests via
  `tower-test` (mocked apiserver `tower::Service`); BDD via `cucumber-rs`
  (`.feature` files); E2E via `k3d`/`kind` on CI across the N-2 matrix.
  `kube-rs/envtest` is explicitly rejected for v1 (Go toolchain cost violates
  Principle V).
- **Performance targets (provisional, ratify in /speckit-plan)**: p99 admission
  decision < 100 ms excluding kube-apiserver overhead, < 50 ms p50; webhook
  resource footprint target < 256 Mi request, < 500 m CPU.
- **Security**: TLS for the webhook endpoint (cert from a Secret or issued by
  cert-manager); least-privilege RBAC (read on nodes + pods; no writes); no
  secrets stored or logged by the webhook.
- **No host paths or machine-specific paths in tracked files.** The repository
  is portable across the dev setup.

## Development Workflow

- **Spec-driven**: features are specified (`/speckit-specify`) and planned
  (`/speckit-plan`) before implementation. Implementation MUST cite the plan.
- **Dual-agent split**: planning (constitution, clarify, specify, plan) happens
  on the planning host; implementation (tasks, implement, test) is delegated to
  the coding agent on the build host. The git repository is the sync mechanism —
  planning commits are pulled before implementation begins.
- **Branch-and-PR workflow**: every spec is implemented on a dedicated feature
  branch (e.g. `spec/<feature>`, matching the `specs/<feature>/` directory) and
  merged into `main` **only** via a pull request — no implementation work lands
  directly on `main`. The pull request is the review and integration point: it
  MUST pass the quality gate (below) before merge. This governs implementation;
  planning artifacts (constitution, spec, plan) continue to land on `main` per
  the dual-agent split, since they are the input to — not the output of —
  implementation.
- **Test-first (TDD)**: development follows strict Red-Green-Refactor
  (Principle VIII). The admission decision logic MUST have unit tests covering
  admit, reject, and every enumerated failure-mode path from Principle III;
  these tests are written FIRST, watched to fail, then implemented. Capacity
  budget arithmetic MUST be tested at boundaries (exactly at ceiling, one unit
  over, zero remaining). Integration tests (Principle VI) are likewise written
  before the workflow they cover.
- **Formatting as code**: mechanical formatting (indent, line endings, final
  newline, trailing whitespace) is declared in `.editorconfig` (Principle IX)
  and MUST agree with the language's canonical formatter. Formatting drift is a
  review-blocking diff, not an editor preference.
- **Documentation as a deliverable**: every user-facing capability (flags, env
  vars, admission behaviour, CRD spec/status fields, metrics, log keys,
  deployment, upgrade notes) MUST be documented and discoverable from
  `README.md`, with detailed reference in `docs/` articles (Principle X).
  Every contribution-workflow capability (build steps, test commands,
  verification tooling, development process) MUST be documented in
  `CONTRIBUTING.md` (Principle XIII). Both are first-class deliverables updated
  in the same change — a PR shipping functionality without the matching doc
  delta is incomplete and blocked at review, on the same footing as the
  test-first (Principle VIII) and formatting (Principle IX) rules.
- **CI-green completion gate**: a task or feature is not complete until CI
  passes on the merge branch — all jobs, not just the Rust quality gate
  (Principle XI). A pre-existing infrastructure failure on `main` MUST be
  fixed before any change can be validated and merged; shipping on top of red
  CI is shipping unverified work. The implementation agent reports CI failures
  and fixes or escalates them — it does not declare success while the pipeline
  is red.
- **Quality gate**: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo
  test` (unit + integration) all green before merge. No admission-core change
  lands without a covering test, and no production code lands without a failing
  test first.
- **Integration test default**: the mocked-apiserver path (`tower-test`) is the
  default test target for `cargo test`; E2E tests run on CI against a `k3d`
  cluster and are marked `#[ignore]` so they do not run on a plain `cargo test`.
- **Verification gate**: a feature is not complete until its tests pass against
  the real code path, not a stub.
- **Scratch space**: transient artifacts (validation reports, intermediate
  logs, checkpoint dumps, agent-to-agent handoff files) MUST be written to the
  git-ignored `.temp/` directory (Principle XII), never to the repository root
  or any tracked directory. If an artifact must persist beyond the task, promote
  it to a tracked location (specs, README, source) with an explicit rationale.

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

**Version**: 2.9.0 | **Ratified**: 2026-07-25 | **Last Amended**: 2026-08-08
