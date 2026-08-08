# Quickstart: Validation Guide — README Documentation Hub Split

This is a **validation guide**, not an implementation tutorial. It defines the
checks the implementing agent (or reviewer) runs to prove the split is correct:
zero information loss, all links resolve, all values match the code.

## Prerequisites

- A clone of the repository at the implementation branch
- `grep` / `rg` (ripgrep) for content searches
- The source files referenced in the accuracy rules (data-model.md VR-001..VR-021)

## Validation Scenarios

### VS-1: README line count (SC-001)

**Given** the implementation branch is checked out,
**When** `wc -l README.md` is run,
**Then** the output is **under 250 lines**.

```sh
wc -l README.md   # expect < 250
```

### VS-2: docs/ directory exists with all articles (FR-002, FR-003)

**Given** the implementation branch is checked out,
**When** `ls docs/*.md` is run,
**Then** exactly these 12 files exist (11 articles + docs/README.md index):

```sh
ls docs/*.md
# Expected:
# docs/README.md
# docs/architecture.md
# docs/configuration.md
# docs/deployment.md
# docs/enforcement-modes.md
# docs/equalizer.md
# docs/erw-verify.md
# docs/failure-modes.md
# docs/kubernetes-compatibility.md
# docs/node-exclusion.md
# docs/observability.md
# docs/workload-exclusion.md
```

### VS-3: Every docs/ article linked from README TOC (FR-005, FR-006, LR-001)

**Given** the slimmed README,
**When** `grep -oP '\./docs/[a-z-]+\.md' README.md | sort -u` is run,
**Then** the output lists all 11 article files (docs/README.md is the index,
optionally linked).

```sh
grep -oP '\./docs/[a-z-]+\.md' README.md | sort -u
```

### VS-4: No broken internal anchors in README (LR-003)

**Given** the slimmed README,
**When** every `](#anchor)` link is extracted and each anchor is checked against
README headings,
**Then** every anchor resolves to an existing heading.

```sh
# Extract all anchor links and check they resolve:
grep -oP '\]\(#([^)]+)\)' README.md | sed 's/](#//' | sed 's/)//' | while read anchor; do
  # GitHub anchor format: lowercase, spaces→hyphens, strip special chars
  heading=$(grep -iP '^#{2,4} ' README.md | tr '[:upper:]' '[:lower:]' | sed 's/^#\+ //' | sed 's/ /-/g' | sed 's/[^a-z0-9-]//g')
  echo "$heading" | grep -qx "$anchor" || echo "BROKEN: #$anchor"
done
```

### VS-5: Content-trace completeness (FR-008, SC-002)

**Given** the content-trace mapping in `data-model.md` (36 rows),
**When** the implementing agent verifies each old section's content exists in its
destination,
**Then** zero sections are lost. This is a manual verification — for each row,
confirm the key content (heading + core tables/examples) is present at the
destination.

### VS-6: Accuracy — configuration values (VR-001..VR-003)

**Given** `docs/configuration.md`,
**When** each flag name, default value, and the precedence order are compared
against `src/config.rs`,
**Then** they match exactly.

```sh
# Spot-check: the 7 flags exist in config.rs
for flag in port tls_cert_file tls_key_file decision_timeout_ms \
  capacity_freshness_timeout_secs namespace metrics_port; do
  grep -q "$flag" src/config.rs || echo "MISSING in config.rs: $flag"
done
```

### VS-7: Accuracy — CRD fields (VR-004..VR-009)

**Given** `docs/configuration.md`,
**When** each CRD field name is compared against `src/crd/allocation.rs` and
`src/crd/cluster_capacity.rs`,
**Then** they match exactly (serde field names, not Rust struct field names —
check `#[serde(rename = "...")]` if present).

### VS-8: Accuracy — metrics (VR-010..VR-012)

**Given** `docs/observability.md`,
**When** each metric name is compared against `src/metrics.rs`,
**Then** all 8 metric names and label vocabularies match.

```sh
for metric in capacity_admission_verdicts_total \
  capacity_admission_exemptions_total \
  capacity_admission_decision_duration_seconds \
  capacity_admission_capacity_freshness_seconds \
  capacity_admission_allocation_ratio \
  capacity_admission_total_allocatable \
  capacity_admission_current_allocation \
  capacity_admission_ceiling; do
  grep -q "$metric" src/metrics.rs || echo "MISSING in metrics.rs: $metric"
done
```

### VS-9: Accuracy — failure modes (VR-013..VR-014)

**Given** `docs/failure-modes.md`,
**When** each reason slug and HTTP code is compared against `src/webhook/error.rs`,
**Then** they match exactly.

### VS-10: Accuracy — erw-verify CLI (VR-015..VR-017)

**Given** `docs/erw-verify.md`,
**When** each flag name, exit code, and `.env` variable is compared against
`src/bin/erw-verify/args.rs`,
**Then** they match exactly.

### VS-11: Accuracy — scenario inventory (VR-018)

**Given** `docs/erw-verify.md` Scenario Inventory section,
**When** scenario IDs and descriptions are compared against the scenario modules
in `src/bin/erw-verify/scenarios/`,
**Then** all S1-S11, E1-E5 scenarios match.

### VS-12: Cross-reference updates (FR-009, LR-004, LR-005)

**Given** the implementation branch,
**When** these specific cross-references are checked:

```sh
# ARTIFACTS.md no longer points to broken README anchor:
grep 'verification-scenarios' ARTIFACTS.md
# Should reference docs/erw-verify.md now, not README.md#verification-scenarios

# specs/006 quickstart still links to README#quick-start (valid):
grep 'README.md#quick-start' specs/006-schedulable-node-filter/quickstart.md
# Should still work — Quick Start anchor remains in README
```

**Then** ARTIFACTS.md is updated and specs/006 link still resolves.

### VS-13: No absolute GitHub URLs (LR-006, FR-010)

**Given** all `docs/*.md` files,
**When** `grep -rn 'https://github.com' docs/` is run,
**Then** zero matches (all links are relative).

```sh
grep -rn 'https://github.com' docs/   # expect no output
```

### VS-14: docs/README.md index (FR-011)

**Given** `docs/README.md`,
**When** its content is read,
**Then** it lists all 11 articles with one-line descriptions.

### VS-15: CONTRIBUTING.md documentation-structure section (FR-013)

**Given** `CONTRIBUTING.md`,
**When** its headings are checked,
**Then** a "Documentation Structure" section exists explaining the docs/ layout
and the contributor's documentation obligation (create/update docs/ article +
README TOC entry + summary).

### VS-16: .editorconfig compliance

**Given** all new `docs/*.md` files and the modified README.md,
**When** CI's editorconfig job runs,
**Then** it passes (LF line endings, correct indent style).

This is verified by the CI pipeline — no manual step needed beyond confirming
the files use LF endings and match `.editorconfig` settings.

### VS-17: No content in README that belongs in docs/ (FR-001, FR-003)

**Given** the slimmed README,
**When** its headings are reviewed,
**Then** only these top-level sections remain:
- `# emergency-ration-webhook` (title + description)
- `## Overview`
- `## Quick Start` (with subsections: Prerequisites, Published Image, Deploy, Verify)
- `## Documentation` (TOC linking to docs/ articles with summaries)
- `## License`

No `## Configuration`, `## Metrics`, `## Failure Modes`, `## Architecture`, etc.

### VS-18: "Development" section removed (Principle XIII)

**Given** the slimmed README,
**When** `grep -c '## Development' README.md` is run,
**Then** the count is 0. The section is replaced with a link to CONTRIBUTING.md.
