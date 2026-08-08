# Phase 0: Research — README Documentation Hub Split

## R1. Article grouping strategy

**Decision**: Group README content into 11 topic-focused articles under `docs/`,
organized by the operator's journey: get it running → tune it → operate it →
understand it → verify it → extend it (equalizer).

**Rationale**: The current README has ~40 sections with no clear separation
between "getting started" and "deep reference." An operator looking for the
Prometheus metrics table should not have to scroll past node-exclusion selectors
and CRD migration guides. One article per major capability means each is
scannable independently.

**Alternatives considered**:
- **Fewer, larger articles** (e.g. one "Reference" article covering config +
  metrics + failure modes): rejected — recreates the wall-of-text problem inside
  docs/.
- **More, smaller articles** (e.g. splitting configuration into separate files
  per CRD): rejected — fragments related content an operator reads together
  (both CRDs are part of one mental model).

## R2. Article naming convention

**Decision**: Descriptive filenames, kebab-case, no numeric prefixes. Each
filename matches the capability name an operator would search for.

| Article | Filename | Source README sections (line range) |
|---------|----------|-------------------------------------|
| Deployment Guide | `deployment.md` | Quick Start → Published Image, Build, Deploy, TLS, Verify (lines 46-255) — the deploy steps, not the overview |
| Configuration Reference | `configuration.md` | Configuration → CLI Flags, Precedence, Allocation CRD, ClusterCapacity CRD, Budget Edge Cases, Runtime Adjustment, Per-Resource Overrides (lines 257-524, 632-643) |
| Node Exclusion | `node-exclusion.md` | Node Exclusion (lines 357-460) |
| Enforcement Modes | `enforcement-modes.md` | Enforcement Modes (lines 525-568) |
| Workload Exclusion | `workload-exclusion.md` | Workload Exclusion (lines 570-630) |
| Observability | `observability.md` | Metrics & Observability — HTTP Endpoints, Prometheus, Structured Logging, Rejection Messages (lines 645-764) |
| Failure Modes | `failure-modes.md` | Failure Modes, Webhook Self-Admission (lines 766-833) |
| Kubernetes Compatibility | `kubernetes-compatibility.md` | Kubernetes Compatibility (lines 803-818) |
| Architecture | `architecture.md` | Architecture (lines 835-866) |
| On-Demand Verification | `erw-verify.md` | On-Demand Verification — Build, Configure, Run, Usage, CLI Flags, Exit Codes, Scenario Inventory (lines 868-1035) |
| Multi-Cluster Equalizer | `equalizer.md` | Multi-Cluster Capacity Equalizer (lines 1110-1172) |

**Rationale**: Descriptive names (not `01-deployment.md`) because (a) GitHub
renders `docs/` as a file listing — a descriptive name IS the TOC entry, (b) the
README TOC provides the ordering, the filename doesn't need to, (c) contributors
adding a new article don't need to renumber.

**Alternatives considered**:
- **Numeric prefixes** (`01-deployment.md`): rejected — imposes a false ordering
  and creates renumbering churn when articles are inserted.
- **Subdirectories** (`docs/configuration/allocation-crd.md`): rejected —
  over-engineering for 11 articles; flat directory is scannable.

## R3. What stays in the README vs. moves to docs/

**Decision**: The README keeps ONLY: (1) project description (Overview, trimmed),
(2) Quick Start (self-contained deploy+verify, ~80 lines), (3) Table of Contents,
(4) per-capability summaries (1-3 sentences + link), (5) License. Everything else
moves to docs/.

**Rationale**: This is exactly Principle X v2.9.0's hub model. The quick-start
stays self-contained because an operator's first experience should not require
link-hopping. The per-capability summaries give the "what is this?" answer at a
glance; the link gives the "how do I use it?" depth.

**What about the "Overview" section (lines 21-44)?** It's 24 lines of project
description — appropriate for the README top. It stays, lightly trimmed to remove
the paragraph that duplicates the Quick Start's purpose statement.

**What about the "Development" section (lines 1037-1108)?** This is contributor
content (build, test, quality gate, project structure). Per Principle XIII, it
belongs in CONTRIBUTING.md — and CONTRIBUTING.md already has all of it (Build §
lines 27-74, Testing § lines 75-88, Quality Gate § lines 89-102, Project
Structure § lines 136-169). The README "Development" section is **removed** and
replaced with a one-line link to CONTRIBUTING.md. It does NOT move to docs/
(docs/ is operator-facing usage content per Principle X; contributor content goes
to CONTRIBUTING.md per Principle XIII).

## R4. Cross-reference update strategy

**Decision**: Systematically find and update every link pointing to a README
anchor that is being moved.

**Known in-repo cross-references to update** (found via `grep -rn 'README.md#'`):

| Source file | Old link | New link |
|-------------|----------|----------|
| `ARTIFACTS.md:54` | `../README.md#verification-scenarios` (already broken — no such anchor) | `../docs/erw-verify.md#scenario-inventory` |
| `specs/006-schedulable-node-filter/quickstart.md:13` | `../../README.md#quick-start` | stays valid — Quick Start anchor remains in README |

**README internal links**: the current README has many `#anchor` links between
sections (e.g. `[Failure Modes](#failure-modes)`, `[Workload Exclusion](#workload-exclusion)`).
When a section moves to docs/, these links must be updated to point to the docs/
article (e.g. `[Failure Modes](./docs/failure-modes.md)`). The implementing agent
must grep for every `](#` in the slimmed README and fix or remove each.

**docs/ article cross-links**: articles under docs/ reference each other via
relative links (e.g. from `configuration.md` to `enforcement-modes.md`). These
use `./` prefix within docs/.

**Rationale**: broken links are worse than no links. A systematic grep-based
sweep is the only reliable way to catch them all — manual inspection misses links
buried in prose.

## R5. The "Development" section is a duplicate, not a move

**Decision**: The README "Development" section is removed entirely. It is NOT
moved to docs/. It already exists in CONTRIBUTING.md.

**Evidence** (CONTRIBUTING.md already covers it):
- Build: CONTRIBUTING.md § Building (lines 27-74) — webhook binary, container
  image, verification tool.
- Tests: CONTRIBUTING.md § Testing (lines 75-88) — unit, integration, BDD, E2E.
- Quality Gate: CONTRIBUTING.md § Quality Gate (lines 89-102) — fmt, clippy,
  test, editorconfig.
- Project Structure: CONTRIBUTING.md § Project Structure (lines 136-169) — full
  source tree.

**Rationale**: keeping a duplicate in docs/ would violate Principle XIII
(non-overlap rule). The README gets a single line: "For build instructions, test
commands, and project structure, see [CONTRIBUTING.md](./CONTRIBUTING.md)."

## R6. docs/README.md index purpose

**Decision**: Create a `docs/README.md` that serves as the local index for the
`docs/` directory. It lists every article with a one-line description. This is
FR-011 in the spec.

**Rationale**: when an operator browses to `docs/` on GitHub, they see a file
listing. A `README.md` in that directory renders inline, giving a human-readable
index instead of just filenames. It also serves as the contributor's reference
for "what articles exist and what they cover" (FR-013 / US3).

## R7. Relative link convention

**Decision**: All cross-references use relative paths. From README to docs/:
`./docs/deployment.md`. From docs/ articles to README: `../README.md`. From docs/
article to docs/ article: `./configuration.md`. No absolute GitHub URLs
(`https://github.com/.../blob/main/docs/...`).

**Rationale**: relative links work in GitHub render, local clone, editor preview,
and any mirror (e.g. GitVerse). Absolute URLs break on mirrors and in offline
viewing.
