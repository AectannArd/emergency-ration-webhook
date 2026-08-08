# Contract: docs/ Article Structure

This contract defines the structure every `docs/*.md` article MUST follow. The
implementing agent creates each article to satisfy this contract.

## File naming

- Descriptive, kebab-case, `.md` extension.
- No numeric prefixes, no subdirectories (flat `docs/` directory).
- One article per major capability.

## Article structure

Every `docs/*.md` article MUST have:

### 1. Title heading

```markdown
# <Topic Name>
```

Matches the capability name. Example: `# Configuration Reference`.

### 2. Back-link to README

A single line near the top linking back to the README:

```markdown
[← Back to README](../README.md)
```

### 3. Content body

The full reference content extracted from the README, with internal links updated
to point to other `docs/` articles where the target moved. Code blocks, tables,
and examples are preserved verbatim from the source README section.

### 4. Source links

Where the README referenced a source file (e.g. `source: [src/config.rs](./src/config.rs)`),
the link is updated to the correct relative path from `docs/`:
`../src/config.rs`.

## Cross-article links

Articles link to each other using relative paths within `docs/`:

```markdown
See [Enforcement Modes](./enforcement-modes.md) for dry-run behaviour.
```

## What articles MUST NOT contain

- No duplicate of README quick-start content (quick-start stays in README only).
- No contributor content (build/test commands — that's CONTRIBUTING.md).
- No content that isn't from the current README (no new content authored — this
  is a reorganization, not a documentation rewrite).

## Article manifest (11 articles)

| File | Title | Source sections |
|------|-------|-----------------|
| `docs/README.md` | Documentation Index | (new — index file) |
| `docs/deployment.md` | Deployment Guide | Build the Image, TLS Provisioning, deploy detail |
| `docs/configuration.md` | Configuration Reference | CLI Flags, Precedence, Allocation CRD, ClusterCapacity CRD, Budget Adjustment, Per-Resource Overrides, Budget Edge Cases |
| `docs/node-exclusion.md` | Node Exclusion | Node Exclusion (spec-006/007) |
| `docs/enforcement-modes.md` | Enforcement Modes | Enforcement Modes (spec-004) |
| `docs/workload-exclusion.md` | Workload Exclusion | Workload Exclusion (spec-008) |
| `docs/observability.md` | Metrics & Observability | HTTP Endpoints, Prometheus Metrics, Structured Logging, Rejection Messages |
| `docs/failure-modes.md` | Failure Modes | Failure Modes, Webhook Self-Admission (Bootstrap) |
| `docs/kubernetes-compatibility.md` | Kubernetes Compatibility | Kubernetes Compatibility |
| `docs/architecture.md` | Architecture | Architecture (3-component operator) |
| `docs/erw-verify.md` | On-Demand Verification (erw-verify) | Build, Configure, Run, Usage, CLI Flags, Exit Codes, Scenario Inventory |
| `docs/equalizer.md` | Multi-Cluster Capacity Equalizer | Multi-Cluster Capacity Equalizer (spec-013) |
