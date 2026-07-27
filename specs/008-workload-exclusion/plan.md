# Implementation Plan: Workload Exclusion Policy

**Branch**: `008-workload-exclusion` | **Date**: 2026-07-27 | **Spec**: [spec.md](./spec.md)

## Summary

The webhook's own namespace is currently excluded via a static `namespaceSelector`
in the `ValidatingWebhookConfiguration` manifest, plus a `--namespace`/`NAMESPACE`
CLI/ENV var used only for logging. This feature moves the exclusion policy into
the **Allocation CRD** `spec`, introducing two new optional fields:
`excludedNamespaces: []string` and `excludedPriorityClasses: []string`. A pod
matching EITHER list (OR semantics) is admitted without a budget check. The
webhook reads these from its existing Allocation reflector cache — zero extra
API calls on the hot path. The webhook's own namespace remains excluded by the
`namespaceSelector` as defence-in-depth during cold start, and by the CRD at
runtime. Excluded pods are still counted in allocation accounting — exclusion
is an admission-gate bypass only, not an accounting exclusion.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024), per `Cargo.toml` `rust-version`.

**Primary Dependencies**: unchanged — `kube 4.2.0`, `k8s-openapi 0.28.0`,
`schemars 1`, `serde 1`, `prometheus 0.14`. No new crates.

**Testing**: unit (`#[test]`), integration (`tower-test`), BDD (`cucumber-rs`),
E2E (`kind` CI).

**Project Type**: library + binary (Kubernetes admission webhook operator).

**Constraints**: additive CRD schema change (two new optional fields, no field
renames); no new RBAC; no new deps; admission hot path stays reflector-only.

## Constitution Check (Pre-Design)

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default | ✅ PASS | Exclusion is an explicit, operator-configured exception with a recorded justification (the exclusion list in the CRD). Fail-closed paths (stale/missing data, timeout, panic) still reject regardless of exclusion config. Exemption is only checked AFTER the Allocation cache is found and is itself a positive, auditable decision. |
| II | Capacity as a Hard Budget | ✅ PASS | The budget remains hard for non-excluded workloads. Excluded workloads bypass the gate by explicit operator decision — this is the "narrow, explicitly-configured exception with a recorded justification" from Principle III, not a softening of the budget. Excluded pods are still counted in allocation accounting so their consumption is visible. |
| III | Explicit Failure Modes | ✅ PASS | Exemption is a declared, testable outcome with a new `Exempt` verdict in the decision tree. The AdmissionError/DecisionOutcome taxonomy is extended, not bypassed. No new "undefined" category. |
| IV | Observability | ✅ PASS | FR-008 requires a structured log entry + Prometheus counter for every exemption, carrying the namespace/priority class that triggered it. |
| V | Separated Concerns | ✅ PASS | Exclusion policy lives on the Allocation CRD (admission policy singleton) alongside `budgetPercent` and `enforcementMode`. The Allocation Controller does NOT change — it still counts all pods. Only the webhook's admission decision path changes. |
| VI | Integration Test Coverage | ✅ PASS | New unit/integration/BDD tests for namespace exclusion, priority class exclusion, and combined OR semantics. |
| VII | K8s N-2 | ✅ PASS | `pod.spec.priorityClassName` and `request.namespace` are stable since k8s 1.0. No version concerns. |
| VIII | Test-First | ✅ PASS | TDD: write exclusion tests first, then implement. |
| IX | EditorConfig | ✅ PASS | Standard file types, `.editorconfig` covers them. |
| X | README Documentation | ✅ PASS | README must be updated with the new CRD fields, examples, and the exclusion semantics. |
| XI | CI-Green Gate | ✅ PASS | All existing CI jobs must pass. |
| XII | Scratch Space | ✅ PASS | No scratch files needed. |

## Project Structure

```text
specs/008-workload-exclusion/
├── plan.md
├── spec.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── admission-exclusion.md
├── checklists/
│   └── requirements.md
└── tasks.md                # Phase 2 output (NOT created by /speckit-plan)
```

### Modified source files

```text
src/
├── config.rs                   # UNCHANGED (--namespace/NAMESPACE retained for bootstrap)
├── crd/
│   ├── allocation.rs           # ADD excluded_namespaces, excluded_priority_classes to AllocationSpec
│   └── mod.rs                  # RE-EXPORT ExclusionPolicy helper if extracted
├── webhook/
│   ├── handler.rs              # ADD exemption check in evaluate(); ADD Exempt verdict; ADD logging/metrics
│   ├── admission.rs            # UNCHANGED (budget arithmetic — no exclusion concern)
│   └── error.rs                # ADD AdmissionError variant if needed for excluded-pod logging
└── metrics.rs                  # ADD exemption counter (capacity_admission_exemptions_total)

deploy/
├── crds.yaml                   # ADD excludedNamespaces, excludedPriorityClasses to Allocation schema
└── webhook-config.yaml         # SIMPLIFY namespaceSelector (keep only webhook's own ns)

README.md                       # UPDATE exclusion config section
```

**Structure Decision**: this is an additive change to the Allocation CRD spec
and the webhook's admission decision path. No new modules — the exclusion
check is a new early-return branch in `evaluate()`, the CRD struct gains two
optional fields. The Allocation Controller is deliberately untouched (it counts
all pods regardless of exclusion — exclusion is admission-only).

## Complexity Tracking

No constitution violations. The change adds two optional string-list fields to a
CRD and one new decision branch — no new architectural surface.

## Constitution Check (Post-Design)

| # | Principle | Status | Post-Design Evidence |
|---|-----------|--------|----------------------|
| I–XII | (unchanged) | ✅ PASS | Data-model confirms: exemption checked after Allocation cache is found; excluded pods still counted by Allocation Controller; new Exempt verdict is observable; webhook namespace retained as defence-in-depth. |
