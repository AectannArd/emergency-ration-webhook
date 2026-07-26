# Implementation Plan: Controller Singleton Autocreation

**Branch**: `spec/controller-singleton-autocreation` | **Date**: 2026-07-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/003-controller-singleton-autocreation/spec.md`

## Summary

Fix a production bug where the Node Capacity Controller and Allocation Controller
assume their singleton CRD instances already exist before patching status. When
the instances are absent, `patch_status` returns 404 NotFound and the status is
never written, leaving the webhook with no capacity data (it then rejects all
pods per Principle I). The fix adds an `ensure_singleton` get-or-create step to
each controller: the Node Capacity Controller creates `cluster-capacity` with an
empty spec; the Allocation Controller creates `cluster-allocation` with a default
`budgetPercent` of 80. Both are idempotent and never overwrite an existing
instance. The CI workaround (manual ClusterCapacity creation) is removed, and
the contracts + README are updated to document the autocreation responsibility.

## Technical Context

**Language/Version**: Rust (edition 2024, MSRV 1.89).

**Primary Dependencies**: `kube` 4.2.0 (Api, Patch, PostParams), existing
project dependencies. No new crates.

**Storage**: Kubernetes CRDs (ClusterCapacity, Allocation) — singleton instances
auto-created by controllers.

**Testing**: Unit tests via `#[test]` (TDD per Principle VIII). The
`ensure_singleton` logic is testable with a mocked `Api` or by extracting the
create-or-skip decision into a pure function. E2E verification via CI (the
existing smoke test proves the controllers populate status without manual
intervention).

**Target Platform**: Linux container, Kubernetes workload.

**Project Type**: Bugfix to existing controller code. No new modules or files.

**Constraints**:
- Idempotent: creating when AlreadyExists (409) is success, not an error.
- Non-destructive: never overwrite an existing instance's spec.
- TDD: tests written first, watched to fail, then implemented.

## Constitution Check

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default | ✅ PASS | The bug causes fail-closed on ALL pods (no capacity data). The fix enables the webhook to function by ensuring data is available. It does not change the fail-closed behaviour itself. |
| II | Capacity as a Hard Budget | ✅ PASS | The default budgetPercent=80 is a safe default. Operators can override. The budget logic itself is unchanged. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The 404 NotFound on patch_status was an unhandled failure path — the fix handles it (re-create + retry) rather than logging-and-ignoring. |
| IV | Observability Before Optimisation | ✅ PASS | The `ensure_singleton` logs (info) when it creates an instance and (debug) when the instance already exists. |
| V | Separated Concerns, Minimal Surface | ✅ PASS | Each controller owns its singleton lifecycle. No new components, no cross-controller coupling. |
| VI | Integration Test Coverage | ✅ PASS | E2E CI exercises the autocreation path (fresh cluster, no manual singletons). Unit tests cover the create-vs-skip logic. |
| VII | N-2 Support Window | ✅ PASS | Uses the same kube API surface (get, create, patch_status). No new API versions. |
| VIII | Test-First Development | ✅ PASS | TDD: write ensure_singleton test first, watch it fail, implement. |
| IX | Editor Configuration as Code | ✅ PASS | Rust files governed by rustfmt + .editorconfig. |
| X | User-Facing Functionality Documented | ✅ PASS | US3 updates the README and contracts to document the autocreation. |
| XI | CI-Green Completion Gate | ✅ PASS | The fix must make E2E CI pass. The CI workaround is removed as part of the fix. |

## Project Structure

### Documentation (this feature)

```text
specs/003-controller-singleton-autocreation/
├── plan.md              # This file
├── research.md          # Singleton lifecycle patterns in kube-rs
├── data-model.md        # ensure_singleton function signature + state diagram
├── quickstart.md        # Validation: fresh-cluster deployment without manual singletons
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (changes to existing files)

```text
src/controllers/
├── node_capacity.rs     # Add ensure_singleton + call before patch_status
└── allocation.rs        # Add ensure_singleton + call before recompute

.github/workflows/
└── ci.yml               # Remove manual ClusterCapacity creation workaround

specs/001-capacity-admission-webhook/contracts/
├── clustercapacity-crd.md  # Document autocreation responsibility
└── allocation-crd.md       # Document autocreation responsibility

README.md                  # Remove manual singleton-creation from quick start
```

## Complexity Tracking

> No constitution violations. The fix adds a get-or-create step to two
> existing functions — no new components, modules, or dependencies.
