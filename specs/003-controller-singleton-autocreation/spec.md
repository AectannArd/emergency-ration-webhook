# Feature Specification: Controller Singleton Autocreation

**Feature Branch**: `spec/controller-singleton-autocreation`

**Created**: 2026-07-26

**Status**: Draft

**Input**: The Node Capacity Controller and Allocation Controller assume their
singleton CRD instances (`cluster-capacity`, `cluster-allocation`) already exist
before patching their status. When the instances are absent, `patch_status`
returns 404 NotFound and the status is never written. This leaves the webhook
with no capacity data, so it rejects ALL pods (fail-closed per Principle I) —
making the webhook non-functional until an operator manually creates both
singletons. The controllers MUST create-or-ensure their singleton instances.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - ClusterCapacity Singleton Autocreation (Priority: P1)

The Node Capacity Controller starts up in a fresh cluster (or one where the
`cluster-capacity` ClusterCapacity instance has not been created). Instead of
logging a 404 warning on every reconcile attempt, the controller creates the
`cluster-capacity` singleton with an empty spec, then immediately patches its
status with the aggregated node allocatable figures. The webhook becomes
functional as soon as the controller reconciles — no operator intervention
required.

**Why this priority**: Without the ClusterCapacity instance, the Node Capacity
Controller cannot publish capacity data. Without capacity data, the Allocation
Controller cannot compute ceilings. Without ceilings, the webhook rejects all
pods. This is the root cause of the E2E CI failures and would block any
real deployment.

**Independent Test**: Deploy the webhook into a fresh cluster where neither
singleton exists. Observe the Node Capacity Controller creates
`cluster-capacity` and populates its status within the first reconcile cycle.

**Acceptance Scenarios**:

1. **Given** a cluster with nodes but no `cluster-capacity` ClusterCapacity
   instance, **When** the Node Capacity Controller starts, **Then** it creates
   the `cluster-capacity` instance with an empty spec and patches its status
   with the aggregated allocatable figures.
2. **Given** the `cluster-capacity` instance already exists, **When** the Node
   Capacity Controller starts, **Then** it does NOT recreate or overwrite the
   instance — it proceeds directly to patching the status.
3. **Given** the `cluster-capacity` instance is deleted while the controller is
   running, **When** the next reconcile cycle fires, **Then** the controller
   re-creates the singleton and resumes patching its status.

---

### User Story 2 - Allocation Singleton Autocreation (Priority: P2)

The Allocation Controller starts up in a cluster where the
`cluster-allocation` Allocation instance does not exist. Instead of silently
returning from `recompute` (current behaviour), the controller creates the
`cluster-allocation` singleton with a default `budgetPercent` of 80. If the
instance already exists (created by the operator with a specific budget), the
controller MUST NOT overwrite it.

**Why this priority**: The Allocation singleton carries the user-configurable
`budgetPercent`. The auto-creation uses 80% as a safe default — enough headroom
for production while preventing overcommit. Operators can patch the budget at
runtime (FR-009 of spec-002). This ranks after US1 because the Allocation
Controller depends on ClusterCapacity data to compute ceilings.

**Independent Test**: Deploy the webhook without creating the
`cluster-allocation` instance. Observe the controller creates it with
`budgetPercent: 80` and populates the status within the first recompute cycle.

**Acceptance Scenarios**:

1. **Given** a cluster with no `cluster-allocation` Allocation instance,
   **When** the Allocation Controller starts, **Then** it creates the
   `cluster-allocation` instance with `spec.budgetPercent: 80`.
2. **Given** the `cluster-allocation` instance already exists with
   `budgetPercent: 50`, **When** the Allocation Controller starts, **Then** it
   does NOT overwrite the budget — it reads the existing value and uses it for
   ceiling computation.
3. **Given** the `cluster-allocation` instance is deleted while the controller
   is running, **When** the next recompute cycle fires, **Then** the controller
   re-creates the singleton with the default `budgetPercent: 80`.

---

### User Story 3 - Documentation Update (Priority: P3)

The contracts (`specs/001-capacity-admission-webhook/contracts/`), the README
quick start, and the CI workflow are updated to reflect that operators no longer
need to manually create either singleton. The controllers own their singleton
lifecycle. The CI workflow removes the manual ClusterCapacity creation step. The
README quick start removes the manual singleton-creation commands (keeping only
the budget-adjustment instructions for operators who want to change the default
80%).

**Why this priority**: Accurate documentation follows the code fix. It is
essential (Principle X) but comes after the functional fix.

**Independent Test**: Follow the README quick start on a fresh cluster — it
should not include any step to create CRD instances manually. The webhook should
become functional automatically.

**Acceptance Scenarios**:

1. **Given** the updated README quick start, **When** an operator follows it,
   **Then** there are no manual `kubectl apply` steps for `cluster-capacity` or
   `cluster-allocation` singletons.
2. **Given** the CI workflow, **When** E2E runs, **Then** only the Allocation
   budget-configuration step creates the `cluster-allocation` instance (with a
   specific budget for testing) — the ClusterCapacity instance is NOT created
   manually (the controller handles it).
3. **Given** the contracts documentation, **When** a reader checks the
   controller behaviour section, **Then** the singleton autocreation
   responsibility is explicitly documented.

---

### Edge Cases

- **Race condition: two controller replicas create the singleton
  simultaneously**: both replicas call create; one succeeds, the other gets an
  AlreadyExists (409) which is treated as success — the instance exists, proceed
  to patch_status. This is safe because the spec is deterministic (empty for
  ClusterCapacity, budgetPercent=80 for Allocation).
- **ClusterCapacity created by operator before controller starts**: the
  controller must NOT overwrite it. `get` first, only `create` if `get` returns
  NotFound.
- **Allocation created by operator with budgetPercent=0 (circuit-breaker)**:
  the controller must respect the existing value and NOT overwrite it with 80.
  Auto-creation only happens when the instance is entirely absent.
- **CRD definition not yet applied**: if the CRD definition itself doesn't exist
  (e.g. manifests applied in wrong order), `create` returns a different error
  (not NotFound/AlreadyExists). The controller logs the error and retries on the
  next cycle — the CRD will be applied eventually.
- **API server temporarily unreachable**: `get` and `create` may fail transiently.
  The controller logs the error and retries on the next reconcile/recompute cycle.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Node Capacity Controller MUST ensure the `cluster-capacity`
  ClusterCapacity singleton instance exists before attempting to patch its
  status. If the instance does not exist, the controller MUST create it with an
  empty `ClusterCapacitySpec {}`.
- **FR-002**: The Allocation Controller MUST ensure the `cluster-allocation`
  Allocation singleton instance exists before attempting to read its spec or
  patch its status. If the instance does not exist, the controller MUST create it
  with `AllocationSpec { budget_percent: 80 }`.
- **FR-003**: If either singleton already exists, the controller MUST NOT
  overwrite or recreate it — the existing instance (including any operator-set
  fields) is preserved.
- **FR-004**: If `patch_status` returns a 404 NotFound (the singleton was
  deleted mid-operation), the controller MUST re-create the singleton and retry
  the status patch on the next reconcile cycle.
- **FR-005**: The singleton creation MUST be idempotent — calling it multiple
  times (e.g. from two replicas) MUST NOT fail. An AlreadyExists (409) response
  is treated as success.
- **FR-006**: The CI workflow (`.github/workflows/ci.yml`) MUST be updated to
  remove the manual `cluster-capacity` ClusterCapacity creation step. The
  controller now handles it. The Allocation creation step may remain (to set a
  specific test budget) or be removed if the default 80% is acceptable for E2E.
- **FR-007**: The README quick start MUST be updated to remove any manual
  singleton-creation steps. Operators only need to deploy the manifests and
  optionally adjust the budget.
- **FR-008**: The contracts documentation (under
  `specs/001-capacity-admission-webhook/contracts/`) MUST be updated to
  explicitly document the singleton autocreation responsibility.

### Key Entities

- **ClusterCapacity singleton** (`cluster-capacity`): supply-side CRD instance.
  Auto-created by the Node Capacity Controller with empty spec; status patched
  with aggregated node allocatable figures.
- **Allocation singleton** (`cluster-allocation`): demand-side CRD instance.
  Auto-created by the Allocation Controller with `budgetPercent: 80` default;
  status patched with allocation figures. Operator can override the budget at
  runtime.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The webhook becomes functional (admits/denies pods correctly) in a
  fresh cluster without any manual CRD instance creation — controllers
  auto-create their singletons.
- **SC-002**: E2E CI passes on all three Kubernetes versions (1.34, 1.35, 1.36)
  without manual ClusterCapacity creation in the workflow.
- **SC-003**: An operator who has set a custom `budgetPercent` does not have it
  overwritten by the controller on restart.
- **SC-004**: The README quick start and contracts accurately reflect the
  autocreation behaviour — no manual singleton creation steps.

## Assumptions

- **The default budgetPercent for auto-creation is 80%**: this is a safe
  production default that leaves 20% headroom. Operators can change it at
  runtime via `kubectl patch allocation cluster-allocation`.
- **The CRD definitions (not instances) are still applied manually** via
  `deploy/crds.yaml` — the controllers auto-create instances, not CRD
  definitions.
- **Two replicas creating the singleton simultaneously is safe**: the ClusterCapacity
  spec is empty (deterministic); the Allocation spec uses a fixed default. A 409
  AlreadyExists on the second create is harmless.
- **The fix applies to the existing shipped code** (spec-001 implementation, all
  44 tasks merged). No new components or architecture changes.
