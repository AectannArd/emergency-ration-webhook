# Feature Specification: Per-Resource Budget Tracking

**Feature Branch**: `012-per-resource-budget`

**Created**: 2026-08-06

**Status**: Draft

**Input**: User description: "they should maintain separate limits" — CPU and RAM
budgets must be configurable independently, so that a deployment can enforce a
stricter ceiling on one resource than the other (e.g. protect RAM more tightly
while allowing higher CPU headroom).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Asymmetric Budgets Per Resource (Priority: P1)

As a cluster operator, I want to set a higher budget for CPU than for RAM (or
vice versa) on the cluster Allocation singleton, so that the admission webhook
enforces an independent ceiling on each resource and I can tune headroom per
resource according to which one is the actual contention point in my fleet.

**Why this priority**: this is the entire point of the feature. Until CPU and
RAM can be budgeted independently, the system cannot express "RAM is precious,
CPU is ample" — the single `budgetPercent` forces both resources onto one
number. P1 because it is the smallest slice that delivers user value on its own:
once both overrides exist and the webhook honours them, the feature is usable.

**Independent Test**: set `cpuBudgetPercent: 90` + `memoryBudgetPercent: 60`
on the cluster Allocation singleton (leaving the legacy `budgetPercent` at its
prior value). Launch a pod whose CPU fits at 90% but whose memory exceeds 60%
of the cluster ceiling. The pod is admitted on CPU but denied on memory, and the
rejection message names memory (not CPU) as the violated resource. Reversing the
overrides (CPU 60 / RAM 90) and launching the symmetric pod flips which resource
denies — proving each resource is gated by its OWN ceiling, not a shared one.

**Acceptance Scenarios**:

1. **Given** an Allocation singleton with `cpuBudgetPercent: 90` and
   `memoryBudgetPercent: 60` (no legacy `budgetPercent` change), **When** a pod
   whose projected CPU fits under the 90% CPU ceiling but whose projected memory
   exceeds the 60% RAM ceiling is submitted, **Then** the admission webhook
   rejects the pod with a denial naming memory as the exceeded resource (CPU is
   not reported as violated).
2. **Given** the same singleton with the overrides swapped
   (`cpuBudgetPercent: 60`, `memoryBudgetPercent: 90`), **When** the symmetric
   pod (CPU-heavy, memory-light) is submitted, **Then** the pod is rejected with
   CPU named as the exceeded resource (memory is not reported).
3. **Given** an Allocation singleton with only `cpuBudgetPercent: 90` set and
   `memoryBudgetPercent` absent, **When** a memory-heavy pod is submitted,
   **Then** the pod is evaluated against the legacy `budgetPercent` value for its
   memory ceiling (partial override — one resource overridden, the other falls
   back). The CPU ceiling derives from the override.
4. **Given** an Allocation singleton where the operator patches BOTH overrides to
   new values at runtime (no webhook restart), **When** the next pod is
   submitted, **Then** both ceilings reflect the new overrides immediately
   (runtime-adjustable, consistent with the existing `budgetPercent` pattern).

---

### User Story 2 - Backward Compatibility with Single Budget (Priority: P2)

As a cluster operator with an existing deployment that uses only
`budgetPercent`, I want my cluster to behave exactly as before after upgrading
to the per-resource release — no CRD migration, no spec change required, no
behavioural drift — so that the per-resource feature is strictly additive and
carries no upgrade risk.

**Why this priority**: this is the safety net. Without guaranteed backward
compatibility the feature is undeployable for any existing cluster. P2 (not P1)
because it does not add new capability — it certifies that the P1 capability
does not regress existing behaviour. But it is mandatory before merge: a
behavioural change to single-budget deployments is a release blocker.

**Independent Test**: upgrade a cluster whose Allocation singleton carries only
`budgetPercent: 80` (no overrides) to the new release. Run the existing
admission test suite (budget enforcement, capacity awareness, dry-run, fail-safe,
exclusion) unchanged. Every existing test passes with identical verdicts, and the
Allocation status `ceiling_cpu_milli` / `ceiling_memory_bytes` are byte-for-byte
identical to the pre-upgrade values for the same cluster capacity.

**Acceptance Scenarios**:

1. **Given** a pre-feature Allocation singleton with `budgetPercent: 80` and no
   override fields, **When** the singleton is loaded by the new controller, **Then**
   the effective CPU budget, effective memory budget, computed CPU ceiling, and
   computed memory ceiling are all identical to what the legacy controller would
   have produced (80% for both).
2. **Given** the same pre-feature singleton, **When** it is serialised back (e.g.
   via the CRD round-trip or controller status write), **Then** the override
   fields are absent from the JSON (not defaulted to `budgetPercent`'s value, not
   populated with `null` — they remain unset, matching their `Option<>` storage).
3. **Given** a pod that the legacy webhook would have admitted at `budgetPercent:
   80`, **When** the same pod is evaluated by the new webhook against the same
   singleton (no overrides), **Then** the verdict is identical (admit, same
   capacity figures in the summary, no new warnings or reasons).

---

### User Story 3 - Observability of Effective Per-Resource Budget (Priority: P3)

As an operator or SRE debugging an unexpected admit/deny, I want to see — in the
admission decision's structured log, the metrics, and the Allocation CRD status —
exactly which effective budget percentage was applied to CPU and which to
memory, including which value was the source (override vs fallback to
`budgetPercent`), so that I can reconstruct why a pod was admitted or denied
without guessing which field governed the ceiling.

**Why this priority**: Constitution Principle IV (Observability Before
Optimisation) makes this mandatory, but it is P3 because it is the verification
surface on top of P1/P2: once the budgets are split and backward-compatible,
exposing the effective values is the natural completion. A v1 that shipped P1
without P3 would work but be opaque; P3 makes it debuggable.

**Independent Test**: set `cpuBudgetPercent: 90` and leave `memoryBudgetPercent`
absent with `budgetPercent: 70`. Submit a pod that is denied on memory. The
structured log line and the summary carry `effective_cpu_budget_percent: 90`
(source: override) and `effective_memory_budget_percent: 70` (source: fallback)
as distinct fields, and the Allocation status exposes the effective per-resource
budgets so `kubectl get allocations -o yaml` shows them without recomputation.

**Acceptance Scenarios**:

1. **Given** an Allocation singleton with `cpuBudgetPercent: 90`, no
   `memoryBudgetPercent`, and `budgetPercent: 70`, **When** any admission decision
   is made (admit or deny), **Then** the structured log carries both
   `effective_cpu_budget_percent` (90, source override) and
   `effective_memory_budget_percent` (70, source fallback) as explicit fields.
2. **Given** the same singleton after the Allocation Controller has reconciled,
   **When** the operator reads `kubectl get allocations cluster-allocation -o
   yaml`, **Then** the status reports the effective CPU budget percent and the
   effective memory budget percent as computed values (distinct from the raw spec
   fields), so the applied budgets are inspectable without manual resolution.
3. **Given** an Allocation singleton with no overrides (legacy mode), **When**
   the controller reconciles, **Then** the status reports both effective
   per-resource budgets as equal to `budgetPercent` (no false "override applied"
   signal — the fallback source is recorded faithfully).

---

### Edge Cases

- **Both overrides absent** → both resources use `budgetPercent` (legacy path,
  US2 AC1). This is the overwhelmingly common case and MUST be bit-identical to
  the pre-feature behaviour.
- **Both overrides present** → both resources use their respective overrides;
  `budgetPercent` is ignored entirely for ceiling computation (but MUST still be
  required by the schema — see FR-006).
- **Only one override present** → the overridden resource uses its override; the
  other falls back to `budgetPercent` (US1 AC3). This is a legitimate partial
  configuration, not an error.
- **Override equals `budgetPercent`** → equivalent to the legacy path; the
  ceiling is the same. No special handling needed, but the observability fields
  should still report the value as override-sourced (FR-010) so operators can
  see the override is set.
- **Override at 0** → the resource has a zero ceiling (no pod requesting that
  resource is admitted). This is the existing `budgetPercent: 0` semantics,
  applied per-resource. A pod requesting 0 of the resource still admits on that
  resource (the existing inclusive-ceiling rule). Must not panic.
- **Override at 100** → the resource ceiling equals total allocatable (the
  existing full-capacity semantics). Must not overflow the ceiling arithmetic
  (the existing 128-bit intermediate in `ceiling()` already guards this).
- **Operator sets only `memoryBudgetPercent`** → symmetric to "only CPU"; CPU
  falls back. Must work identically regardless of which single resource is
  overridden.
- **All three fields present and consistent** (e.g. `budgetPercent: 80`,
  `cpuBudgetPercent: 80`, `memoryBudgetPercent: 80`) → overrides take precedence
  but produce identical ceilings. No conflict.
- **Negative override** → rejected by the CRD schema (`#[schemars(range(min = 0,
  max = 100))]`), consistent with the existing `budgetPercent` range validation.
- **Webhook namespace / excluded-namespace / excluded-priority-class pods** →
  unaffected; the exemption path (spec-008) runs before budget resolution and is
  not changed by this feature.
- **Dry-run mode** (spec-004) → the per-resource ceilings feed the existing
  budget check; a memory-only violation in dry-run mode produces a memory-only
  warning, symmetric to the enforce-mode per-resource violation reporting.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Allocation CRD spec MUST accept two new optional fields,
  `cpuBudgetPercent` and `memoryBudgetPercent`, each an integer in the range 0–100,
  serialising as `cpuBudgetPercent` / `memoryBudgetPercent` in camelCase, mirroring
  the existing `budgetPercent` field.

- **FR-002**: The system MUST resolve the effective CPU budget as
  `cpuBudgetPercent` when present, else `budgetPercent`. The effective memory
  budget resolves symmetrically via `memoryBudgetPercent`, else `budgetPercent`.
  Each resource's resolution is independent.

- **FR-003**: The Allocation Controller MUST compute the CPU ceiling from the
  effective CPU budget and the memory ceiling from the effective memory budget,
  using the existing `ceiling(total, percent)` arithmetic. The status fields
  `ceiling_cpu_milli` and `ceiling_memory_bytes` reflect the per-resource
  ceilings.

- **FR-004**: The admission webhook MUST enforce the per-resource ceilings
  written by the Allocation Controller. The existing `check_budget` logic
  (already per-resource) requires NO change — it reads the ceilings from status,
  which now reflect independent budgets.

- **FR-005**: When both override fields are absent, the effective per-resource
  budgets MUST both equal `budgetPercent`, and the computed ceilings MUST be
  byte-identical to those produced by the pre-feature controller for the same
  `budgetPercent` and cluster capacity. (Backward compatibility — US2 AC1.)

- **FR-006**: The legacy `budgetPercent` field MUST remain present and required
  on the Allocation spec. It continues to serve as the fallback for any resource
  without an override, and guarantees that a pre-feature singleton remains valid
  without migration. (This is the keystone of backward compatibility: the field
  cannot be made optional, because a singleton with no overrides and no
  `budgetPercent` would have no budget at all.)

- **FR-007**: The Allocation Controller MUST keep the legacy
  `budget_percent` field untouched on auto-creation and auto-heal — it seeds
  `budget_percent` with the existing default and never modifies it based on the
  overrides. The overrides are operator-set; the controller does not infer them.

- **FR-008**: The auto-created `cluster-allocation` singleton MUST be seeded
  with both override fields absent (`None`), preserving the legacy default
  behaviour on first cluster boot. Operators opt into per-resource budgets by
  patching the overrides after creation.

- **FR-009**: The Allocation status MUST expose the effective CPU budget
  percent and the effective memory budget percent as computed fields
  (`effectiveCpuBudgetPercent` / `effectiveMemoryBudgetPercent`), so operators
  can inspect the applied budgets via `kubectl get allocations -o yaml` without
  manually resolving override-vs-fallback.

- **FR-010**: The structured admission log MUST carry
  `effective_cpu_budget_percent` and `effective_memory_budget_percent` as
  distinct fields on every budget-resolved decision (admit, deny, dry-run-deny).
  Fail-closed paths that return before budget resolution (missing allocation,
  stale, exemption) are exempt from this requirement — they carry no budget
  figures today and this feature does not change that.

- **FR-011**: The system MUST treat a pod that is over budget on exactly one
  resource as denied for that resource only — the rejection message and any
  dry-run warning name the violated resource (or resources), never the
  non-violated one. (This is the existing `check_budget` behaviour; this FR
  documents that per-resource budgets do not weaken it — with independent
  ceilings, a pod can now be over on one resource while comfortably under on the
  other, and the reporting must remain per-resource accurate.)

- **FR-012**: The on-demand verification tool (`erw-verify`) MUST, as part of
  its scenario matrix, validate that setting asymmetric per-resource overrides
  produces per-resource enforcement: a pod admitted on CPU but denied on memory
  when `cpuBudgetPercent` is high and `memoryBudgetPercent` is low. (This extends
  the existing enforcement scenario; it does not require a new binary or new
  CLI surface — the scenario exercises the existing Allocation patch +
  admission path with the new fields.)

### Key Entities *(include if feature involves data)*

- **Allocation CRD (extended)**: the existing cluster-scoped singleton
  (`cluster-allocation`, group `emergency-ration.dev/v1`, kind `Allocation`).
  The spec gains two optional integer fields `cpuBudgetPercent` /
  `memoryBudgetPercent` (0–100). The status gains two computed fields
  `effectiveCpuBudgetPercent` / `effectiveMemoryBudgetPercent`. No new CRD is
  introduced; no new singleton. The CRD's OpenAPI schema version remains `v1`
  (additive fields, backward-compatible).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A cluster operator can enforce an 80% CPU budget and a 50% RAM
  budget simultaneously on a single Allocation singleton, and the admission
  webhook enforces each independently — a CPU-heavy pod is admitted while a
  RAM-heavy pod (under the same CPU pressure) is denied on memory alone.

- **SC-002**: An existing cluster upgraded to the per-resource release with no
  spec change exhibits zero behavioural change: identical admission verdicts,
  identical `ceiling_cpu_milli` / `ceiling_memory_bytes` status values, and no
  new CRD validation errors, across the full existing integration and BDD test
  suites (budget enforcement, capacity awareness, dry-run, fail-safe, exclusion,
  node-filter).

- **SC-003**: The effective per-resource budget is observable in three places —
  the Allocation status (`kubectl get allocations -o yaml`), the structured
  admission log, and (via the existing summary path) the decision's capacity
  figures — with no manual override-vs-fallback resolution required by the
  operator.

- **SC-004**: The feature is delivered as a single backward-compatible change:
  one CRD schema bump (additive), no new components, no new CLI, no new RBAC,
  no webhook-path behavioural change. The blast radius is confined to the
  Allocation CRD types, the controller's ceiling computation, and the admission
  log/status fields.

## Assumptions

- The existing single-`budgetPercent` model is correct for its scope; this
  feature extends it per-resource rather than replacing it. Operators who do not
  need per-resource control are unaffected and require no action.
- The `ceiling(total, percent)` arithmetic and 128-bit-overflow guard (already
  in `src/webhook/admission.rs`) are sufficient for per-resource ceilings —
  splitting the budget per resource does not change the magnitude of any
  intermediate, so no new overflow protection is required.
- The admission webhook already evaluates CPU and RAM ceilings independently in
  `check_budget` (verified against `src/webhook/admission.rs`); the coupling
  this feature removes is only at the *budget-resolution* layer (one
  `budgetPercent` feeding both ceilings), not at the enforcement layer.
- The Allocation Controller's reconciliation loop already recomputes ceilings on
  every cluster-capacity or `budgetPercent` change; extending that loop to read
  the overrides and recompute per-resource ceilings is a localised change, not
  an architectural one.
- The CRD schema remains at apiVersion `v1` (additive optional fields are
  backward-compatible per Kubernetes CRD semantics; no conversion webhook is
  required).
- Per-resource budgets are intended for use by a downstream multi-cluster
  equalizer (separate spec) that will set the override fields programmatically;
  this feature makes the override fields available and enforced, but does not
  itself implement the equalizer.
- The feature does NOT amend the constitution: it strengthens Principle II
  (capacity as a hard budget now configurable per resource) within the existing
  3-component architecture (Principle V), with no new component, no new failure
  mode (Principle I/III — the fail-closed paths are upstream of budget
  resolution), and standard test/observability discipline (Principles IV/VI/VIII).
