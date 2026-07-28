# Implementation Plan: ERW Verify Image Build Automation

**Branch**: `009-erw-image-build-automation` | **Date**: 2026-07-28 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/009-erw-image-build-automation/spec.md`

## Summary

Integrate Docker image build + push into the `erw-verify` binary's lifecycle,
driven by a `.env` file at the repository root. The tool reads build/push
parameters (registry endpoint, image name, kubeconfig path), shells out to
`docker build` and `docker push` via `std::process::Command`, substitutes a
placeholder image reference in `deploy/deployment.yaml` at apply time, then
proceeds with the existing spec-005 verification lifecycle. A `--skip-build`
flag opts out of the build+push phase.

## Technical Context

**Language/Version**: Rust (edition 2024, MSRV 1.89 — same as the existing crate).

**Primary Dependencies**:
- *Existing (reused)*: `kube` 4.2.0, `k8s-openapi` 0.28.0, `tokio` 1,
  `serde_json` 1, `tracing` 0.1. All already in the erw-verify binary's
  dependency tree.
- *New*: none. The `.env` parser is hand-rolled (dependency-free, matching
  the existing `args.rs` style per Constitution Principle V). Docker operations
  use `std::process::Command` (std-only, no new crates).

**Storage**: reads `.env` from the repository root at runtime. The `.env` file
is not embedded — it is read from disk (unlike the deploy manifests, which are
`include_str!`-embedded). A `.env.example` file is committed to the repo root.

**Testing**: unit tests for the `.env` parser (pure function: `&str →
BTreeMap<String, String>`) and the image-reference resolver (configuration →
fully-qualified image string). Docker build+push cannot be unit-tested; it is
exercised by running the tool.

**Target Platform**: the operator's machine (must have `docker` on `PATH`).
Same platform constraints as spec-005 (Linux/macOS/Windows).

**Project Type**: modification to the existing `erw-verify` CLI binary.

**Performance Goals**: the build+push phase adds 2-10 minutes (Docker build
time). The verification lifecycle itself is unchanged.

**Constraints**:
- **Single binary**: no external scripts. Docker build+push via
  `std::process::Command`, `.env` parser hand-rolled.
- **Docker prerequisite**: the tool requires `docker` on `PATH` unless
  `--skip-build` is set.
- **`.env` git-ignored**: `.env` is never committed; `.env.example` is.

**Scale/Scope**: ~3 new modules (env parsing, image build, image push) +
modifications to `args.rs` (new config fields), `main.rs` (pipeline wiring),
and `deploy/deployment.yaml` (placeholder image).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | Build automation is a pre-verify phase; it does not touch the admission path. A build/push failure aborts before any cluster resource is created. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | No change to budget semantics. |
| III | Explicit Failure Mode Configuration | ✅ PASS | Build/push failures are enumerated (docker missing, build failed, push auth failed, .env missing) with clear error messages and early abort. |
| IV | Observability Before Optimisation | ✅ PASS | Each pipeline phase is logged via `tracing` (env loaded, building, pushing, deploying). |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | No new dependencies. `.env` parser is hand-rolled. Docker via std::process::Command. The build phase is a separate module from the verify lifecycle. |
| VI | Integration Test Coverage | ✅ PASS | The `.env` parser and image resolver are unit-tested. The Docker integration is exercised by running the tool against a real cluster. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | No new Kubernetes APIs used. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | The `.env` parser and image resolver are pure functions with deterministic unit tests, written first (RED→GREEN→REFACTOR). |
| IX | Editor Configuration as Code | ✅ PASS | New files (`.env.example`, env module) follow `.editorconfig` rules. |
| X | User-Facing Functionality is Documented in README.md | ✅ PASS | The `.env.example` is the configuration contract; README must document the one-command pipeline. |
| XI | CI-Green Completion Gate | ✅ PASS | Unit tests for `.env` parser and image resolver run in CI. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | Build logs go to `.temp/` (existing pattern). |

## Project Structure

### Documentation (this feature)

```text
specs/009-erw-image-build-automation/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── env.md           # .env variable contract
└── tasks.md             # Phase 2 output (not created by /speckit-plan)
```

### Source Code (repository root)

```text
src/bin/erw-verify/
├── main.rs              # Modified: wire build+push phase into pipeline
├── args.rs              # Modified: add build/skip-build/image config fields
├── env_config.rs        # NEW: .env file parser (pure: &str → BTreeMap)
├── image.rs             # NEW: Docker build + push via std::process::Command
├── setup.rs             # Modified: substitute image placeholder at apply time
└── ...                  # (unchanged: client.rs, scenarios/, teardown.rs, report.rs)

.env.example             # NEW: committed template with all variables
.env                     # (git-ignored, operator-provided)
deploy/
└── deployment.yaml      # Modified: IMAGE_PLACEHOLDER in image field
```

**Structure Decision**: the build+push logic lives in two new modules inside
the existing `erw-verify` binary directory. `env_config.rs` is the pure `.env`
parser (unit-testable). `image.rs` wraps the Docker CLI calls. The existing
`setup.rs` is modified to substitute the image placeholder when applying the
Deployment manifest. This keeps all changes inside the verify binary — the
webhook's production code is untouched.

## Complexity Tracking

No constitution violations to justify.
