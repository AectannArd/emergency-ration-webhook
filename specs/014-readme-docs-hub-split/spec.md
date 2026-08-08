# Feature Specification: README Documentation Hub Split

**Feature Branch**: `014-readme-docs-hub-split`

**Created**: 2026-08-08

**Status**: Draft

**Input**: User description: "Our README has grown too heavy again. We should split README content into separate articles under `./docs`, leaving a table of contents and only brief descriptions in the README. Other content should be accessible via links to the `./docs` directory."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Quick-Start Reader (Priority: P1)

A first-time operator lands on the GitHub repository page and wants to get the
webhook running in their cluster as fast as possible. They open the README,
scan the first screen, and immediately see: what the project is, a concise
quick-start (install, deploy, verify), and a short table of contents with
one-line descriptions linking to deeper material. They never have to scroll past
hundreds of lines of CRD reference, metrics catalog, or failure-mode tables to
reach the deploy command.

**Why this priority**: the README is the most-visited page of any open-source
project. If the first screen is a wall of text, operators leave. The quick-start
experience is the single highest-impact deliverable of this split.

**Independent Test**: render the README and confirm — within the first scroll
page — an operator can find the project description, the quick-start deploy
sequence, and a TOC linking to every major capability. No section deeper than
the quick-start appears above the fold.

**Acceptance Scenarios**:

1. **Given** the current 1176-line README, **When** an operator opens it on
   GitHub, **Then** the first visible section is a brief project description
   followed by a quick-start guide, and a table of contents links to every
   detailed article under `docs/`.
2. **Given** an operator following the quick-start, **When** they execute the
   documented deploy + verify steps, **Then** the steps are self-contained in
   the README (no link-hopping required to get a basic deployment running).
3. **Given** the quick-start section, **When** measured by line count, **Then**
   it does not exceed approximately 80 lines (clone → deploy → verify), keeping
   it a true quick-start and not a reference manual.

---

### User Story 2 - Reference Lookup (Priority: P2)

An experienced operator who already runs the webhook needs to look up a specific
configuration detail — how `budgetPercent` works, what a specific Prometheus
metric means, how to configure node exclusion selectors, or what the
`EqualizerConfig` CRD spec looks like. From the README's TOC they click the
relevant link and land directly on the dedicated article covering that topic in
depth, without scrolling past unrelated sections.

**Why this priority**: this is the daily-use workflow for any operator running
the webhook in production. The monolithic README made this painful — the
information was there but buried.

**Independent Test**: for every major capability, verify a direct link path
exists from README TOC → the `docs/` article covering that capability, and that
the article contains the full reference content (no information lost in the
split).

**Acceptance Scenarios**:

1. **Given** an operator needs the full CRD reference for the Allocation CRD,
   **When** they click the "Configuration" link in the README TOC, **Then** they
   land on a `docs/` article containing the complete field-by-field reference
   (spec, status, examples) previously inlined in the README.
2. **Given** an operator needs the Prometheus metrics catalog, **When** they
   click the "Metrics & Observability" link in the TOC, **Then** they land on a
   `docs/` article with the full metrics table, structured-log keys, and HTTP
   endpoint reference.
3. **Given** an operator needs the `erw-verify` CLI flags reference, **When**
   they click the relevant TOC link, **Then** they land on a `docs/` article with
   the full flag reference, exit codes, and scenario inventory.
4. **Given** the `docs/` directory, **When** listing its contents, **Then**
   every article is discoverable by a descriptive filename (not numbered or
   opaque names).

---

### User Story 3 - Contributor Adding a New Capability (Priority: P3)

A contributor implements a new user-facing capability (a new flag, a new CRD
field, a new enforcement mode). Following the constitution's same-change rule
(Principle X, updated v2.9.0), they need to know exactly where to document it:
which `docs/` article to create or update, and how to add the TOC entry +
summary in the README. The structure must make this obvious — the contributor
should not need to reverse-engineer the documentation layout from scratch.

**Why this priority**: the split's long-term success depends on contributors
maintaining it. If the structure is unclear, the README re-bloats within a few
PRs.

**Independent Test**: verify a CONTRIBUTING section or `docs/README.md` index
explains the documentation structure and the contributor's documentation
obligation (create/update `docs/` article + README TOC entry + summary).

**Acceptance Scenarios**:

1. **Given** a contributor adding a new user-facing flag, **When** they consult
   CONTRIBUTING.md or the docs index, **Then** they find clear guidance: "user-
   facing config goes in `docs/<topic>.md`; add a TOC entry + one-line summary in
   the README."
2. **Given** a contributor updating an existing capability, **When** they consult
   the docs structure, **Then** they find which article to edit without guessing.

---

### Edge Cases

- **External links to README anchors**: the old README had many internal
  anchors (`#configuration`, `#metrics--observability`, etc.) that may be
  linked from external sources, issues, or the spec docs themselves. These
  anchors will break when content moves to `docs/`. The split MUST verify no
  critical in-repo links break (spec docs, AGENTS.md, CLAUDE.md cross-refs) and
  SHOULD preserve README-level anchors where the section remains (Overview,
  Quick Start, License).
- **License section**: the License section is short and canonical — it stays in
  the README, not in `docs/`.
- **Quick-start duplication risk**: the quick-start in the README must not
  duplicate the detailed deployment article in `docs/` verbatim. If the
  quick-start and the deployment article diverge, the deployment article
  (`docs/`) is the source of truth.
- **ARTIFACTS.md and CONTRIBUTING.md are out of scope**: they are separate
  governed documents (Principle XIII, XIV) and are NOT part of this split.
- **No information loss**: every piece of information in the current 1176-line
  README MUST be present either in the slim README (quick-start, TOC,
  summaries, license) or in a `docs/` article. The split is a reorganization,
  not a deletion. A content-trace verification (every old section → new
  location) MUST confirm zero information loss.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The README MUST be reduced from its current monolithic form
  (1176 lines) to a navigational hub containing: a project description, a
  concise quick-start guide, a table of contents linking to every `docs/`
  article, brief per-capability summaries (1–3 sentences each with a link), and
  the License section.
- **FR-002**: A `docs/` directory MUST be created at the repository root
  containing separate Markdown articles for each major documentation topic
  extracted from the README.
- **FR-003**: Every major section of the current README whose content exceeds a
  few paragraphs (Configuration, Node Exclusion, Per-Resource Budget Overrides,
  Enforcement Modes, Workload Exclusion, Metrics & Observability, Failure Modes,
  Kubernetes Compatibility, Architecture, On-Demand Verification, Multi-Cluster
  Capacity Equalizer) MUST be extracted into a dedicated `docs/` article.
- **FR-004**: The quick-start section in the README MUST be self-contained —
  an operator can deploy and verify the webhook using only the quick-start
  steps without clicking through to any `docs/` article.
- **FR-005**: The README MUST contain a table of contents section that links to
  every `docs/` article, organized logically (not in arbitrary order).
- **FR-006**: Each `docs/` article MUST be reachable via exactly one link from
  the README TOC (discoverability guarantee per Principle X v2.9.0).
- **FR-007**: For each per-capability summary in the README, the summary MUST
  be 1–3 sentences and MUST link to the corresponding `docs/` article. The
  summary MUST NOT duplicate the article's detailed content verbatim.
- **FR-008**: The split MUST preserve all information currently in the README —
  no content is deleted. A content-trace mapping (old README section → new
  location) MUST be produced as a verification artifact.
- **FR-009**: Internal cross-references within the old README (links between
  sections) MUST be updated to point to the new `docs/` article locations.
- **FR-010**: The `docs/` articles MUST use relative links (e.g.
  `../README.md`, `./configuration.md`) so the links work correctly whether
  viewed on GitHub, in a local clone, or in an editor preview.
- **FR-011**: A `docs/README.md` index file (or equivalent table of contents
  within `docs/`) MUST be created that lists all articles with one-line
  descriptions, serving as a local index for the `docs/` directory.
- **FR-012**: Every documented value (CRD field names, metric names, flag
  names, env var names, default values, exit codes, scenario IDs) in the
  `docs/` articles MUST match the shipped code exactly. If any discrepancy is
  found between the old README content and the actual code, the code is the
  source of truth — the `docs/` article documents what the code does, not what
  stale README content claimed.
- **FR-013**: CONTRIBUTING.md MUST be updated with a brief section documenting
  the `docs/` structure and the contributor's documentation obligation: new
  user-facing capability → create/update `docs/` article + add README TOC entry
  + summary.

### Key Entities *(include if feature involves data)*

- **`docs/` article**: a Markdown file under `docs/` covering a single major
  documentation topic. Named descriptively (e.g. `configuration.md`,
  `metrics.md`, `architecture.md`), not numbered. Contains the full reference
  content extracted from the README.
- **README summary**: a 1–3 sentence description of a capability in the README
  that links to the corresponding `docs/` article. The summary is a pointer,
  not a duplicate.
- **Content-trace mapping**: a verification artifact documenting, for every
  section of the old README, its new location (slim README section or specific
  `docs/` article). Proves zero information loss.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The README is reduced to under 250 lines (from 1176), containing
  only the hub structure: description, quick-start, TOC, per-capability
  summaries, and License.
- **SC-002**: Every piece of information from the original README is present in
  either the slim README or a `docs/` article (zero information loss, verified
  by content-trace).
- **SC-003**: An operator can reach any specific reference topic (CRD fields,
  metrics catalog, CLI flags, deployment manifests) in at most 2 clicks from the
  README (README TOC link → article).
- **SC-004**: All documented values (CRD fields, metric names, flag names,
  default values, exit codes, scenario IDs) match the shipped code — verified
  by cross-checking each against its source file.
- **SC-005**: All in-repo cross-references (from spec docs, AGENTS.md,
  CLAUDE.md) that pointed to old README anchors resolve to valid locations
  (either remaining README anchors or `docs/` articles).

## Assumptions

- The current README (1176 lines, 40+ sections) represents the complete set of
  user-facing documentation to be split; no new content is authored — this is
  purely a structural reorganization.
- The Constitution Principle X (v2.9.0) defines the target model and is the
  authority for the hub-and-articles structure.
- CONTRIBUTING.md already exists and contains contribution-workflow
  documentation (Principle XIII); this spec only adds a documentation-structure
  subsection, not a rewrite.
- ARTIFACTS.md (Principle XIV) is out of scope — it remains at the repo root,
  unchanged.
- The `docs/` directory naming convention is descriptive filenames
  (e.g. `configuration.md`, not `01-configuration.md`), matching GitHub
  community norms.
- This is a documentation-only spec: no production source code (Rust) is
  changed. Implementation is limited to Markdown file creation/editing and
  cross-reference updates.
- The implementation will be delegated to Claude Code on the build host, like
  any other spec — documentation restructuring with accuracy verification is
  well-suited to the TDD-adapted discipline (FR-012 accuracy gate).
