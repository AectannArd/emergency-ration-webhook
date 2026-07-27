# Data Model — Workload Exclusion Policy

## 1. CRD Spec Change

### Before (current)

```rust
pub struct AllocationSpec {
    pub budget_percent: i32,
    pub enforcement_mode: Option<EnforcementMode>,
}
```

### After (spec-008)

```rust
pub struct AllocationSpec {
    pub budget_percent: i32,
    pub enforcement_mode: Option<EnforcementMode>,

    /// Optional list of namespace names whose pods are exempt from capacity
    /// admission (spec-008, FR-001). A pod whose namespace matches any entry
    /// is admitted without a budget check.
    pub excluded_namespaces: Option<Vec<String>>,

    /// Optional list of priority class names whose pods are exempt from
    /// capacity admission (spec-008, FR-002). A pod whose
    /// `spec.priorityClassName` matches any entry is admitted without a budget
    /// check. String match — the webhook does NOT resolve PriorityClass
    /// resources (R3).
    pub excluded_priority_classes: Option<Vec<String>>,
}
```

**JSON fields**: `excludedNamespaces` and `excludedPriorityClasses` (camelCase)
— arrays of strings, both optional.

```yaml
spec:
  budgetPercent: 80
  enforcementMode: enforce
  excludedNamespaces:
    - kube-system
    - monitoring
  excludedPriorityClasses:
    - system-node-critical
    - system-cluster-critical
```

## 2. Status (unchanged)

The `AllocationStatus` struct is NOT modified. Excluded pods are still counted
in `allocatedCpuMilli` / `allocatedMemoryBytes` — exclusion is admission-only
(R5).

## 3. Decision Path Change

### 3.1 Exemption check — new early-return in `evaluate()`

The exclusion check is inserted in `evaluate()` AFTER the Allocation singleton
is found (so the exclusion config is available) but BEFORE the budget check
(`check_budget`). This means:

- Fail-closed paths that fire BEFORE finding the Allocation (missing allocation,
  missing status) still reject — exclusion config is not available yet.
- Once the Allocation is found, the exemption check runs. If the pod is exempt,
  the function returns an `Exempt` outcome immediately — no freshness check, no
  capacity-data lookup, no budget arithmetic.
- If the pod is NOT exempt, the existing decision path runs unchanged.

```
evaluate(request, alloc_store, cap_store, now, freshness_threshold, webhook_ns):
  1. Find Allocation singleton — if missing, reject (CapacityDataMissing) [unchanged]
  2. Resolve budget_percent + enforcement_mode from spec [unchanged]
  3. Find Allocation status — if missing, reject [unchanged]
  4. >>> NEW: check_exemption(request, allocation.spec, webhook_ns)
     if exempt → return Exempt outcome (allowed: true, no budget check)
  5. Assess freshness — if stale, reject [unchanged]
  6. Find ClusterCapacity status — if missing, reject [unchanged]
  7. Resolve effective request — if parse fails, reject [unchanged]
  8. check_budget → Admit / Deny [unchanged]
```

### 3.2 NEW: `check_exemption` — pure function

```rust
/// The criterion that triggered an exemption, for observability (R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExemptionReason {
    /// Pod namespace is in `excludedNamespaces`.
    Namespace,
    /// Pod priorityClassName is in `excludedPriorityClasses`.
    PriorityClass,
    /// Pod is in the webhook's own namespace (FR-007 bootstrap fallback).
    WebhookNamespace,
}

/// Check whether a pod is exempt from capacity admission.
/// Returns `Some(reason)` if exempt, `None` if subject to the budget.
///
/// Order: webhook namespace (FR-007) → excludedNamespaces → excludedPriorityClasses.
/// The first match wins; subsequent checks are skipped.
pub fn check_exemption(
    pod_namespace: Option<&str>,
    pod_priority_class: Option<&str>,
    spec: &AllocationSpec,
    webhook_namespace: &str,
) -> Option<ExemptionReason>
```

Algorithm:
```
1. if pod_namespace == Some(webhook_namespace) → Some(WebhookNamespace)
2. if let Some(ns_list) = spec.excluded_namespaces:
     if ns_list.contains(&pod_namespace) → Some(Namespace)
3. if let Some(pc_list) = spec.excluded_priority_classes:
     if pc_list.contains(&pod_priority_class) → Some(PriorityClass)
4. None
```

Notes:
- `pod_namespace` comes from `request.namespace` (the AdmissionRequest's
  namespace field, set by the apiserver).
- `pod_priority_class` comes from `request.object.spec.priorityClassName`.
  Absent or empty-string priority class never matches.
- Duplicate entries in the lists are harmless (`Vec::contains` is idempotent).

### 3.3 NEW: `DecisionVerdict::Exempt`

```rust
pub enum DecisionVerdict {
    Allow,
    Deny,
    DryRunDeny,
    Error,
    Exempt,  // spec-008: admitted by exclusion policy, no budget check
}
```

The `Exempt` verdict produces:
- `AdmissionResponse { allowed: true }` (no warnings, no reason message).
- A structured log at INFO level carrying `decision = "exempt"`,
  `exemption_reason = "namespace"|"priority_class"|"webhook_namespace"`.
- An increment of `capacity_admission_exemptions_total{reason=...}`.

### 3.4 `AppState` — gains `webhook_namespace`

The webhook's own namespace (from `--namespace`/`NAMESPACE` config) must be
available in `evaluate()` for the FR-007 bootstrap self-exemption. It is added
to `AppState`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub webhook_namespace: String,
}
```

Threaded from `main.rs` → `AppState::new(...)` → `evaluate()`. The `evaluate`
signature gains a `webhook_namespace: &str` parameter.

## 4. Metrics Change

### 4.1 NEW counter: `capacity_admission_exemptions_total`

```rust
// metrics.rs
exemptions: IntCounterVec  // labels: ["reason"]
```

- Metric name: `capacity_admission_exemptions_total`
- Labels: `reason ∈ {namespace, priority_class, webhook_namespace}`
- Pre-created at zero for all three reasons (same pattern as the existing
  verdict counter).

### 4.2 `record_metrics` — handle Exempt verdict

The `Exempt` verdict does NOT increment `capacity_admission_verdicts_total`
(it is not an allow/deny/error) — it increments the separate exemptions counter.
This keeps the verdict counter semantically clean (budget decisions only) while
the exemptions counter tracks policy bypasses.

Latency IS still observed (the exemption check takes time, and operators want to
know the webhook's response time regardless of outcome).

## 5. CRD YAML Delta (deploy/crds.yaml)

Two new properties under the Allocation `spec.properties`:

```yaml
                excludedNamespaces:
                  type: array
                  nullable: true
                  description: >
                    Optional list of namespace names whose pods are exempt
                    from capacity admission. A pod whose namespace matches
                    any entry is admitted without a budget check.
                  items:
                    type: string
                excludedPriorityClasses:
                  type: array
                  nullable: true
                  description: >
                    Optional list of priority class names whose pods are
                    exempt from capacity admission. Matched against
                    pod.spec.priorityClassName (string match, no resource
                    resolution).
                  items:
                    type: string
```

Neither field is added to the `required` array (both optional, FR-004).

## 6. Webhook Config Delta (deploy/webhook-config.yaml)

The `namespaceSelector` is simplified — the system-namespace list moves to the
CRD; only the webhook's own namespace remains as apiserver-level defence-in-depth:

```yaml
    namespaceSelector:
      matchExpressions:
        # Defence-in-depth: the apiserver filters the webhook's own namespace
        # so it can never self-gate, even during cold start. All other namespace
        # exclusions are CRD-based (spec-008, dynamic).
        - key: kubernetes.io/metadata.name
          operator: NotIn
          values: ["capacity-admission"]
```

## 7. Allocation Controller Singleton Seeding

`default_allocation_singleton()` seeds the two new fields as `None` (no
exclusions by default — backward-compatible). The controller never touches these
fields afterwards, same as `enforcement_mode`. An operator patches them in.

## 8. Test Matrix

| Test | Type | What it proves |
|------|------|----------------|
| `check_exemption` — webhook namespace match | unit | pod in webhook ns → Exempt(WebhookNamespace) |
| `check_exemption` — namespace list match | unit | pod ns in list → Exempt(Namespace) |
| `check_exemption` — priority class match | unit | pod pc in list → Exempt(PriorityClass) |
| `check_exemption` — no match | unit | pod matches neither → None |
| `check_exemption` — OR semantics | unit | ns match OR pc match → first-match wins |
| `check_exemption` — empty lists | unit | None/empty vec → None (no exemption) |
| `check_exemption` — absent priority class | unit | pod with no pc → never PriorityClass match |
| `check_exemption` — duplicate entries | unit | `["a","a"]` → single match, no error |
| `evaluate` — exempt pod skips budget check | integration | over-budget pod in excluded ns → allowed |
| `evaluate` — non-exempt pod still budget-checked | integration | over-budget pod in normal ns → denied |
| `evaluate` — cold-start webhook ns exemption | integration | webhook ns exempt even with empty CRD cache |
| `AllocationSpec` — new fields serialise camelCase | unit | `excludedNamespaces`/`excludedPriorityClasses` round-trip |
| `AllocationSpec` — absent fields default to None | unit | pre-spec-008 CRD deserialises cleanly |
| CRD schema — fields optional, not in required | unit | neither field in `spec.required` |
| Metrics — exemptions counter increments | unit | exempt decision → counter +1 with correct reason |
| Metrics — exemptions counter pre-created at zero | unit | all 3 reason labels visible at startup |
| US1: namespace exclusion end-to-end | BDD | pod in excluded ns admitted at full budget |
| US2: priority class exclusion end-to-end | BDD | pod with excluded pc admitted at full budget |
| US3: combined OR semantics | BDD | ns-only match, pc-only match, both, neither |
