# Implementation Plan: Multi-Selector Node Exclusion

**Branch**: `007-multi-selector-exclusion` | **Date**: 2026-07-27 | **Spec**: [spec.md](./spec.md)

## Summary

Spec-006's single `LabelSelector` field ANDs all requirements, so it cannot
express OR across different label keys. This feature replaces
`spec.nodeSelector: Option<LabelSelector>` with
`spec.nodeSelectors: Option<Vec<LabelSelector>>` — a list of selectors where a
node is excluded if it matches ANY one. Each selector internally ANDs its own
matchLabels/matchExpressions; the OR is at the list level. The change is a clean
rename (spec-006 was just merged, no production deployments to migrate).

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024), per `Cargo.toml` `rust-version`.

**Primary Dependencies**: unchanged — `kube 4.2.0`, `k8s-openapi 0.28.0`
(provides `LabelSelector`), `schemars 1`, `serde 1`, `thiserror 2`. No new crates.

**Testing**: unit (`#[test]`), integration (`tower-test`), BDD (`cucumber-rs`), E2E (`kind` CI).

**Project Type**: library + binary (Kubernetes admission webhook operator).

**Constraints**: CRD schema migration (rename field, additive since the singular field was just added in spec-006); no new RBAC; no new deps.

## Constitution Check (Pre-Design)

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default | ✅ PASS | Multi-selector exclusion reduces capacity → stricter admission. No new fail-open path. |
| II | Capacity as a Hard Budget | ✅ PASS | More accurate exclusion → more accurate budget denominator. |
| III | Explicit Failure Modes | ✅ PASS | Invalid selector → skip + warn (same as spec-006); all-invalid → unschedulable-only fallback. |
| IV | Observability | ✅ PASS | `excludedBySelector` count preserved; node matching multiple selectors counted once. |
| V | Separated Concerns | ✅ PASS | Pure filter extension; no taint replication; standard LabelSelector semantics. |
| VI | Integration Test Coverage | ✅ PASS | New unit/integration/BDD tests for multi-selector OR logic. |
| VII | K8s N-2 | ✅ PASS | LabelSelector GA since k8s 1.0. No version concerns. |
| VIII | Test-First | ✅ PASS | TDD: write multi-selector tests first, then implement. |
| IX | EditorConfig | ✅ PASS | Standard file types, `.editorconfig` covers them. |
| X | README Documentation | ✅ PASS | FR-011 requires README update with OR semantics + migration + examples. |
| XI | CI-Green Gate | ✅ PASS | All existing CI jobs must pass. |
| XII | Scratch Space | ✅ PASS | No scratch files needed. |

## Project Structure

```text
specs/007-multi-selector-exclusion/
├── plan.md
├── spec.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── clustercapacity-crd.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Modified source files

```text
src/
├── crd/
│   └── cluster_capacity.rs    # RENAME node_selector → node_selectors: Option<Vec<LabelSelector>>
├── controllers/
│   ├── node_filter.rs         # ADD labels_match_any_selector(); MODIFY is_node_counted for Vec
│   └── node_capacity.rs       # RENAME read_selector → read_selectors; MODIFY sum_node_allocatable signature
└── ...

deploy/
└── crds.yaml                  # RENAME nodeSelector → nodeSelectors (array)

README.md                      # UPDATE multi-selector section
Cargo.toml                     # ADD [[test]] entries (if new test files)
tests/                         # EXTEND existing node_filter tests for multi-selector
```

**Structure Decision**: this is a delta on spec-006's files. No new modules — the
filter logic extends `node_filter.rs`, the CRD struct changes in
`cluster_capacity.rs`, the controller plumbing changes in `node_capacity.rs`.
Existing tests are extended, not replaced.

## Complexity Tracking

No constitution violations. The change is a field rename + list iteration — no
new architectural surface.

## Constitution Check (Post-Design)

| # | Principle | Status | Post-Design Evidence |
|---|-----------|--------|----------------------|
| I–XII | (unchanged) | ✅ PASS | Data-model confirms: OR semantics via `labels_match_any_selector`; invalid-selector skip; no double-count; status fields preserved. |
