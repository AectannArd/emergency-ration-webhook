# Implementation Plan: README Documentation

**Branch**: `spec/readme-documentation` | **Date**: 2026-07-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/002-readme-documentation/spec.md`

## Summary

Replace the 4-line `README.md` stub with comprehensive operator documentation
that satisfies Constitution Principle X. The README is the single entry point
for all user-facing functionality: installation/quick start, configuration
reference (7 CLI flags, 2 CRDs, 3 HTTP endpoints), operations guide (7
Prometheus metrics, structured logging, fail-closed model), and the Kubernetes
version support window. No new code is written — the deliverable is a Markdown
file whose every documented value (flag name, default, metric name, CRD field,
port) is derived from and verified against the shipped implementation on
`main`.

## Technical Context

**Language/Version**: Markdown (GitHub Flavoured). No source code changes.

**Primary Dependencies**: None. The README is plain Markdown with standard
GitHub rendering (tables, code blocks, badges). No static-site generator,
no documentation framework — just `README.md`.

**Storage**: N/A — `README.md` at the repository root.

**Testing**: Accuracy validation against source. Every documented value is
cross-checked against the implementation:
- CLI flags → `src/config.rs` (the `Config` struct + `resolve` calls)
- Defaults → `impl Default for Config` in `src/config.rs`
- CRD fields → `src/crd/allocation.rs`, `src/crd/cluster_capacity.rs`
- Metrics → `src/metrics.rs` (the `Metrics::new` registrations)
- Endpoints/ports → `src/main.rs` + `deploy/deployment.yaml`
- Manifest references → `deploy/*.yaml`
- Rejection format → `src/webhook/error.rs` + `src/webhook/admission.rs`

**Target Platform**: GitHub README renderer (standard GFM). Must render
correctly on github.com without external tooling.

**Project Type**: Documentation deliverable (single file: `README.md`).

**Performance Goals**: N/A — documentation has no runtime.

**Constraints**:
- Accuracy is the hard constraint (FR-012): every documented value MUST match
  the shipped code. Inaccurate documentation is a defect, not a cosmetic issue
  (Principle X).
- No host-specific paths (repo portability rule from Technology Constraints).
- The README is the single entry point (FR-011): it MUST cover the essentials;
  deeper material MAY be linked but not delegated.

**Scale/Scope**: A single `README.md` covering ~7 sections. Estimated
300–500 lines of Markdown. No new tests, no new code, no new dependencies.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The README MUST document the fail-closed model accurately — every degradation path rejects. This documentation does not change the behaviour; it describes what already exists. Documenting it incorrectly would be a Principle X defect, not a Principle I violation. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | The README MUST document the budget as a hard ceiling (inclusive). No new budget logic. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The README MUST document every enumerated failure path. The failure model is already explicit in the code; the README surfaces it. |
| IV | Observability Before Optimisation | ✅ PASS | The README MUST document all 7 metrics, the structured log format, and the rejection message format. This is documentation of existing observability, not new instrumentation. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | The README MUST describe the 3-component architecture. No new components. The README itself adds no complexity to the webhook's surface. |
| VI | Integration Test Coverage | ✅ PASS | Not directly applicable — this is a documentation change. The README's quickstart section references the existing test suite. No new test types introduced. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | The README MUST document the N-2 window and reference the CI version matrix. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | Adapted for documentation: the "test" is the accuracy checklist (every documented value verified against source). The quickstart.md artifact serves as the validation spec — it is written BEFORE the README, defining what must be covered. The README is then written to pass it. |
| IX | Editor Configuration as Code | ✅ PASS | `README.md` is governed by the `.editorconfig` `*.md` section. Markdown formatting (line endings, final newline) must comply. |
| X | User-Facing Functionality is Documented in README.md | ✅ PASS | This principle IS the feature. The README backfill directly satisfies it. Every user-facing capability (flags, env vars, CRD fields, metrics, endpoints, admission behaviour, deployment, version support) is documented. |

## Project Structure

### Documentation (this feature)

```text
specs/002-readme-documentation/
├── plan.md              # This file
├── research.md          # Phase 0 — README best practices + authoritative surface
├── data-model.md        # Phase 1 — README section structure + reference tables
├── quickstart.md        # Phase 1 — README accuracy validation guide
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
README.md                # THE DELIVERABLE — replaces the 4-line stub

# Reference sources (READ-ONLY — the README is derived from these):
src/config.rs            # CLI flags, env vars, defaults
src/metrics.rs           # 7 Prometheus metric names, types, labels
src/crd/allocation.rs    # Allocation CRD spec + status fields
src/crd/cluster_capacity.rs  # ClusterCapacity CRD status fields
src/main.rs              # Endpoints, ports, startup flow
src/webhook/admission.rs # Budget check logic
src/webhook/error.rs     # Rejection message format
deploy/*.yaml            # Kubernetes manifests referenced by quick start
```

**Structure Decision**: Single-file deliverable (`README.md`). No new
directories, no new source files, no new tests. The plan artifacts
(research.md, data-model.md, quickstart.md) live under `specs/002-readme-documentation/`
and serve as the design trail for the README's structure and accuracy
validation.

## Complexity Tracking

> No constitution violations requiring justification. This is a pure
> documentation change that satisfies Principle X — it adds no new code,
> components, or complexity to the webhook itself.

---

## Constitution Check (Post-Design)

*Re-evaluated after Phase 1 design artifacts (research.md, data-model.md,
quickstart.md) were produced.*

| # | Principle | Post-Design Status | Notes |
|---|-----------|-------------------|-------|
| I | Fail-Closed by Default | ✅ CONFIRMED | research.md §R9 enumerates every fail-closed path; data-model.md §6 reproduces the failure-mode table the README must document. The README describes existing behaviour — it cannot weaken it. |
| II | Capacity as a Hard Budget | ✅ CONFIRMED | research.md §R6 + data-model.md §3 lock the budget formula (`floor(total × percent / 100)`, inclusive ceiling) to match `admission.rs`. |
| III | Explicit Failure Mode Configuration | ✅ CONFIRMED | data-model.md §6 has all 6 failure paths, each mapped to a reject outcome. |
| IV | Observability Before Optimisation | ✅ CONFIRMED | research.md §R5 + data-model.md §5 define all 7 metrics with exact names/types/labels. The README must reproduce them. |
| V | Separated Concerns, Minimal Surface | ✅ CONFIRMED | data-model.md §1 includes a brief Architecture section describing the 3-component model, linking to specs/001 for detail. No new components. |
| VI | Integration Test Coverage | ✅ CONFIRMED | quickstart.md defines 5 validation scenarios that exercise the README's accuracy end-to-end against source. |
| VII | N-2 Support Window | ✅ CONFIRMED | research.md §R8 locks the version matrix (1.34, 1.35, 1.36); data-model.md references it. |
| VIII | Test-First Development | ✅ CONFIRMED | quickstart.md (the validation spec) was written BEFORE the README. It defines 8 verification rules (VR-001–008) the README must pass — the documentation equivalent of a failing test written first. |
| IX | Editor Configuration as Code | ✅ CONFIRMED | README.md is Markdown; the `.editorconfig` `*.md` section governs its formatting. |
| X | User-Facing Functionality is Documented | ✅ CONFIRMED | data-model.md §1 defines a section tree covering every user-facing surface: 7 flags, 2 CRDs, 3 endpoints, 7 metrics, 6 failure modes, deployment, version window. FR-011 (single entry point) satisfied by the README being the single deliverable. |

**Gate result**: PASS — no violations. Design advances to Phase 2 (tasks).
