# Feature Specification: README Documentation

**Feature Branch**: `spec/readme-documentation`

**Created**: 2026-07-26

**Status**: Draft

**Input**: User description: "All user-facing functionality should be thoroughly
documented in README.md." This spec brings the existing `README.md` (currently a
4-line stub) into compliance with Constitution Principle X, documenting the
already-shipped capacity admission webhook.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Installation & Quick Start (Priority: P1)

A cluster operator who has never seen this project before arrives at the
repository and wants to deploy the webhook into their cluster. From the README
alone — without reading source code, specs, or commit history — they can follow
a step-by-step quick start that takes them from clone to a running, functional
admission webhook in a Kubernetes cluster.

The quick start covers: prerequisites (Rust toolchain or pre-built image, a
Kubernetes cluster, `kubectl`), building the container image, applying the
deployment manifests (Namespace, RBAC, CRDs, Deployment, Service,
ValidatingWebhookConfiguration, TLS cert provisioning), and verifying the
webhook is operational (pods running, health check responding, a test pod
admitted). An operator who completes the quick start has a webhook that is
actively gating pod admission against the capacity budget.

**Why this priority**: A tool that cannot be installed is a tool that does not
exist. The README's first job is to get the operator from zero to a running
deployment. Without this, no other documentation matters — the operator never
gets far enough to need it.

**Independent Test**: Follow the quick start on a fresh cluster (e.g. `k3d`/
`kind`) and verify: the webhook pod reaches `Ready`, `/healthz` returns 200,
and a pod with resource requests is admitted while a pod that exceeds the
budget is rejected.

**Acceptance Scenarios**:

1. **Given** a developer with a local Kubernetes cluster and the repository
   cloned, **When** they follow the README quick start, **Then** they reach a
   state where the webhook Deployment shows all replicas `Ready` and the
   ValidatingWebhookConfiguration is registered.
2. **Given** the webhook is deployed per the quick start, **When** the operator
   applies the ClusterCapacity and Allocation CRD instances and submits a test
   pod, **Then** the pod is admitted or rejected according to the budget, and
   the operator can see the decision reflected in the webhook logs.
3. **Given** the TLS certificate provisioning section of the README, **When**
   the operator follows it, **Then** the webhook's HTTPS endpoint serves with a
   valid certificate (via cert-manager or manual Secret) and the
   apiserver successfully reaches `/validate`.

---

### User Story 2 - Configuration Reference (Priority: P2)

A cluster operator needs to configure the webhook's runtime behaviour — the
admission budget threshold, TLS settings, timeouts, ports, namespace — and
needs to understand what each configuration knob does, its default value, and
the implications of changing it. From the README's configuration section alone,
the operator can find every configurable parameter and make an informed
decision without inspecting source code or guessing.

The configuration reference covers: all CLI flags and their environment-variable
equivalents (with defaults and types), the Allocation CRD `spec.budgetPercent`
field (the runtime-adjustable budget), the ClusterCapacity and Allocation CRD
`status` fields (what the operator can inspect), and the precedence rules
(CLI flag → environment variable → compiled default). An operator who reads this
section knows every knob the webhook exposes and how to turn it.

**Why this priority**: Once installed, configuration is the most common operator
interaction. An undocumented flag or an unknown default is an operational
hazard — the operator cannot tune what they cannot find, and silent defaults
can mask misconfiguration (e.g. a freshness timeout too loose to catch stale
capacity data).

**Independent Test**: Open the README configuration section. Pick any flag or
CRD field at random and verify its name, type, default, and effect are stated.
Change the budget via the Allocation CRD and confirm the new ceiling takes
effect without a restart, as the README describes.

**Acceptance Scenarios**:

1. **Given** the README configuration section, **When** an operator looks for
   any CLI flag (e.g. `--decision-timeout-ms`), **Then** they find a table or
   list entry naming the flag, its env-var equivalent (`DECISION_TIMEOUT_MS`),
   its type, its default value, and a one-line description of what it controls.
2. **Given** the README configuration section, **When** an operator wants to
   change the budget ceiling at runtime, **Then** they find instructions to
   patch the Allocation CRD `spec.budgetPercent` and an explanation that the
   change takes effect without a restart (hot-reload via controller reconcile).
3. **Given** the CRD reference in the README, **When** an operator inspects the
   ClusterCapacity or Allocation status, **Then** they find a field-by-field
   description (name, type, unit, meaning) so they can interpret what the
   controller has computed.
4. **Given** the README, **When** an operator needs to know the precedence
   (flag vs. env vs. default), **Then** the precedence rule is explicitly
   stated, not left to inference.

---

### User Story 3 - Operations & Observability (Priority: P3)

An operator running the webhook in production needs to monitor its health,
understand its admission decisions, debug a rejection, and know its failure
mode behaviour. From the README's operations section alone, the operator can
set up a Prometheus scrape, interpret the exposed metrics, read a structured
log line, understand the fail-closed guarantees, and troubleshoot a denied pod.

The operations section covers: the exposed HTTP endpoints (`/validate`,
`/metrics`, `/healthz`) and their ports (HTTPS 8443, HTTP 9090), the full set
of Prometheus metrics (names, types, labels, meaning), the structured-log
format and key fields, the fail-closed failure model (every degradation path
rejects), the Kubernetes version support window (N-2), and guidance on reading
a rejection message. An operator who reads this section can build a dashboard,
set an alert, and explain to a workload owner why a pod was rejected.

**Why this priority**: Observability and operability are what make the webhook
trustworthy in production. This ranks after installation and configuration
because it presupposes a running, configured deployment — but it is essential
for day-2 operations, incident response, and building operator confidence.

**Independent Test**: Point a Prometheus instance at the documented metrics
endpoint and verify all seven metric families appear. Trigger a denial and
verify the rejection message and structured log match the README's documented
format. Simulate a capacity-data-stale condition and verify the webhook rejects
as the README's fail-closed section describes.

**Acceptance Scenarios**:

1. **Given** the README metrics section, **When** an operator scrapes the
   `/metrics` endpoint, **Then** every documented metric name is present in the
   exposition and its type, labels, and meaning match the README description.
2. **Given** a pod is rejected for exceeding the budget, **When** the operator
   reads the rejection message, **Then** it matches the format documented in
   the README (violated resource, current, requested, projected, ceiling).
3. **Given** the README operations section, **When** the webhook cannot reach
   the capacity data (stale beyond the freshness threshold), **Then** the
   documented fail-closed behaviour (reject, log the reason) matches what the
   operator observes.
4. **Given** the README, **When** an operator needs to know which Kubernetes
   versions are supported, **Then** the N-2 support window and the CI-tested
   version matrix are documented and discoverable.

---

### Edge Cases

- **Offline / air-gapped cluster**: the README quick start should note that the
  container image must be available in a registry the cluster can reach (or
  loaded locally), since `imagePullPolicy: IfNotPresent` assumes a present
  image.
- **Custom namespace**: the default namespace is `capacity-admission`; the README
  should note that changing it requires updating the namespace in the Deployment,
  RBAC, ValidatingWebhookConfiguration `namespaceSelector`, and the webhook's
  `--namespace` flag consistently.
- **TLS without cert-manager**: the README should cover both the cert-manager
  path (automated) and the manual Secret path (for clusters without
  cert-manager), since TLS is mandatory for the webhook endpoint.
- **Budget set to 0% or 100%**: the README configuration section should document
  the edge behaviours (0% = circuit-breaker, 100% = physical overcommit guard)
  so operators do not treat them as bugs.
- **Webhook self-admission (bootstrap)**: the README should explain how the
  `namespaceSelector` exclusion prevents the webhook from blocking its own
  deployment.
- **Metrics port exposure**: the README should note that the metrics endpoint is
  plaintext HTTP and should not be exposed externally without an additional
  network policy or auth layer.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The README MUST contain a quick start guide that takes an operator
  from clone to a running webhook in a Kubernetes cluster, covering image build,
  manifest application, TLS provisioning, and verification.
- **FR-002**: The README MUST list every CLI flag and its environment-variable
  equivalent, with the flag name, env-var name, type, default value, and a
  one-line description — covering all seven configurable parameters.
- **FR-003**: The README MUST document the Allocation CRD (`spec.budgetPercent`
  and all `status` fields) and the ClusterCapacity CRD (all `status` fields),
  with field name, type, unit, and meaning for each.
- **FR-004**: The README MUST document all three HTTP endpoints (`/validate`,
  `/metrics`, `/healthz`), their serving protocol (HTTPS/HTTP), their ports, and
  their purpose.
- **FR-005**: The README MUST list all seven Prometheus metrics by name, with
  their type (counter/histogram/gauge), labels, and a description of what each
  measures.
- **FR-006**: The README MUST document the fail-closed failure model: every
  degradation path (stale data, component unreachable, timeout, malformed
  request, unknown error) results in rejection, never silent admission.
- **FR-007**: The README MUST document the Kubernetes version support window
  (N-2: the three most recent major releases) and reference the CI test matrix.
- **FR-008**: The README MUST document the precedence rule for configuration
  (CLI flag → environment variable → compiled default).
- **FR-009**: The README MUST document the runtime budget-adjustment workflow
  (patching the Allocation CRD `spec.budgetPercent` takes effect without
  restart).
- **FR-010**: The README MUST document the rejection message format so operators
  and workload owners can interpret a denial (violated resource, current
  allocation, requested increment, projected total, ceiling) without contacting
  the platform team.
- **FR-011**: The README MUST be the single entry point: any deeper reference
  material (CRD schemas, architecture) MAY be linked from the README, but the
  README itself MUST cover the essentials and not delegate the core operator
  workflow elsewhere.
- **FR-012**: The README MUST be accurate against the shipped code as of the
  current `main` branch — documented flag names, defaults, metric names, CRD
  fields, and ports MUST match the implementation, not aspirational or stale
  values.

### Key Entities *(include if feature involves data)*

- **README.md**: the single entry point for all user-facing documentation. This
  is the deliverable of this spec.
- **CLI flags / environment variables**: the seven runtime configuration knobs
  (`--port`, `--tls-cert-file`, `--tls-key-file`, `--decision-timeout-ms`,
  `--capacity-freshness-timeout-secs`, `--namespace`, `--metrics-port`) and
  their env-var equivalents.
- **Allocation CRD**: the user-facing custom resource carrying the configurable
  `budgetPercent` and the controller-computed status (allocated, ceiling,
  utilization, last-updated).
- **ClusterCapacity CRD**: the supply-side custom resource carrying
  controller-computed total allocatable capacity.
- **Prometheus metrics**: the seven exposed metric families that operators use
  for dashboards and alerts.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator who has never seen the project can deploy a working
  webhook into a fresh cluster by following only the README quick start — no
  external documentation, source-code reading, or team consultation required.
- **SC-002**: Every CLI flag, env-var, CRD field, metric name, and endpoint
  documented in the README matches the shipped implementation on `main` — 100%
  accuracy, zero stale or invented values.
- **SC-003**: An operator can find any configurable parameter in the README
  within seconds (via a table or structured section), including its default and
  effect.
- **SC-004**: An operator can build a Prometheus dashboard and interpret a
  rejection message using only the README's metrics and operations sections.
- **SC-005**: The README is the single entry point — it covers installation,
  configuration, operations, and troubleshooting, linking to deeper material
  only for non-essential detail.

## Assumptions

- **The webhook implementation on `main` is the source of truth.** This spec
  documents what already exists (all 44 tasks of spec 001 are complete and
  merged). It does not specify new webhook functionality — only documentation of
  the existing surface.
- **The README is written in English**, matching the rest of the project's
  documentation. Localisation is out of scope.
- **The target audience is a Kubernetes cluster operator or SRE** who is
  comfortable with `kubectl`, pods, requests/limits, and admission webhooks, but
  should not need to read Rust source code to operate the webhook.
- **Deeper architectural material (3-component design, CRD data-model) MAY be
  linked from the README** to the `specs/` directory, but the README MUST
  summarise enough for an operator to understand what the components do without
  reading the full spec.
- **The README documents the current (v1) surface only.** Future features
  (per-namespace budgets, mutating webhooks) are out of scope and should not be
  documented as if they exist.
- **TLS certificate provisioning is documented for both paths** (cert-manager
  automated, manual Secret) because both are valid deployment patterns for the
  target audience.
