# Implementation Plan: S10 Degradation Restore Fix

**Branch**: `010-s10-degradation-restore-fix` | **Date**: 2026-07-28 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/010-s10-degradation-restore-fix/spec.md`

## Summary

Fix the S10 degradation scenario failure where `restore_readiness` returns
before the Kubernetes Service Endpoints are populated, causing S10 to hit an
unreachable rejection (S9's failure mode) instead of the intended
capacity-data-missing rejection. The fix adds a Service-Endpoints readiness
check to the restore phase: after pods are Ready and the ceiling is non-zero,
the restore waits for the Service to have at least one ready endpoint and
confirms the webhook is reachable.

## Technical Context

**Language/Version**: Rust (edition 2024, MSRV 1.89 — same as the existing crate).

**Primary Dependencies**: no new dependencies. Uses the existing `kube` client
API to read `Endpoints`/`EndpointSlice` objects and to make a probe request to
the webhook's `/healthz` endpoint.

**Storage**: N/A.

**Testing**: the fix is integration-tested by running the tool against a real
cluster (S10 must pass on the first run). The classification helpers are pure
functions that are unit-tested. The restore logic is exercised by the
degradation suite itself.

**Target Platform**: same as spec-005 (operator's machine).

**Project Type**: bugfix to the existing `erw-verify` binary.

**Performance Goals**: the restore phase should complete within the existing
`RESTORE_TIMEOUT` (60 seconds). Adding an Endpoints check may add 5-15 seconds
of propagation delay; this fits within the existing budget.

**Constraints**:
- **Confined to `degradation.rs`**: the fix modifies only the restore/readiness
  helpers in `src/bin/erw-verify/scenarios/degradation.rs`. No production code
  changes.
- **No new dependencies**: uses existing kube-rs APIs.

**Scale/Scope**: ~30-50 lines changed in one file. Small, focused fix.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The fix improves the test's ability to verify the fail-closed guarantee on real infrastructure. It does not alter the webhook's fail-closed behaviour. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | No change to budget semantics. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The fix makes the degradation scenarios correctly distinguish between failure modes (unreachable vs capacity-data-missing vs stale). This directly supports Principle III's goal of enumerable, testable failure paths. |
| IV | Observability Before Optimisation | ✅ PASS | The restore phase logs each readiness dimension (pods Ready, endpoints populated, ceiling non-zero, webhook reachable). |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | The fix is confined to the verify binary's degradation module. No new dependencies. |
| VI | Integration Test Coverage | ✅ PASS | The fix improves the integration test that covers the CapacityDataMissing fail-closed path. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | Uses `Endpoints` (core/v1) — available across all supported versions. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | The updated classification helpers are unit-tested first. The restore fix is verified by running the degradation suite against a real cluster. |
| IX | Editor Configuration as Code | ✅ PASS | Modified file follows `.editorconfig`. |
| X | User-Facing Functionality is Documented in README.md | ✅ PASS | The degradation scenario list in README is unchanged (the fix makes S10 pass, not changes its description). |
| XI | CI-Green Completion Gate | ✅ PASS | Unit tests for classification helpers run in CI. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | N/A for this fix. |

## Project Structure

### Documentation (this feature)

```text
specs/010-s10-degradation-restore-fix/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
└── tasks.md             # Phase 2 output (not created by /speckit-plan)
```

### Source Code (repository root)

```text
src/bin/erw-verify/scenarios/
├── degradation.rs       # MODIFIED: add endpoints readiness to restore_readiness
│                        #   + improve S10 probe classification
└── ...                  # (unchanged: enforcement.rs, mod.rs)
```

**Structure Decision**: the fix is a surgical modification to
`degradation.rs` — the `restore_readiness` function (and its internal
`wait_for_readiness` helper) gains a Service-Endpoints check. No other files
are touched.

## Complexity Tracking

No constitution violations to justify.
