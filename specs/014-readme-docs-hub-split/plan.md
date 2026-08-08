# Implementation Plan: README Documentation Hub Split

**Branch**: `014-readme-docs-hub-split` | **Date**: 2026-08-08 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/014-readme-docs-hub-split/spec.md`

## Summary

Split the 1176-line monolithic README into a navigational hub (project description
+ quick-start + TOC + per-capability summaries + License) with all detailed
reference content extracted into 11 focused articles under `docs/`. This
operationalizes Constitution Principle X (v2.9.0): README as Documentation Hub
with docs/ Articles. The split is purely structural — zero information loss,
verified by a content-trace mapping. No production source code is touched.

## Technical Context

**Language/Version**: N/A — documentation-only spec (Markdown files only)

**Primary Dependencies**: N/A — no code dependencies

**Storage**: Git-tracked Markdown files (`README.md`, `docs/*.md`)

**Testing**: Accuracy validation — every documented value (CRD field names, metric
names, flag names, env var names, default values, exit codes, scenario IDs)
cross-checked against source files (`src/config.rs`, `src/crd/*.rs`,
`src/metrics.rs`, `src/webhook/error.rs`, `src/bin/erw-verify/args.rs`). Link
validation — every `docs/` article reachable from README TOC; every relative link
resolves. The editorconfig CI job validates formatting compliance of all new
Markdown files.

**Target Platform**: GitHub-rendered Markdown (primary), local editor preview,
plain-text viewers. All links must be relative (no absolute GitHub URLs).

**Project Type**: Documentation reorganization

**Performance Goals**: README reduced to < 250 lines (from 1176). Each docs/
article focused on a single topic, scannable in one sitting.

**Constraints**: Zero information loss (content-trace verified). All in-repo
cross-references updated. `.editorconfig` compliance (LF line endings, 4-space
indent for lists inside Markdown — actually, .editorconfig uses `indent_size = 2`
for Markdown). No content changes — pure reorganization (with FR-012 accuracy
corrections if discrepancies are found).

**Scale/Scope**: 1176 lines → ~11 articles + slim README. ~10 cross-references
to update (README internal links, ARTIFACTS.md, specs/006 quickstart, CONTRIBUTING
cross-links).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Evidence |
|-----------|--------|----------|
| I. Fail-Closed by Default | ✅ PASS | Documentation-only; no code paths affected. The fail-closed behaviour is documented in the docs/ articles with identical content. |
| II. Capacity as a Hard Budget | ✅ PASS | Budget semantics documented verbatim in docs/configuration.md. |
| III. Explicit Failure Mode Configuration | ✅ PASS | Failure-mode table moves to docs/failure-modes.md unchanged. |
| IV. Observability Before Optimisation | ✅ PASS | Metrics/log reference moves to docs/observability.md unchanged. |
| V. Separated Concerns, Minimal Surface | ✅ PASS | The split itself separates concerns — each docs/ article covers one topic. |
| VI. Integration Test Coverage | ✅ PASS | N/A — no code changes; CI editorconfig job validates new files. |
| VII. Kubernetes Version Support Window | ✅ PASS | K8s compatibility content moves to docs/kubernetes-compatibility.md unchanged. |
| VIII. Test-First Development | ✅ PASS | Adapted for docs: the content-trace mapping + accuracy rules (data-model.md VR-001..VR-NNN) are the "failing tests" — written before the split. The implementing agent validates against them. |
| IX. Editor Configuration as Code | ✅ PASS | All new docs/*.md files must comply with .editorconfig. CI editorconfig job enforces this. |
| **X. README as Documentation Hub with docs/ Articles** | ✅ PASS | **This spec IS the operationalization of Principle X.** The plan implements exactly what the principle mandates: README = hub (description + quick-start + TOC + summaries), docs/ = detail articles. |
| XI. CI-Green Completion Gate | ✅ PASS | The editorconfig CI job must pass on the new files. No Rust jobs are affected (no .rs changes). |
| XII. Scratch Space for Agent Intercommunication | ✅ PASS | Content-trace work product goes in the spec directory (tracked), not .temp/. |
| XIII. Separation of Usage and Contribution Documentation | ✅ PASS | README's "Development" section (Build/Test/Quality/Project Structure) duplicates CONTRIBUTING.md — removed from README, linked to CONTRIBUTING.md. The docs/ split respects the usage/contribution boundary: all docs/ articles are usage (operator-facing). |
| XIV. Artifact Inventory | ✅ PASS | ARTIFACTS.md unchanged. One cross-reference (`../README.md#verification-scenarios`) is updated to point to the new docs/ location. |
| XV. Build and Publish Procedure for Every Docker Artifact | ✅ PASS | N/A — no build/publish changes. |

## Project Structure

### Documentation (this feature)

```text
specs/014-readme-docs-hub-split/
├── plan.md              # This file
├── research.md          # Article grouping decisions, naming, cross-ref strategy
├── data-model.md        # Content-trace mapping + accuracy verification rules
├── quickstart.md        # Validation scenarios (link checks, accuracy checks)
├── contracts/
│   └── docs-structure.md  # The docs/ article structure contract
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Repository file changes

```text
docs/                           # NEW directory
├── README.md                   # NEW — docs/ index (one-line description per article)
├── deployment.md               # NEW — published image, build, deploy, TLS, verify
├── configuration.md            # NEW — CLI/env, CRD reference, budget adjustment, edge cases
├── node-exclusion.md           # NEW — node exclusion (spec-006/007)
├── enforcement-modes.md        # NEW — enforce/dry-run (spec-004)
├── workload-exclusion.md       # NEW — namespace/priority exclusion (spec-008)
├── observability.md            # NEW — HTTP endpoints, Prometheus, structured logs, rejection msgs
├── failure-modes.md            # NEW — failure mode table, self-admission/bootstrap
├── kubernetes-compatibility.md # NEW — K8s version support window
├── architecture.md             # NEW — 3-component operator architecture
├── erw-verify.md               # NEW — verification tool reference
└── equalizer.md                # NEW — multi-cluster capacity equalizer (spec-013)

README.md                       # MODIFIED — slimmed to hub (description + quick-start + TOC + summaries + License)
CONTRIBUTING.md                 # MODIFIED — add "Documentation Structure" subsection
ARTIFACTS.md                    # MODIFIED — fix broken #verification-scenarios anchor link
```

**Structure Decision**: 11 topic-focused articles under `docs/`, each covering a
single major capability. Articles are grouped by audience journey: deployment
first (getting started), then configuration (day-to-day tuning), then operational
reference (observability, failure modes), then architecture/background, then
tooling, then the equalizer (separate component). The `docs/README.md` index
provides a local TOC. The repo-root README retains the quick-start (self-contained
deploy+verify) and adds a TOC linking to every article with 1-3 sentence summaries.

The README "Development" section (Build, Tests, Quality Gate, Project Structure,
lines 1037-1108) is contributor content per Principle XIII and already exists in
CONTRIBUTING.md. It is **removed** from the README and replaced with a single link
to CONTRIBUTING.md — it does NOT move to docs/ (docs/ is for operator-facing
usage content, not contributor content).

## Complexity Tracking

No constitution violations to justify. The plan is a pure documentation
reorganization with no architectural complexity.

## Constitution Check (Post-Design)

*Re-evaluate every principle against the actual design artifacts.*

| Principle | Status | Evidence |
|-----------|--------|----------|
| I–IX | ✅ PASS | Unchanged from pre-design check — documentation only. |
| **X. README as Documentation Hub** | ✅ PASS | The design artifacts implement the hub model precisely: data-model.md maps every old section to README-hub or docs/article; contracts/docs-structure.md defines the article structure; quickstart.md VS-17 enforces that README retains only hub sections. The "Development" section removal (to CONTRIBUTING.md) respects the Principle XIII boundary. |
| XI–XV | ✅ PASS | Unchanged — CI editorconfig job covers new files; no build/publish changes. |

All principles pass post-design. No gate failures.
