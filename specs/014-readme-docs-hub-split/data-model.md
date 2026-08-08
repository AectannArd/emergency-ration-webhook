# Phase 1: Data Model — Content-Trace Mapping & Accuracy Rules

## Overview

For a documentation reorganization, the "data model" is the **content-trace
mapping** (old location → new location for every README section) and the
**accuracy verification rules** (each documented value cross-checked against
source code). These are the "tests" the implementing agent validates against.

## Content-Trace Mapping

Every section of the current README MUST appear in exactly one destination.
The implementing agent verifies this mapping is complete — no section is lost.

| # | Old README section (heading) | Lines | Destination |
|---|------------------------------|-------|-------------|
| 1 | `# emergency-ration-webhook` (title + blurb) | 1-5 | README (stays, trimmed) |
| 2 | Intro paragraph (lines 7-19) | 7-19 | README (stays, trimmed — remove "single entry point" para that contradicts hub model) |
| 3 | `## Overview` | 21-44 | README (stays — this is the project description) |
| 4 | `## Quick Start` | 46-49 | README (stays — self-contained quick-start) |
| 5 | `### Prerequisites` | 51-63 | README (stays — part of quick-start) |
| 6 | `### Published Image` | 65-88 | README quick-start (stays — compressed to 2-3 lines + link to `docs/deployment.md` for full tag conventions) |
| 7 | `### Build the Image` | 90-108 | `docs/deployment.md` (moves — build is deep reference) |
| 8 | `### Deploy to Kubernetes` | 110-169 | README quick-start (stays — the 6-step deploy sequence is the core quick-start; keep compressed, link to `docs/deployment.md` for TLS detail) |
| 9 | `#### TLS Provisioning` | 171-218 | `docs/deployment.md` (moves — deep reference) |
| 10 | `### Verify` | 220-255 | README quick-start (stays — the verify commands are the end of quick-start) |
| 11 | `## Configuration` heading + intro | 257-258 | README summary (1-2 sentences + link to `docs/configuration.md`) |
| 12 | `### CLI Flags & Environment Variables` | 259-276 | `docs/configuration.md` |
| 13 | `### Precedence` | 277-292 | `docs/configuration.md` |
| 14 | `### Allocation CRD` | 294-326 | `docs/configuration.md` |
| 15 | `### ClusterCapacity CRD` | 328-355 | `docs/configuration.md` |
| 16 | `### Node Exclusion` | 357-460 | `docs/node-exclusion.md` |
| 17 | `### Adjusting the Budget at Runtime` | 462-479 | `docs/configuration.md` |
| 18 | `### Per-Resource Budget Overrides (spec-012)` | 481-523 | `docs/configuration.md` |
| 19 | `### Enforcement Modes (Enforce / Dry-Run)` | 525-568 | `docs/enforcement-modes.md` |
| 20 | `### Workload Exclusion` | 570-630 | `docs/workload-exclusion.md` |
| 21 | `### Budget Edge Cases` | 632-643 | `docs/configuration.md` |
| 22 | `## Metrics & Observability` | 645-666 | README summary (1-2 sentences + link to `docs/observability.md`) |
| 23 | `### HTTP Endpoints` | 647-665 | `docs/observability.md` |
| 24 | `### Prometheus Metrics` | 667-693 | `docs/observability.md` |
| 25 | `### Structured Logging` | 695-738 | `docs/observability.md` |
| 26 | `### Rejection Messages` | 740-764 | `docs/observability.md` |
| 27 | `## Failure Modes` | 766-801 | `docs/failure-modes.md` |
| 28 | `## Kubernetes Compatibility` | 803-818 | `docs/kubernetes-compatibility.md` |
| 29 | `### Webhook Self-Admission (Bootstrap)` | 819-833 | `docs/failure-modes.md` (closely related to fail-closed behaviour; also cross-linked from `docs/kubernetes-compatibility.md`) |
| 30 | `## Architecture` | 835-866 | `docs/architecture.md` |
| 31 | `## On-Demand Verification (erw-verify)` | 868-883 | README summary (1-2 sentences + link to `docs/erw-verify.md`) |
| 32 | `### Build` through `### Exit Codes` | 884-985 | `docs/erw-verify.md` |
| 33 | `### Scenario Inventory` | 987-1035 | `docs/erw-verify.md` |
| 34 | `## Development` | 1037-1108 | **REMOVED** — duplicate of CONTRIBUTING.md (§ Building, Testing, Quality Gate, Project Structure). Replaced with a one-line link. |
| 35 | `## Multi-Cluster Capacity Equalizer (spec-013)` | 1110-1172 | `docs/equalizer.md` |
| 36 | `## License` | 1174-1176 | README (stays) |

## Accuracy Verification Rules

Each rule (VR-NNN) cross-checks a documented value against its source file. The
implementing agent MUST verify each one. If a discrepancy is found, the code is
the source of truth (FR-012).

### Configuration values (source: `src/config.rs`)

- **VR-001**: The 7 CLI flags in `docs/configuration.md` match `src/config.rs`
  field names: `port`, `tls_cert_file`, `tls_key_file`, `decision_timeout_ms`,
  `capacity_freshness_timeout_secs`, `namespace`, `metrics_port`.
- **VR-002**: Default values match: port=8443, tls-cert-file=/tls/tls.crt,
  tls-key-file=/tls/tls.key, decision-timeout-ms=100,
  capacity-freshness-timeout-secs=30, namespace=capacity-admission,
  metrics-port=9090.
- **VR-003**: Precedence order (flag → env → default) matches `src/config.rs`
  resolution logic.

### CRD fields (source: `src/crd/allocation.rs`, `src/crd/cluster_capacity.rs`)

- **VR-004**: Allocation spec fields match: `budgetPercent`, `enforcementMode`,
  `excludedNamespaces`, `excludedPriorityClasses`, `cpuBudgetPercent`,
  `memoryBudgetPercent`.
- **VR-005**: Allocation status fields match: `allocatedCpuMilli`,
  `allocatedMemoryBytes`, `ceilingCpuMilli`, `ceilingMemoryBytes`,
  `utilizationPercentCpu`, `utilizationPercentMemory`, `lastUpdated`,
  `effectiveCpuBudgetPercent`, `effectiveMemoryBudgetPercent`.
- **VR-006**: ClusterCapacity spec fields match: `nodeSelectors`.
- **VR-007**: ClusterCapacity status fields match: `totalAllocatableCpuMilli`,
  `totalAllocatableMemoryBytes`, `nodeCount`, `lastUpdated`, `excludedNodeCount`,
  `excludedByUnschedulable`, `excludedBySelector`.
- **VR-008**: Default budgetPercent is 80 (auto-created singleton).
- **VR-009**: Default enforcementMode is `enforce` (absent → enforce).

### Metrics (source: `src/metrics.rs`)

- **VR-010**: All 8 metric names match `src/metrics.rs`:
  `capacity_admission_verdicts_total`, `capacity_admission_exemptions_total`,
  `capacity_admission_decision_duration_seconds`,
  `capacity_admission_capacity_freshness_seconds`,
  `capacity_admission_allocation_ratio`, `capacity_admission_total_allocatable`,
  `capacity_admission_current_allocation`, `capacity_admission_ceiling`.
- **VR-011**: Label vocabularies match: `resource ∈ {cpu, memory}`, `verdict ∈
  {allow, deny, dry_run_deny, error}`, `reason ∈ {namespace, priority_class,
  webhook_namespace}`.
- **VR-012**: Histogram buckets match: 0.005, 0.01, 0.025, 0.05, 0.075, 0.1,
  0.25, 0.5, 1.0.

### Failure modes (source: `src/webhook/error.rs`, `src/webhook/handler.rs`)

- **VR-013**: Failure-mode reason slugs match: `capacity_data_stale`,
  `capacity_data_missing`, `deserialisation_failure`, `quantity_parse_failure`,
  `timeout`, `internal_error`, `unknown`, `over_budget`.
- **VR-014**: HTTP status codes match: 403 (over-budget), 400 (malformed),
  500 (all other fail-closed).

### erw-verify CLI (source: `src/bin/erw-verify/args.rs`)

- **VR-015**: CLI flags match: `--registry`, `--image-name`, `--image-tag`,
  `--skip-build`, `--kubeconfig`, `--json`, `--keep-on-failure`, `--timeout-secs`.
- **VR-016**: Exit codes match: 0 (pass), 1 (scenario fail), 2 (setup error), 3
  (teardown partial), 4 (config error).
- **VR-017**: `.env` variables match: `ERW_REGISTRY`, `ERW_IMAGE_NAME`,
  `ERW_IMAGE_TAG`, `ERW_KUBECONFIG`, `ERW_SKIP_BUILD`, `VERIFY_TIMEOUT_SECS`.

### Scenario inventory (source: `src/bin/erw-verify/scenarios/`)

- **VR-018**: Scenario IDs match: S1-S9 (enforcement), S10-S11 (degradation),
  E1-E5 (equalizer). Each scenario description matches the scenario module's
  documented assertion.

### EqualizerConfig CRD (source: `src/crd/equalizer_config.rs` or equivalent)

- **VR-019**: EqualizerConfig spec fields match: `cpuTargetBudgetPercent`,
  `memoryTargetBudgetPercent`, `targets[].name`,
  `targets[].kubeconfigSecretRef.{name,key,namespace}`.
- **VR-020**: Per-cluster status states match: `healthy`, `over`, `unreachable`,
  `config-error`. Fleet conditions match: `healthy`, `compensating`, `degraded`.

### Structured log fields (source: `src/webhook/handler.rs`)

- **VR-021**: Log field names match: `workload`, `operation`, `decision`,
  `resource_type`, `allocated`, `requested`, `projected`, `ceiling`,
  `budget_percent`, `effective_cpu_budget_percent`,
  `effective_memory_budget_percent`, `enforcement_mode`, `exemption_reason`,
  `freshness_seconds`, `latency_ms`, `reason`.

## Link Integrity Rules

- **LR-001**: Every `docs/*.md` file is linked from the README TOC.
- **LR-002**: Every README TOC link resolves to an existing `docs/*.md` file.
- **LR-003**: Every `](#anchor)` link remaining in the slimmed README resolves
  to a heading still in the README.
- **LR-004**: The `ARTIFACTS.md` reference to `README.md#verification-scenarios`
  is updated to `docs/erw-verify.md#scenario-inventory`.
- **LR-005**: The `specs/006.../quickstart.md` reference to `README.md#quick-start`
  remains valid (Quick Start anchor stays in README).
- **LR-006**: No absolute GitHub URLs in any docs/*.md file (all relative).
- **LR-007**: `docs/README.md` lists all 11 articles with one-line descriptions.
