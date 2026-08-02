# Implementation Plan: Docker Hub Image Publishing

**Branch**: `011-docker-hub-publishing` | **Date**: 2026-08-02 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/011-docker-hub-publishing/spec.md`

## Summary

Add a GitHub Actions workflow (`.github/workflows/publish.yml`) that publishes
the webhook container image to Docker Hub (`aectann/emergency-ration-webhook`)
as a multi-arch manifest (`linux/amd64` + `linux/arm64`). The workflow triggers
on semantic-version git tags (`v1.0.0` for stable, `v1.0.0-rc.1` for
pre-release), re-runs the quality gate, then builds and pushes via
`docker/build-push-action`. Stable tags also update `latest`; pre-release tags
do not. A `workflow_dispatch` input supports manual ad-hoc publishing. Docker
Hub credentials are read from GitHub secrets `DOCKERHUB_USERNAME` and
`DOCKERHUB_TOKEN`.

## Technical Context

**Language/Version**: YAML (GitHub Actions workflow). No Rust changes — this
feature touches only `.github/workflows/` and documentation.

**Primary Dependencies**:
- *GitHub Actions (pinned at current major)*:
  - `actions/checkout@v4`
  - `docker/setup-qemu-action@v3` — QEMU for arm64 cross-build
  - `docker/setup-buildx-action@v3` — multi-platform buildx builder
  - `docker/login-action@v3` — non-interactive Docker Hub auth
  - `docker/metadata-action@v5` — tag derivation from git refs
  - `docker/build-push-action@v6` — multi-arch build + push
  - `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2` — quality gate
    (mirrors `ci.yml`)

**Storage**: none. The workflow is stateless; the only durable artifact is the
image manifest on Docker Hub.

**Testing**: the workflow is exercised by triggering it (tag push or manual
dispatch). There are no unit tests for a YAML workflow — validation is via
[quickstart.md](./quickstart.md) scenarios (pull the image on both
architectures, inspect the manifest).

**Target Platform**: GitHub Actions (`ubuntu-latest` runners). The published
image targets Linux/amd64 and Linux/arm64 (the webhook's deployment target).

**Project Type**: new CI workflow + documentation update. No production code
changes.

**Performance Goals**: the publish workflow should complete within 15 minutes
(QEMU-emulated arm64 build is the slow step; GHA cache mitigates Rust compile
time).

**Constraints**:
- **No secrets in tracked files**: credentials live in GitHub Secrets only.
- **Fork-safe**: the workflow is gated on `github.repository ==
  'AectannArd/emergency-ration-webhook'`.
- **Quality gate enforcement**: a tag whose commit fails the quality gate must
  not publish.
- **`.editorconfig` compliance**: the workflow YAML follows 2-space indent,
  Unix line endings.

**Scale/Scope**: 1 new workflow file (`.github/workflows/publish.yml`, ~80
lines) + README update (image pull instructions, per Principle X).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The publish workflow does not touch the admission path. A failed build or auth failure produces no image (fail-closed by the build system). |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | No change to budget semantics. |
| III | Explicit Failure Modes | ✅ PASS | All failure modes are enumerated (missing secret, auth fail, build fail, per-arch build fail, push fail) with clear error propagation to the workflow run status (research R3, contracts/workflow.md). |
| IV | Observability Before Optimisation | ✅ PASS | Each step is a named workflow step; failures surface as failed steps with logs. Docker Hub's registry serves as the observable artifact. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS | No new Rust dependencies. The feature is a single declarative workflow file using canonical GitHub Actions. No coupling to the webhook's runtime code. |
| VI | Integration Test Coverage | ✅ PASS | The workflow is validated by the quickstart scenarios (tag push → pullable image on both architectures). There is no unit-testable logic in a YAML workflow. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | No Kubernetes APIs used. The published image's K8s support is unchanged. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | N/A — no production code is written. The workflow IS the test (triggering it validates the publish path). The quality gate (fmt/clippy/test) is re-run before publishing. |
| IX | Editor Configuration as Code | ✅ PASS | The new YAML workflow file follows `.editorconfig` (2-space indent, LF, final newline). The `.editorconfig` already has a YAML section. |
| X | User-Facing Functionality is Documented in README.md | ✅ PASS | FR-015 requires README documentation of the published image location and pull command. The plan includes a README delta. |
| XI | CI-Green Completion Gate | ✅ PASS | The publish workflow's `quality` job re-runs the gate; `publish` depends on it. A red quality gate blocks publishing. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | No scratch files needed; the workflow is committed, not transient. |
| XIII | Usage/Contribution Doc Separation | ✅ PASS | Image pull instructions (operator concern) go in README; workflow details (contributor concern) go in CONTRIBUTING.md. The README links to the workflow for contributors. |

## Project Structure

### Documentation (this feature)

```text
specs/011-docker-hub-publishing/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   ├── workflow.md      # Publish workflow contract
│   └── secrets.md       # Docker Hub secret contract
└── tasks.md             # Phase 2 output (not created by /speckit-plan)
```

### Source Code (repository root)

```text
.github/workflows/
├── ci.yml               # Unchanged (existing quality + e2e workflow)
└── publish.yml          # NEW: tag-triggered multi-arch Docker Hub publish

README.md                # Modified: add "Published Image" section (pull command, supported architectures)
CONTRIBUTING.md          # Modified: add "Publishing" section (how releases are cut, secret setup link)
```

**Structure Decision**: a single new workflow file `publish.yml` sits alongside
the existing `ci.yml`. It is self-contained (defines its own `quality` job
rather than depending on `ci.yml` cross-workflow, per research R5). The
Dockerfile is reused unchanged. No source code (`src/`) is touched.

## Complexity Tracking

No constitution violations to justify.
