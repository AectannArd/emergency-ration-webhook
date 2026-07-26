# Phase 0: Research — Dry-Run Enforcement Mode

**Feature**: spec-004 (dry-run mode) | **Date**: 2026-07-27

This document resolves every technical unknown needed to produce the Phase 1
design artifacts. Each item carries a Decision, Rationale, and Alternatives
considered.

---

### R1. Enforcement mode field on the Allocation CRD spec

**Decision**: Add an optional field `enforcement_mode` to `AllocationSpec`,
serialised as `enforcementMode` (camelCase, matching the existing
`budgetPercent`). The type is an enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnforcementMode {
    Enforce,
    DryRun,
}
```

The field is `Option<EnforcementMode>` on the struct so an absent field
deserialises to `None` → treated as `Enforce` (FR-003). The auto-created
singleton seeds `Some(EnforcementMode::Enforce)` explicitly (FR-010).

**Rationale**: `serde(rename_all = "kebab-case")` produces the JSON values
`"enforce"` and `"dry-run"` — exactly the values operators type in `kubectl
patch`. An `Option` field with `None`-means-enforce semantics gives backward
compatibility with pre-existing Allocation singletons that predate this feature
(they simply have no `enforcementMode` field). An invalid value (e.g. `"audit"`)
fails deserialisation — but since the webhook reads from a reflector cache (not a
direct deserialisation), the cached object would simply not appear if the spec is
malformed. To be robust, the webhook resolves the mode via a helper that maps
`None` and any unrecognised value to `Enforce`.

**Alternatives considered**:
- A boolean `dryRun: bool` field — rejected: less self-documenting, harder to
  extend to future modes (e.g. `warn-only`), and `dryRun` collides with the
  AdmissionReview's own `dryRun` field conceptually.
- A string field with manual parsing — rejected: loses type safety, requires
  hand-written validation, and the derive macro generates the CRD OpenAPI schema
  automatically from the enum.

---

### R2. Reading enforcement mode in the webhook hot path

**Decision**: The `evaluate` function in `handler.rs` already reads
`allocation.spec.budget_percent` from the cached Allocation singleton. It will
also read `allocation.spec.enforcement_mode` (resolved to `EnforcementMode` via
the safe-default helper) in the same read. No additional cache, no network call,
no additional reflector — the mode travels on the same cached object as the
budget.

**Rationale**: The enforcement mode changes at human speed (an operator patches
it), and the reflector cache propagates spec changes within the watch latency
(typically <1s). This is identical to how `budgetPercent` changes propagate
(FR-011 of spec-001, already in production). Adding a separate watch or a
periodic GET for mode would violate Principle V (minimal surface) for no
benefit.

**Alternatives considered**:
- A dedicated informer for enforcement mode — rejected: unnecessary complexity;
  the mode is on the same CRD the webhook already watches.
- Reading the mode from the Allocation *status* instead of spec — rejected: the
  mode is an operator-set policy, not controller-computed state. It belongs in
  spec (like `budgetPercent`), not status.

---


### R3. Dry-run verdict conversion in the decision pipeline

**Decision**: The conversion happens at the **point where `check_budget` returns
`Deny(violations)`** in the `evaluate` function. Instead of immediately
producing a reject outcome, the code checks the enforcement mode:

- If `Enforce` → existing behaviour: produce a deny outcome (reject).
- If `DryRun` → produce an **admit** outcome with:
  - `response.allowed = true`
  - `response.warnings = Some(vec![would_be_rejection_message])`
  - `summary.verdict = DecisionVerdict::DryRunDeny` (new variant)
  - The summary carries the same capacity figures a real deny would.

The fail-closed paths (capacity data missing/stale, deserialisation failure,
timeout, panic) are **upstream** of `check_budget` — they return early via
`reject_outcome` before the budget check is ever reached. Therefore dry-run mode
cannot affect them. This is the architectural guarantee for FR-006.

**Rationale**: Placing the conversion at the deny branch of the budget check —
not at the response-building layer — keeps the fail-closed paths untouched. The
error paths never reach `check_budget`, so they are structurally incapable of
being converted to admits. This is the safest possible insertion point.

**Alternatives considered**:
- Converting at the HTTP response layer (after `evaluate` returns) — rejected:
  would require the response to carry the enforcement mode, blurring the
  boundary between decision and serialisation. Also would make it harder to
  distinguish dry-run from enforce in metrics/logging.
- A separate `evaluate_dry_run` function — rejected: duplicates the entire
  decision pipeline, violating DRY and risking divergence between modes.

---

### R4. New DecisionVerdict variant: `DryRunDeny`

**Decision**: Add `DryRunDeny` to the `DecisionVerdict` enum:

```rust
pub enum DecisionVerdict {
    Allow,
    Deny,
    DryRunDeny,  // NEW
    Error,
}
```

This flows through `emit_log` and `record_metrics` exactly like `Deny`, but:
- `emit_log`: logs at **WARN** with `decision = "dry_run_deny"` (distinct from
  `"deny"`), carrying the same capacity figures and the violated resource.
- `record_metrics`: records a new `VerdictLabel::DryRunDeny` on the verdict
  counter, so dashboards can query `verdict="dry_run_deny"` separately from
  `verdict="deny"`.

**Rationale**: The verdict must be distinguishable in both logs and metrics
(FR-007, FR-008, SC-004). A dedicated variant is the cleanest way to thread this
through the existing `DecisionSummary` → `emit_log` / `record_metrics` pipeline
without overloading existing variants. The log `decision` field value
`"dry_run_deny"` is self-documenting.

**Alternatives considered**:
- Overloading `Deny` with a `dry_run: bool` flag on `DecisionSummary` —
  rejected: requires every logging/metrics call site to check the flag; easy to
  forget; less type-safe.
- Using `Allow` for dry-run admits and distinguishing only via the `warnings`
  field — rejected: metrics cannot distinguish them; SC-004 requires a
  first-class signal.

---

### R5. New metrics VerdictLabel: `DryRunDeny`

**Decision**: Add `VerdictLabel::DryRunDeny` (serialised as `"dry_run_deny"`)
to the metrics module. Pre-create the new label combinations in `Metrics::new()`
so the series appear at zero from startup (matching the existing pattern).

The `capacity_admission_verdicts_total` counter will now have these label
combinations:
- `{resource="cpu", verdict="allow"}`
- `{resource="cpu", verdict="deny"}`
- `{resource="cpu", verdict="dry_run_deny"}` ← NEW
- `{resource="cpu", verdict="error"}`
- `{resource="memory", verdict="allow"}`
- `{resource="memory", verdict="deny"}`
- `{resource="memory", verdict="dry_run_deny"}` ← NEW
- `{resource="memory", verdict="error"}`

**Rationale**: The pre-creation pattern is already established (every series at
zero from startup). Adding `dry_run_deny` as a fourth verdict value is
back-compatible: existing dashboards querying `verdict="deny"` are unaffected
(the new decisions go to `dry_run_deny`, not `deny`). An operator querying
`verdict=~"deny|dry_run_deny"` gets the combined view.

**Alternatives considered**:
- A separate counter `capacity_admission_dry_run_denies_total` — rejected:
  fragments the metric surface, requires a new metric family, and breaks the
  single-counter-query pattern.
- Adding a `mode` label to the existing counter — rejected: high-cardinality
  risk if more modes are added; a verdict label is more natural (the verdict IS
  the distinguishable outcome).

---

### R6. AdmissionResponse warnings field

**Decision**: Use the `AdmissionResponse.warnings` field (type
`Option<Vec<String>>`), confirmed present in `kube::core::admission::AdmissionResponse`
(kube-rs 4.2.0, line 314 of `kube-core/src/admission.rs`). When dry-run mode
admits an over-budget pod, the warning is the same message that a real rejection
would carry (the `BudgetViolation::message_line()` output for each violated
resource, joined by newlines).

The admission `warnings` field was introduced in Kubernetes 1.19 (AdmissionReview
v1 GA). All versions in the support window (1.34–1.36) are far above this floor.
The apiserver surfaces warnings to the client as `Warning` events in `kubectl`
output and in the cluster event log.

**Rationale**: This is the K8s-native way to surface advisory information on an
admitted request. It does not modify `allowed` or `status.message`, so the
rejection message contract (SC-002) is preserved for real rejections. The
operator sees `Warning: CPU budget exceeded: ...` in `kubectl` output — clear,
actionable, and non-blocking.

**Alternatives considered**:
- Setting `status.message` on an allowed response — rejected: the contract says
  `status` is for denials/errors; an allow with a status is confusing and
  non-standard.
- A mutating webhook that annotates the pod — rejected: violates the
  validating-only constraint (Constitution Principle V, spec assumption).
- Logging only (no warning on the response) — rejected: the operator would have
  to read logs to discover what dry-run blocked; the warnings field surfaces it
  at the point of action.

---

### R7. Enforcement mode in structured logging

**Decision**: Add an `enforcement_mode` field to every structured log entry. The
value is `"enforce"` or `"dry_run"` (lowercase, matching the CRD enum values).
This field is present on all decisions (allow, deny, dry_run_deny, error) so an
operator reviewing logs can immediately tell which mode was active.

Additionally, the `DecisionSummary` struct gains an `enforcement_mode: String`
field (populated from the resolved mode), threaded through `emit_log`.

**Rationale**: FR-009 requires the mode in every log entry. Adding it to
`DecisionSummary` is the natural place — it already carries `budget_percent`,
`freshness_seconds`, etc. The log field name `enforcement_mode` is
self-documenting and distinct from the AdmissionReview's `dryRun` field (which
means the apiserver is dry-running the request, not the webhook).

**Alternatives considered**:
- Logging the mode only on dry-run decisions — rejected: FR-009 requires it on
  all decisions so an operator can confirm enforce mode is active.
- Using the `tracing` span context — rejected: the mode changes per-decision (an
  operator can patch it between requests), so it must be a per-event field, not
  a process-level span.

---

### R8. Allocation controller: auto-created singleton default

**Decision**: The `default_allocation_singleton()` function in
`controllers/allocation.rs` seeds `enforcement_mode:
Some(EnforcementMode::Enforce)` explicitly. The `recompute` function does NOT
read or use `enforcement_mode` — the mode is a webhook concern, not a controller
concern. The controller only reads `budget_percent` (to compute ceilings) and
writes status (allocated figures + ceilings). The enforcement mode lives on the
spec and is read only by the webhook.

**Rationale**: The Allocation Controller's job is to compute allocation figures
and ceilings — it is agnostic to enforcement mode. Adding mode awareness to the
controller would violate the separation of concerns (Principle V). The mode is
simply a field on the CRD spec that the controller creates with a default and
then never touches again.

**Alternatives considered**:
- Having the controller write the current mode to status — rejected: the mode is
  spec (operator-set), not status (controller-computed). Mirroring it to status
  would be redundant and could diverge.
- Having the controller log the mode — rejected: the controller does not make
  enforcement decisions; logging the mode there adds noise.

---

### R9. CRD schema update and backward compatibility

**Decision**: The `enforcementMode` field is added to the Allocation CRD's
OpenAPI schema automatically by the `#[derive(JsonSchema)]` on the enum. The
generated CRD YAML (`deploy/crds.yaml`) must be regenerated and re-applied to
the cluster. Since the field is optional (`Option<EnforcementMode>`), existing
Allocation instances that lack the field remain valid — the apiserver accepts
them, and the webhook treats the absent field as `Enforce`.

The CRD does NOT need a new version (stays `v1`). Adding an optional field to a
spec is a backward-compatible schema evolution in Kubernetes — no conversion
webhook is needed.

**Rationale**: Kubernetes CRD schema evolution allows adding optional fields to
an existing version. The `Option<>` ensures the field is not required, so the
upgrade is non-breaking. The webhook's safe-default resolution (`None` →
`Enforce`) is the runtime safety net.

**Alternatives considered**:
- A new CRD version `v1beta1` — rejected: massive overkill for one optional
  field; would require a conversion webhook, storage version migration, and
  dual-version serving.
- Making the field required with a default — rejected: Kubernetes CRD required
  fields cannot have server-side defaults in `v1` without a defaulting
  (mutating) webhook; `Option<>` with runtime default is simpler and safer.

---

### R10. ValidatingWebhookConfiguration: no change needed

**Decision**: The `ValidatingWebhookConfiguration` does not need any changes for
dry-run mode. The webhook still uses `failurePolicy: Fail` (Principle I), still
processes CREATE/UPDATE on pods, and still returns `allowed: true/false`. The
only new surface is the `warnings` field on allowed responses, which the
apiserver handles natively.

**Rationale**: Dry-run mode is a webhook-internal behavioural toggle, not a
configuration change to the webhook registration. The apiserver does not need to
know the webhook is in dry-run mode — it just sees `allowed: true` with warnings.

**Alternatives considered**:
- A second ValidatingWebhookConfiguration for dry-run — rejected: the mode is
  per-decision (read from the CRD), not per-registration. Two configurations
  would require two webhooks or a complex routing setup.

---

### R11. Test strategy

**Decision**: The test strategy mirrors the existing three-tier approach:

1. **Unit tests** (in-module `#[cfg(test)]`):
   - `EnforcementMode` enum: serialisation round-trip (`enforce`, `dry-run`,
     absent → default).
   - `resolve_enforcement_mode` helper: `None` → `Enforce`, `Some(DryRun)` →
     `DryRun`.
   - `evaluate` function: dry-run mode converts a deny to a dry-run-deny admit
     with warnings; enforce mode unchanged.
   - `emit_log`: dry-run-deny logs at WARN with `decision = "dry_run_deny"` and
     `enforcement_mode` field.
   - `record_metrics`: dry-run-deny increments `verdict="dry_run_deny"`.

2. **Integration tests** (`tests/integration/dry_run.rs`, new file):
   - Dry-run admit of over-budget pod: response has `allowed: true` + warnings.
   - Dry-run fail-closed: stale data still rejects even in dry-run mode.
     - Mode switch at runtime: same `evaluate` call with different mode.
   - Enforce mode unaffected: over-budget pod still rejected.

3. **BDD** (`tests/bdd/features/dry_run.feature`, new file):
   - Scenario: dry-run admits over-budget pod with warning.
   - Scenario: dry-run rejects on stale capacity data.
   - Scenario: enforce mode rejects over-budget pod (no change).

**Rationale**: Principle VIII (test-first) and Principle VI (integration test
coverage) apply. The dry-run feature adds a new decision branch, so it needs its
own unit + integration + BDD coverage. The BDD scenarios are readable by
non-Rust reviewers (the whole point of the Gherkin format).

**Alternatives considered**:
- Extending the existing `budget_enforcement.rs` integration test — rejected:
  the dry-run behaviour is a distinct feature with its own scenarios; mixing it
  into the budget enforcement tests obscures both.

---

### R12. README update obligation

**Decision**: The README must be updated in the same PR (Constitution Principle
X). The additions:
- A new section **"Enforcement Modes (Enforce / Dry-Run)"** under Configuration,
  documenting `spec.enforcementMode`, the two values, the default, and how to
  switch via `kubectl patch`.
- Update the **Allocation CRD** spec table to include `enforcementMode`.
- Update the **Failure Modes** table to note that fail-closed paths reject in
  both modes.
- Update the **Prometheus Metrics** table to include the new `dry_run_deny`
  verdict label value.
- Update the **Structured Logging** table to include the `enforcement_mode`
  field and the `dry_run_deny` decision value.

**Rationale**: Principle X requires every user-facing capability to be
documented in README.md in the same change. Dry-run mode is user-facing
(operators toggle it, see warnings, read metrics).
