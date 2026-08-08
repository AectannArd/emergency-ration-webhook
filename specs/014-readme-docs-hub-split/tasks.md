# Tasks: README Documentation Hub Split

**Input**: Design documents from `/specs/014-readme-docs-hub-split/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/docs-structure.md, quickstart.md

**Organization**: Tasks are grouped by user story. This is a documentation-only spec — all tasks create or edit Markdown files. No production source code (`.rs`) is touched.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup

**Purpose**: Create the `docs/` directory structure.

- [ ] T001 Create the `docs/` directory at the repository root

**Checkpoint**: `docs/` directory exists and is ready for article files.

---

## Phase 2: Foundational (Create All docs/ Articles)

**Purpose**: Extract every major section from the current README (1176 lines) into its dedicated `docs/` article. These MUST all be created before the README can be slimmed (the content must exist in `docs/` before it can be removed from the README).

**Source of truth**: The content-trace mapping in `data-model.md` (36 rows). Each task below corresponds to one or more rows. Content is moved verbatim from the README — no rewriting, no new content.

**Critical rules for every article**:
- Title heading `# <Topic Name>` at the top.
- Back-link `[← Back to README](../README.md)` immediately after the title.
- Source file links updated: `./src/...` → `../src/...`, `./deploy/...` → `../deploy/...`.
- Internal README anchor links updated to point to other `docs/` articles where the target moved.
- Code blocks, tables, and examples preserved verbatim.

- [ ] T002 [P] Create `docs/deployment.md` — extract README sections: "Build the Image" (lines 90-108), "TLS Provisioning" (lines 171-218), and the detailed deploy context from "Deploy to Kubernetes" (lines 110-169). Title: `# Deployment Guide`. Include the cert-manager + manual Secret TLS paths, the image pull policy note, and the build command.
- [ ] T003 [P] Create `docs/configuration.md` — extract README sections: "CLI Flags & Environment Variables" (259-276), "Precedence" (277-292), "Allocation CRD" (294-326), "ClusterCapacity CRD" (328-355), "Adjusting the Budget at Runtime" (462-479), "Per-Resource Budget Overrides" (481-523), "Budget Edge Cases" (632-643). Title: `# Configuration Reference`. This is the largest article — contains both CRD field tables, all flag tables, budget adjustment examples, and edge cases.
- [ ] T004 [P] Create `docs/node-exclusion.md` — extract README section "Node Exclusion" (357-460). Title: `# Node Exclusion`. Includes the two-layer model, the label-selector examples (control-plane, OR semantics, label value), the spec-006→007 migration, the remove/inspect examples, and the invalid-selector handling note.
- [ ] T005 [P] Create `docs/enforcement-modes.md` — extract README section "Enforcement Modes (Enforce / Dry-Run)" (525-568). Title: `# Enforcement Modes`. Includes the mode table, fail-closed-in-both-modes note, and the kubectl patch examples. **Remove the `(NON-NEGOTIABLE)` text from the fail-closed note** — per constitution formatting convention, all principles are inherently non-negotiable and no inline markers are used.
- [ ] T006 [P] Create `docs/workload-exclusion.md` — extract README section "Workload Exclusion" (570-630). Title: `# Workload Exclusion`. Includes the check order, priority-class string match, "excluded pods are still counted", backward compatibility, fail-closed note, kubectl patch examples, and the cold-start/self-admission callout.
- [ ] T007 [P] Create `docs/observability.md` — extract README sections: "HTTP Endpoints" (647-665), "Prometheus Metrics" (667-693), "Structured Logging" (695-738), "Rejection Messages" (740-764). Title: `# Metrics & Observability`. Includes the endpoint table, the 8-metric Prometheus table, the log field table, the decision-type log levels, the example log line, and the rejection message format with examples.
- [ ] T008 [P] Create `docs/failure-modes.md` — extract README sections: "Failure Modes" (766-801) and "Webhook Self-Admission (Bootstrap)" (819-833). Title: `# Failure Modes`. Includes the failure-mode table (condition/outcome/reason/HTTP), the exempt-decision note, and the self-admission namespaceSelector explanation. Cross-link to `./kubernetes-compatibility.md` for the K8s version aspect.
- [ ] T009 [P] Create `docs/kubernetes-compatibility.md` — extract README section "Kubernetes Compatibility" (803-818). Title: `# Kubernetes Compatibility`. Includes the N-2 window explanation, the current CI matrix (1.34-1.36), the GA API list, and the deprecation policy.
- [ ] T010 [P] Create `docs/architecture.md` — extract README section "Architecture" (835-866). Title: `# Architecture`. Includes the ASCII data-flow diagram, the 3-component descriptions, and the link to the full design in `../specs/001-capacity-admission-webhook/data-model.md`.
- [ ] T011 [P] Create `docs/erw-verify.md` — extract README sections: the erw-verify intro (868-883), "Build" (884-893), "Configure (.env)" (894-916), "Run the full pipeline" (917-938), "Usage" (940-952), "CLI Flags" (954-970), "Exit Codes" (972-985), "Scenario Inventory" (987-1035). Title: `# On-Demand Verification (erw-verify)`. This is the second-largest article — contains the full CLI reference, exit code table, .env variable table, and the three scenario groups (S1-S9, S10-S11, E1-E5).
- [ ] T012 [P] Create `docs/equalizer.md` — extract README section "Multi-Cluster Capacity Equalizer (spec-013)" (1110-1172). Title: `# Multi-Cluster Capacity Equalizer`. Includes the how-it-works algorithm, worked example, deployment steps, EqualizerConfig CRD reference table, per-cluster status states, and the "not on admission critical path" note.

**Checkpoint**: All 11 articles exist under `docs/`. Verify with `ls docs/*.md` (expect 11 files). Each has a title heading, back-link, and content from the README.

---

## Phase 3: User Story 1 — Quick-Start Reader (Priority: P1) 🎯 MVP

**Goal**: Slim the README to a navigational hub — project description, self-contained quick-start, TOC with per-capability summaries, and License. An operator's first-screen experience is a concise overview + quick-start, not a wall of reference text.

**Independent Test**: VS-1 (README < 250 lines), VS-4 (no broken anchors), VS-17 (only hub sections remain), VS-18 (Development section removed).

- [ ] T013 [US1] Create `docs/README.md` index — list all 11 articles with one-line descriptions, organized by audience journey (Getting Started → Configuration → Operations → Architecture → Tooling → Equalizer). Title: `# Documentation Index`. This is FR-011.
- [ ] T014 [US1] Rewrite `README.md` as the hub. Keep ONLY these sections (see data-model.md rows 1-10, 36):
  1. **Title + blurb** (lines 1-5, keep as-is).
  2. **Intro paragraph** — trim lines 7-19: remove the "This README is the single entry point..." paragraph (it contradicts the hub model). Replace with 2-3 sentences: what the project is + link to [Documentation](#documentation) TOC.
  3. **`## Overview`** (lines 21-44) — keep verbatim (this is the project description).
  4. **`## Quick Start`** — keep the Prerequisites (51-63), a compressed Published Image reference (2-3 lines + link to `./docs/deployment.md` for tag conventions), the 6-step Deploy sequence (110-145, compressed — remove the inline TLS detail, link to `./docs/deployment.md#tls-provisioning`), and the Verify commands (220-255). Target: ~80 lines total for the Quick Start section. The quick-start MUST be self-contained (operator deploys + verifies without clicking through).
  5. **`## Documentation`** — NEW section: a TOC linking to all 11 `docs/` articles. For each article, a heading or bold title + 1-3 sentence summary + link. Use the descriptions from `docs/README.md` (T013). Organize by journey: Deployment → Configuration → Node Exclusion → Enforcement Modes → Workload Exclusion → Observability → Failure Modes → Kubernetes Compatibility → Architecture → Verification Tool → Equalizer.
  6. **Remove `## Development`** (lines 1037-1108) entirely. Replace with a single line under Documentation: "For build instructions, testing, and project structure, see [CONTRIBUTING.md](./CONTRIBUTING.md)."
  7. **`## License`** (lines 1174-1176) — keep verbatim.
  8. Remove ALL other sections (Configuration, Node Exclusion, Enforcement Modes, Workload Exclusion, Metrics & Observability, Failure Modes, Kubernetes Compatibility, Architecture, On-Demand Verification, Multi-Cluster Equalizer) — their content now lives in `docs/`.
- [ ] T015 [US1] Fix all internal anchor links in the slimmed README. Run `grep -oP '\]\(#[^)]+\)' README.md` and verify every remaining `](#anchor)` resolves to a heading still in the README. Any anchor pointing to a moved section must be updated to a `./docs/...` link or removed. Known anchors to check: `#tls-provisioning` → `./docs/deployment.md#tls-provisioning`, `#node-exclusion` → `./docs/node-exclusion.md`, `#failure-modes` → `./docs/failure-modes.md`, `#workload-exclusion` → `./docs/workload-exclusion.md`, `#structured-logging` → `./docs/observability.md#structured-logging`, `#prometheus-metrics` → `./docs/observability.md#prometheus-metrics`, `#scenario-inventory` → `./docs/erw-verify.md#scenario-inventory`, `#kubernetes-compatibility` → `./docs/kubernetes-compatibility.md`, `#adjusting-the-budget-at-runtime` → `./docs/configuration.md#adjusting-the-budget-at-runtime`, `#per-resource-budget-overrides-spec-012` → `./docs/configuration.md#per-resource-budget-overrides-spec-012`, `#enforcement-modes-enforce--dry-run` → `./docs/enforcement-modes.md`, `#rejection-messages` → `./docs/observability.md#rejection-messages`, `#on-demand-verification-erw-verify` → `./docs/erw-verify.md`, `#webhook-self-admission-bootstrap` → `./docs/failure-modes.md#webhook-self-admission-bootstrap`.

**Checkpoint**: README is under 250 lines. `grep '^## ' README.md` shows only: Overview, Quick Start, Documentation, License. All `](#anchor)` links resolve.

---

## Phase 4: User Story 2 — Reference Lookup (Priority: P2)

**Goal**: Ensure every `docs/` article is reachable from the README TOC, all cross-references between articles work, and no external links to the old README anchors break silently.

**Independent Test**: VS-3 (every article linked from README TOC), VS-12 (cross-references updated), VS-13 (no absolute GitHub URLs).

- [ ] T016 [US2] Verify every `docs/*.md` file is linked from the README TOC (VS-3). Run `grep -oP '\./docs/[a-z-]+\.md' README.md | sort -u` and confirm all 11 articles + `docs/README.md` appear. Add any missing links.
- [ ] T017 [P] [US2] Fix cross-reference in `ARTIFACTS.md` line 54: change `../README.md#verification-scenarios` to `../docs/erw-verify.md#scenario-inventory` (LR-004). The old anchor was already broken.
- [ ] T018 [P] [US2] Verify `specs/006-schedulable-node-filter/quickstart.md` line 13 link to `../../README.md#quick-start` still resolves (LR-005) — the Quick Start anchor remains in the README. No change needed unless the anchor format changed.
- [ ] T019 [US2] Scan all `docs/*.md` files for absolute GitHub URLs (`https://github.com/...`) and replace with relative paths (VS-13, LR-006). Run `grep -rn 'https://github.com' docs/` — expect zero matches after fix.
- [ ] T020 [US2] Verify all cross-article links within `docs/` resolve. Run `grep -oP '\]\(\./[a-z-]+\.md[^)]*\)' docs/*.md` and confirm each linked file exists.

**Checkpoint**: Every docs/ article reachable from README in ≤2 clicks. No broken links. No absolute GitHub URLs.

---

## Phase 5: User Story 3 — Contributor Guidance (Priority: P3)

**Goal**: A contributor adding a new capability knows exactly where to document it and how to maintain the hub structure.

**Independent Test**: VS-15 (CONTRIBUTING.md has Documentation Structure section).

- [ ] T021 [US3] Add a "Documentation Structure" subsection to `CONTRIBUTING.md` (after the existing "Code Style" section, before "Project Structure"). Content: explain the `docs/` hub model per Principle X — user-facing reference goes in `docs/<topic>.md`; the README holds only a TOC + 1-3 sentence summary per capability. State the contributor obligation: adding a user-facing capability requires (1) creating/updating the `docs/` article, (2) adding a TOC entry + summary in the README, both in the same PR. Reference `docs/README.md` as the article index.

**Checkpoint**: CONTRIBUTING.md explains the docs/ structure and the same-change documentation obligation.

---

## Phase 6: Polish & Validation

**Purpose**: Verify the split is complete and accurate before merge.

- [ ] T022 Run content-trace verification (VS-5): for each of the 36 rows in `data-model.md`, confirm the old README section's content exists at its destination. No section lost.
- [ ] T023 Run accuracy spot-checks: verify VR-001 through VR-021 by cross-checking documented values against source files (`src/config.rs`, `src/crd/allocation.rs`, `src/crd/cluster_capacity.rs`, `src/metrics.rs`, `src/webhook/error.rs`, `src/bin/erw-verify/args.rs`). Fix any discrepancies — the code is the source of truth (FR-012).
- [ ] T024 Verify `.editorconfig` compliance: all new `docs/*.md` files and the modified README use LF line endings and match `.editorconfig` settings (VS-16).
- [ ] T025 Run `wc -l README.md` and confirm the result is under 250 lines (VS-1).
- [ ] T026 Verify no `## Configuration`, `## Metrics`, `## Failure Modes`, `## Architecture`, `## On-Demand Verification`, `## Multi-Cluster Capacity Equalizer`, or `## Development` headings remain in README.md (VS-17, VS-18).
- [ ] T027 Verify the scenario inventory (S1-S11, E1-E5) in `docs/erw-verify.md` matches the scenario modules in `src/bin/erw-verify/scenarios/` (VR-018).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — creates the `docs/` directory.
- **Phase 2 (Foundational)**: Depends on Phase 1. All 11 articles are `[P]` — fully parallelizable (different files, no dependencies between them).
- **Phase 3 (US1 — README slim)**: Depends on Phase 2 completion (content must exist in `docs/` before README is slimmed). T013 → T014 → T015 (sequential within the story).
- **Phase 4 (US2 — Cross-refs)**: Depends on Phase 3 (README must be slimmed before cross-refs are verified). T017/T018 are `[P]`; T016/T019/T020 depend on T014.
- **Phase 5 (US3 — CONTRIBUTING)**: Depends on Phase 3 (references the final docs/ structure). Independent of Phase 4.
- **Phase 6 (Polish)**: Depends on all prior phases.

### Parallel Opportunities

- **T002–T012**: All 11 article-creation tasks are fully parallel — different files, no dependencies. This is the bulk of the work and can be done in one batch.
- **T017, T018**: Independent cross-reference fixes, parallelizable.

### Implementation Strategy

**MVP**: Phase 1 + Phase 2 + Phase 3 (T001–T015). After this, the README is a slim hub with all content in `docs/`. The split is functionally complete.

**Full delivery**: Add Phase 4 (cross-ref fixes) + Phase 5 (CONTRIBUTING) + Phase 6 (validation). The validation tasks (T022–T027) are the accuracy gate — they catch any content loss or stale values before merge.

**Recommended delegation**: This is a documentation-only spec with no code changes. It is well-suited for a single Claude Code delegation round. The parallel article-creation tasks (T002–T012) are the bulk; the README rewrite (T014) is the critical-path task requiring the most judgment.

---

## Notes

- All content is moved verbatim from the README — no new content is authored (this is reorganization, not a documentation rewrite).
- The only content CHANGE allowed: removing the `(NON-NEGOTIABLE)` inline marker from the enforcement-modes text (T005) per constitution formatting convention.
- The "Development" section (lines 1037-1108) is DELETED from the README, not moved — it already exists in CONTRIBUTING.md (Principle XIII).
- FR-012 accuracy gate: if any documented value disagrees with the code, the code wins. Fix the doc, not the code.
- No `.rs` files are touched. The CI quality gate (`cargo fmt/clippy/test`) is unaffected; only the `editorconfig` CI job runs on the new files.
