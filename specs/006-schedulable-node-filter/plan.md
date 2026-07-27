# Implementation Plan: Schedulable Node Filter

**Branch**: `006-schedulable-node-filter` | **Date**: 2026-07-27 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/006-schedulable-node-filter/spec.md`

## Summary

The Node Capacity Controller currently sums `.status.allocatable` across **all**
cluster nodes — including control-plane masters and cordoned (`spec.unschedulable
= true`) nodes — inflating the capacity pool beyond what kube-scheduler can
actually place workloads on. This plan introduces a two-layer exclusion filter:
(1) default exclusion of unschedulable nodes (the correctness fix), and (2) an
optional Kubernetes `LabelSelector` on the `ClusterCapacity` CRD spec for
arbitrary node-subset exclusion (e.g. control-plane nodes by role label). The
status gains observability fields showing included vs excluded node counts with
a reason breakdown. The change is additive and backward-compatible: existing
deployments with no selector configured gain only the cordon fix.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024), per `Cargo.toml` `rust-version`.

**Primary Dependencies** (existing, no new dependencies):
- `kube = 4.2.0` (runtime, derive, client, rustls-tls, admission) — watcher/reflector, `Api`, CRD derive
- `k8s-openapi = 0.28.0` (features: latest, schemars) — provides `LabelSelector` and `LabelSelectorRequirement` types at `apimachinery::pkg::apis::meta::v1`
- `schemars = 1` — `JsonSchema` derive (already used by both CRDs; `LabelSelector` has its own `JsonSchema` impl from `k8s-openapi`)
- `serde = 1` (derive) — serialization for CRD structs

**Storage**: N/A — state lives in CRDs (ClusterCapacity, Allocation), no external store.

**Testing**: unit tests (`#[test]`), integration tests (`tower-test` mocked apiserver), BDD (`cucumber-rs`), E2E (`kind` on CI across k8s 1.34/1.35/1.36).

**Target Platform**: Linux container, deployed as a Kubernetes `Deployment`.

**Project Type**: library + binary (Kubernetes admission webhook operator).

**Performance Goals**: node filtering adds O(nodes) label-matching per reconciliation — negligible (clusters with thousands of nodes reconcile in <1ms for the filter; the existing allocatable-parsing dominates). No latency budget impact.

**Constraints**: backward-compatible CRD change (additive fields only); no new RBAC permissions needed (already has get/list/watch on nodes).

**Scale/Scope**: small additive feature — modifies 1 CRD struct, 1 controller function, adds 1 pure helper module, updates 1 deploy manifest, 1 README section.

## Constitution Check (Pre-Design)

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default | ✅ PASS | Excluding nodes reduces reported capacity → stricter admission. When all nodes are excluded, capacity drops to zero → webhook fails closed on all admissions (correct: no verifiable capacity). No new fail-open path introduced. |
| II | Capacity as a Hard Budget | ✅ PASS | The budget becomes *accurate* — it now reflects schedulable capacity rather than phantom capacity. This *strengthens* Principle II: the budget was previously over-reported, admitting workloads that couldn't actually be scheduled. |
| III | Explicit Failure Modes | ✅ PASS | Invalid label selector → log + fallback to unschedulable-only exclusion (FR-010). This is a new explicit failure path, documented and testable. No undefined behaviour. |
| IV | Observability Before Optimisation | ✅ PASS | Status gains `excludedNodeCount` + `excludedNodes` breakdown (FR-008, FR-009). Operators can see *why* nodes were excluded (unschedulable vs label-matched) without inspecting metrics. |
| V | Separated Concerns, Minimal Surface | ✅ PASS | Node filtering is supply-side only (Node Capacity Controller). Demand side (Allocation Controller) is unchanged. Taint/toleration matching is deliberately NOT replicated — that's the scheduler's job. The filter uses standard Kubernetes `LabelSelector`, not a custom dialect. |
| VI | Integration Test Coverage | ✅ PASS | New integration tests for cordon exclusion, label-selector exclusion, and the combined filter. BDD scenarios for the three user stories. All added to the existing test infrastructure (`tower-test` mock apiserver + `cucumber-rs`). |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | `LabelSelector` is GA since Kubernetes 1.0 (apimachinery core API). `Node.spec.unschedulable` is GA since 1.0. No version-specific concerns across the 1.34–1.36 window. |
| VIII | Test-First Development | ✅ PASS | TDD applies: write `sum_node_allocatable` filtering tests first (RED), then implement the filter (GREEN). The pure `is_node_counted` helper is unit-testable in isolation. |
| IX | Editor Configuration as Code | ✅ PASS | All new `.rs`, `.yaml`, `.md`, `.feature` files follow `.editorconfig` (LF, 2-space YAML, 4-space Rust). No new file types introduced. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | FR-012 requires README documentation of the exclusion feature: default cordon exclusion, label-selector configuration, and new status fields. Tracked as a task. |
| XI | CI-Green Completion Gate | ✅ PASS | All existing CI jobs (quality + E2E 1.34/1.35/1.36) must pass with the new code. No CI workflow changes needed. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | No scratch files needed for this feature; any transient artifacts go to `.temp/`. |

**Gate result**: PASS — all 12 principles satisfied. No violations to justify.

## Project Structure

### Documentation (this feature)

```text
specs/006-schedulable-node-filter/
├── plan.md              # This file
├── spec.md              # Feature specification (/speckit-specify output)
├── research.md          # Phase 0: research decisions
├── data-model.md        # Phase 1: CRD schema changes, filter algorithm
├── quickstart.md        # Phase 1: validation scenarios
├── contracts/
│   └── clustercapacity-crd.md  # Phase 1: updated CRD contract (delta)
├── checklists/
│   └── requirements.md  # Quality checklist (/speckit-specify output)
└── tasks.md             # Phase 2 output (/speckit-tasks — not created yet)
```

### Source Code (repository root — modified files)

```text
src/
├── crd/
│   └── cluster_capacity.rs    # MODIFIED: ClusterCapacitySpec gains optional nodeSelector field;
│                              #   ClusterCapacityStatus gains excluded node observability fields
├── controllers/
│   ├── node_capacity.rs       # MODIFIED: sum_node_allocatable gains filtering; reconcile reads selector from CRD
│   └── node_filter.rs         # NEW: pure is_node_counted() + LabelSelector evaluation logic
└── resources/
    └── quantity.rs            # UNCHANGED (CPU/memory parsing already here)

deploy/
├── crds.yaml                  # MODIFIED: ClusterCapacity schema gains spec.nodeSelector + status fields
└── rbac.yaml                  # UNCHANGED (already has get/list/watch on nodes)

tests/
├── integration/
│   └── node_filter.rs         # NEW: integration tests for cordon + label-selector exclusion
├── bdd/
│   ├── features/
│   │   └── node_filter.feature # NEW: BDD scenarios for the 3 user stories
│   └── steps/
│       └── node_filter_steps.rs # NEW: step definitions
└── unit/                      # unit tests live inline in each src/ module (#[cfg(test)])

README.md                      # MODIFIED: new "Node Exclusion" section (FR-012)
Cargo.toml                     # MODIFIED: new [[test]] entries for integration + BDD tests
```

**Structure Decision**: The filter logic lives in a new pure module
`src/controllers/node_filter.rs` — separate from `node_capacity.rs` (which
handles the watcher/reflector/patch plumbing) — so the filtering decision
(`is_node_counted`) is independently unit-testable (Principle VIII) and the
controller stays focused on reconciliation. The CRD struct change is a
field-addition to the existing `cluster_capacity.rs`; the deploy manifest is a
schema-property addition to the existing `crds.yaml`. No new binaries, no new
RBAC, no new dependencies.

## Complexity Tracking

> No constitution violations to justify. The design is additive within the
> existing 3-component architecture.

## Constitution Check (Post-Design)

*Re-evaluated against the actual Phase 1 design artifacts below.*

| # | Principle | Status | Post-Design Evidence |
|---|-----------|--------|----------------------|
| I | Fail-Closed by Default | ✅ PASS | Data-model confirms: zero-counted-nodes → zero capacity → fail-closed. Invalid selector fallback (log + unschedulable-only) does not admit under degraded knowledge — it narrows the exclusion to the safe default. |
| II | Capacity as a Hard Budget | ✅ PASS | The supply sum now excludes non-schedulable nodes — the budget denominator shrinks to match reality. Demand side (Allocation Controller) unchanged. |
| III | Explicit Failure Modes | ✅ PASS | Invalid selector → `SelectorParseError` → `warn!` log → fallback to unschedulable-only. Enumerated in data-model.md §Error Paths. Testable. |
| IV | Observability Before Optimisation | ✅ PASS | Status fields `excludedNodeCount` + `excludedByUnschedulable` + `excludedBySelector` give operators full visibility without metrics endpoints. |
| V | Separated Concerns, Minimal Surface | ✅ PASS | The filter is a pure function in its own module; the controller calls it without coupling to watcher internals. No taint replication. Standard `LabelSelector` type. |
| VI | Integration Test Coverage | ✅ PASS | Quickstart maps all 3 user stories to test scenarios: unit (`is_node_counted`), integration (mock-apiserver reconcile with cordon/label events), BDD (`.feature` scenarios). |
| VII–XII | (unchanged from pre-design) | ✅ PASS | No new version/API/dependency/format/doc/ci/scratch concerns introduced by the design artifacts. |

**Post-design gate result**: PASS.
