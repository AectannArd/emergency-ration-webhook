# Implementation Plan: On-Demand Infrastructure Verification

**Branch**: `005-on-demand-verification` | **Date**: 2026-07-27 | **Spec": [spec.md](./spec.md)

**Input**: Feature specification from `/specs/005-on-demand-verification/spec.md`

## Summary

A second binary (`erw-verify`) in the existing crate that an operator points at a
clean, throwaway Kubernetes cluster via a kubeconfig. It installs the full
webhook stack from the existing `deploy/` manifests, generates a self-signed TLS
certificate in-process, waits for readiness, runs an exhaustive verification
matrix (enforcement scenarios + active fail-closed degradation), tears down
everything it installed, and prints a human-readable or JSON report.

## Technical Context

**Language/Version**: Rust (edition 2024, MSRV **1.89** — same as the existing
crate, recorded in `Cargo.toml`).

**Primary Dependencies**:
- *Existing (reused)*: `kube` 4.2.0 (client, rustls-tls, derive),
  `k8s-openapi` 0.28.0, `tokio` 1 (full), `serde` 1, `serde_json` 1,
  `tracing` 0.1, `tracing-subscriber` 0.3, `thiserror` 2.
- *New*: `rcgen` 0.13 (pure-Rust self-signed certificate generation — no OpenSSL
  dependency; resolved in research R3), `serde_yaml` 0.9 (multi-document YAML
  manifest parsing — resolved in research R2).

**Storage**: N/A. The tool reads `deploy/*.yaml` manifests at compile time via
`include_str!` (embedded in the binary); it writes nothing to disk. All cluster
state is transient and torn down at exit.

**Testing**: unit tests via `#[test]` for the report module (pure rendering),
CLI arg parsing, and scenario-result aggregation. The verification scenarios
themselves are integration tests by nature — they run against a real cluster and
are not unit-testable. CI may exercise the tool against the same `kind` cluster
the existing E2E job uses (a CI workflow concern, not a production-code concern).

**Target Platform**: the operator's machine (Linux, macOS, or Windows). The tool
is built with `cargo build --bin erw-verify` and run locally. It is **not**
deployed into the cluster — the cluster only receives the webhook Deployment
(via the applied manifests).

**Project Type**: CLI tool — a second binary target in the existing crate.

**Performance Goals**: a complete run (setup + all scenarios + teardown)
completes within a practical wall-clock time (SC-006: target < 10 minutes on a
typical cluster). Most time is spent waiting for pod readiness and controller
reconciliation, not in the tool's own logic.

**Constraints**:
- **Single binary, no external dependencies**: the tool must not shell out to
  `kubectl`, `openssl`, or any other external binary. All cluster operations go
  through the kube-rs client; all certificate generation is in-process via
  `rcgen`.
- **Throwaway cluster only**: the tool installs into the default
  `capacity-admission` namespace and actively degrades the installation (killing
  pods, deleting CRDs). It relies on the caller's guarantee that the cluster is
  disposable.
- **No new Kubernetes APIs**: the tool uses the same GA/stable APIs the webhook
  uses (core v1, apps/v1, rbac.authorization.k8s.io/v1,
  admissionregistration.k8s.io/v1, apiextensions.k8s.io/v1).

**Scale/Scope**: ~10 verification scenarios across 2 groups (enforcement +
degradation). Single-cluster, single-run — the tool is not a long-lived process.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The verify tool is a *consumer* of the fail-closed guarantee, not a component that can violate it. It verifies that the webhook rejects under degraded conditions. The tool itself has no admission-control responsibility. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | The tool verifies budget enforcement on real infrastructure; it does not alter the budget semantics. Its test pods use explicit resource requests against the real budget path. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The tool's own failure modes (cluster unreachable, setup timeout, teardown partial failure) are explicitly enumerated in the spec's Edge Cases (FR-013–015, FR-018–019) and mapped to exit codes in the CLI contract. |
| IV | Observability Before Optimisation | ✅ PASS | The tool produces a structured report (per-scenario pass/fail + diagnostics). The tool's own `tracing` output logs each setup/teardown/scenario step. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | The tool is a separate binary, not a modification to the webhook's runtime. It imports only CRD type definitions from the library crate (read-only reuse), keeping the webhook's component boundaries intact. New dependencies (`rcgen`, `serde_yaml`) are minimal and purpose-specific. See Complexity Tracking for the second-binary justification. |
| VI | Integration Test Coverage | ✅ PASS | The tool IS integration test coverage against real infrastructure. Its unit-testable parts (report, arg parsing) get unit tests (Principle VIII). |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | The tool uses the same Kubernetes APIs as the webhook; it works across the same N-2 window (1.34–1.36). The kube-rs client version is shared. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | The report module and CLI arg parser are pure and unit-tested first (RED→GREEN→REFACTOR). The scenario logic is tested via the tool's own execution against a real cluster. |
| IX | Editor Configuration as Code | ✅ PASS | All new files (Rust, Markdown) comply with `.editorconfig`. 4-space indent for Rust, 2-space for YAML, LF line endings. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | The verify tool's CLI flags, kubeconfig usage, scenario list, and report formats are documented in README.md in the same change (FR-015–019 → README section). |
| XI | CI-Green Completion Gate | ✅ PASS | The existing quality gate (`fmt`, `clippy`, `test`) covers the new binary's code. The verify tool's integration scenarios are not part of `cargo test` (they need a real cluster), so they don't break the gate. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | Any transient files during development go to `.temp/`. The tool itself writes nothing to disk at runtime. |

## Project Structure

### Documentation (this feature)

```text
specs/005-on-demand-verification/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── cli.md            # CLI contract (args, exit codes, report format)
└── checklists/
    └── requirements.md  # Spec quality checklist (specify phase)
```

### Source Code (repository root)

```text
src/
├── main.rs                  # (existing) webhook binary entry point — UNCHANGED
├── lib.rs                   # (existing) crate facade — minor: re-export for verify
├── config.rs                # (existing) webhook config — UNCHANGED
├── metrics.rs               # (existing) — UNCHANGED
├── time_util.rs             # (existing) — UNCHANGED
├── crd/                     # (existing) CRD type definitions — UNCHANGED (read by verify)
│   ├── allocation.rs
│   └── cluster_capacity.rs
├── controllers/             # (existing) — UNCHANGED
├── resources/               # (existing) — UNCHANGED
├── webhook/                 # (existing) — UNCHANGED
└── bin/
    └── erw-verify/          # NEW — the on-demand verification tool
        ├── main.rs              # CLI entry point: arg parsing, orchestration, exit codes
        ├── args.rs               # CLI flag definitions and parsing (hand-rolled, matching config.rs style)
        ├── client.rs             # kube::Client construction from kubeconfig path/flag/env
        ├── setup.rs              # install the webhook stack: apply manifests, generate TLS cert, wait for readiness
        ├── teardown.rs           # delete everything installed, in reverse dependency order
        ├── scenarios/            # the verification matrix
        │   ├── mod.rs                # ScenarioRunner trait, scenario result types
        │   ├── enforcement.rs         # US1: admit/deny, budget edges, runtime adjust, dry-run, capacity accuracy, endpoints
        │   └── degradation.rs         # US2: kill pods, delete CRDs, induce stale data; restore between scenarios
        └── report.rs             # pure report rendering: human-readable (default) + JSON (--json)

tests/
├── verify/                  # NEW — unit tests for the verify tool's pure modules
│   ├── report.rs                # report rendering (human + JSON), exit-code derivation
│   └── args.rs                  # CLI arg parsing edge cases
├── bdd/                     # (existing) — UNCHANGED
└── integration/             # (existing) — UNCHANGED

deploy/                     # (existing) — read at compile time via include_str!
├── crds.yaml                # embedded in erw-verify binary
├── rbac.yaml                # embedded
├── deployment.yaml          # embedded
└── webhook-config.yaml      # embedded
```

**Structure Decision**: the verify tool is a **second binary** under Cargo's
auto-discovered `src/bin/` directory, not a separate crate. This lets it share
the library crate's CRD type definitions (`capacity_admission_webhook::crd::*`)
without duplicating them, while keeping its own logic (kube-rs client setup,
manifest application, scenarios, report) isolated from the webhook runtime.

The verify modules are organised by the tool's lifecycle phases: `setup` →
`scenarios` → `teardown`, with `report` as a pure output layer and `client` /
`args` as infrastructure. This mirrors the spec's user stories (US1 =
enforcement scenarios, US2 = degradation scenarios, US3 = report/exit-code).

The existing webhook source tree (`src/main.rs`, `src/webhook/`,
`src/controllers/`, `src/crd/`, `src/resources/`) is **unchanged** — the verify
tool reads the CRD types but does not modify the webhook runtime. This preserves
Constitution Principle V (Separated Concerns): the verify tool is a separate
concern (operational verification), not a new component in the webhook
architecture.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Second binary target (adds `src/bin/`) | The verify tool is a distinct operational concern (operator-initiated, runs outside the cluster, has its own CLI surface and dependencies like `rcgen`) that does not belong in the webhook runtime binary. | A single binary with a `--verify` subcommand was rejected: it would pull `rcgen` and manifest-parsing into the webhook's production dependency tree, violating Principle V (minimal surface) on the critical admission path. Keeping verify logic in the library crate was rejected for the same reason — it would pollute the library's public API with non-webhook code. |

## Constitution Check (Post-Design)

*Re-evaluated against the concrete Phase 1 design artifacts (data-model.md,
contracts/cli.md, quickstart.md), not just the spec intent.*

| # | Principle | Status | Post-Design Evidence |
|---|-----------|--------|----------------------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The degradation scenarios (S9–S11) verify that fail-closed paths reject on real infrastructure. The design does not modify the webhook's admission logic — it only observes and degrades. The run state machine ensures teardown always runs unless `--keep-on-failure` is explicitly set, so the tool never leaves a degraded cluster in a dangerous state. |
| II | Capacity as a Hard Budget | ✅ PASS | Scenarios S3/S4 verify budget edge values (0% and 100%) on real infrastructure. The tool patches the Allocation spec via the same path operators use (`Patch::Merge`), not a back door. No new budget semantics introduced. |
| III | Explicit Failure Mode Configuration | ✅ PASS | The CLI contract (contracts/cli.md) enumerates every error condition → exit code mapping (0/1/2/3). The tool's own failure modes are as enumerable as the webhook's. |
| IV | Observability Before Optimisation | ✅ PASS | The report module produces structured per-scenario output with diagnostics. The tool's `tracing` logs each setup/teardown/scenario step at INFO, with errors at ERROR. JSON output is fully structured. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | Verified: the existing webhook source tree (`src/webhook/`, `src/controllers/`) is UNCHANGED. The verify tool imports only CRD type definitions (read-only). New dependencies (`rcgen`, `serde_yaml`, `base64`) are in the verify binary's scope only — they do not enter the webhook's compilation path because they are only used in `src/bin/erw-verify/`. The Complexity Tracking justification holds: the second binary is the correct separation. |
| VI | Integration Test Coverage | ✅ PASS | The tool IS integration coverage against real infrastructure. Its pure modules (report, args) get unit tests (see quickstart.md). |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | The tool uses the same Kubernetes APIs (core v1, apps/v1, rbac, admissionregistration, apiextensions) and the same kube-rs client version as the webhook. Works across 1.34–1.36. No new API surfaces introduced. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | The report module and arg parser are pure and will be TDD'd (tests written first, watched to fail, then implemented). The scenario runner logic is integration-tested by the tool's own execution. The unit test surface is clearly bounded in the project structure (`tests/verify/`). |
| IX | Editor Configuration as Code | ✅ PASS | All new Markdown artifacts comply with `.editorconfig` (LF endings, 2-space for lists, no trailing whitespace). New Rust files will use 4-space indent per `rustfmt`. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | The CLI contract flags (`--kubeconfig`, `--json`, `--keep-on-failure`, `--timeout-secs`), exit codes, scenario inventory, and the tool's throwaway-cluster requirement are all README-documentable. The implementation tasks MUST include a README.md update (same-PR obligation). |
| XI | CI-Green Completion Gate | ✅ PASS | The existing `cargo test` quality gate covers the new binary's unit tests (`tests/verify/`). The verify tool's integration scenarios (S1–S11) are not part of `cargo test` (they need a real cluster) and do not break the gate. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | No transient files are written to tracked directories during the design or (future) implementation. The tool itself writes nothing to disk at runtime. |

**Post-design verdict**: no principle violations introduced by the concrete
design. The plan advances to `/speckit-tasks`.
