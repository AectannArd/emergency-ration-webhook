# Feature Specification: ERW Verify Image Build Automation

**Feature Branch**: `009-erw-image-build-automation`

**Created**: 2026-07-28

**Status**: Draft

**Input**: User description: "The real infra test script should be able to read a
`.env` file in the root of the repository. The `.env` file should contain all
parameters necessary to push a produced docker image. The `.env` file should
contain a variable pointing to the target kube-config. Image building should be
a part of the real infra test process — should be scripted inside."

## User Scenarios & Testing *(mandatory)*

### User Story 1 — One-Command Real-Infra Test (Priority: P1)

An operator wants to run the full `erw-verify` test suite against a real
Kubernetes cluster with a single command — without manually building the
container image, pushing it to a registry, editing manifests to reference the
registry, or passing long CLI flags. They create a `.env` file in the repository
root containing the registry endpoint, image name, credentials, and kubeconfig
path, then run a single command. The tool reads the `.env`, builds the webhook
image from the `Dockerfile` in the repo, pushes it to the configured registry,
embeds the fully-qualified image reference into the deployment manifests it
applies, runs the full verification lifecycle (setup → scenarios → teardown),
and prints the report.

**Why this priority**: the current tool requires at least four manual steps
(build, push, patch manifest, run) across two machines before it can verify
anything. This defeats the spec-005 goal (SC-001: "a single command, with no
manual setup"). Making the whole pipeline one command is the minimum viable
improvement.

**Independent Test**: run the tool with a `.env` present; confirm the image is
built, pushed, referenced in the applied Deployment, and all scenarios execute.

**Acceptance Scenarios**:

1. **Given** a `.env` file in the repository root with registry, image, and
   kubeconfig variables set, **When** the operator runs the tool, **Then** the
   tool reads the `.env` and resolves all parameters before any network action.
2. **Given** resolved registry parameters, **When** the tool builds the image,
   **Then** it produces a container image tagged with the fully-qualified
   registry path (e.g. `cr.yandex/<id>/capacity-admission-webhook:latest`).
3. **Given** a built image, **When** the tool pushes it, **Then** the image is
   available in the target registry and pullable by the cluster nodes.
4. **Given** a pushed image, **When** the tool applies the deployment manifests,
   **Then** the Deployment references the fully-qualified image (not a bare
   local name) and the pods successfully pull it.
5. **Given** the image is deployed and ready, **When** the verification scenarios
   run, **Then** the full suite executes and the report is printed.

---

### User Story 2 — `.env` as Configuration Contract (Priority: P2)

An operator or CI engineer wants a documented `.env.example` file that lists
every variable the tool consumes, with sensible placeholder values and inline
comments. This serves as the configuration contract: an operator copies it to
`.env`, fills in their values, and the tool works. The `.env.example` is
committed to the repository (`.env` itself is git-ignored and never committed).

**Why this priority**: without a documented `.env.example`, operators have no
way to discover which variables the tool needs. User Story 1 delivers the
functionality; this story makes it discoverable and reproducible.

**Independent Test**: copy `.env.example` to `.env`, fill in real values, run
the tool — it works with no further configuration.

**Acceptance Scenarios**:

1. **Given** the repository, **When** the operator looks at the repo root,
   **Then** a `.env.example` file exists listing every variable the tool reads.
2. **Given** `.env.example`, **When** the operator copies it to `.env` and fills
   in real values, **Then** the tool reads all parameters from `.env` without any
   CLI flags.
3. **Given** a `.env` file with a missing required variable, **When** the tool
   starts, **Then** it fails fast with a clear error naming the missing variable.

---

### User Story 3 — Skip Build When Image Already Exists (Priority: P3)

An operator iterates on the verification suite (not the webhook code) and wants
to re-run the tests without rebuilding the image each time. They set a flag or
env var to skip the build+push step. The tool reuses the already-pushed image
and proceeds directly to setup → scenarios → teardown.

**Why this priority**: the image build is the slowest step (2-10 minutes). When
iterating on test logic, re-building every time is wasteful. This is a
quality-of-life feature on top of the core pipeline.

**Independent Test**: run with `--skip-build` (or env var); confirm no Docker
build runs and the existing image is used.

**Acceptance Scenarios**:

1. **Given** an image already pushed to the registry, **When** the operator runs
   the tool with the skip-build flag, **Then** the tool skips build + push and
   proceeds directly to deploy + verify.
2. **Given** the skip-build flag is set and the image does NOT exist in the
   registry, **When** the tool deploys, **Then** pods fail with ImagePullBackOff
   and the tool reports a setup failure (the operator is responsible for this
   contradiction).

---

### Edge Cases

- **`.env` not present**: the tool must check for `.env` at startup. If the
  file is missing AND required registry/image variables are not set via CLI or
  ambient environment, the tool must fail fast with a clear message directing the
  operator to copy `.env.example`.
- **`.env` present but incomplete**: a `.env` file with only some variables filled
  in must fail fast, naming the specific missing variable.
- **Registry authentication failure**: if `docker push` fails with an auth error,
  the tool must report it as a build-phase error (exit code > 0) and not attempt
  to deploy.
- **Docker not installed**: if the `docker` binary is not on `PATH`, the tool
  must fail fast before attempting any build. If `--skip-build` is set, Docker is
  not needed.
- **Build failure**: if `docker build` exits non-zero, the tool must report the
  build error and not proceed to push or deploy.
- **Existing local kubeconfig override**: a `--kubeconfig` CLI flag must take
  precedence over the `.env` kubeconfig variable, which must take precedence
  over the ambient `KUBECONFIG` environment variable, which falls back to
  `Config::infer` (the existing spec-005 precedence chain).
- **`.env` with quoted values**: the parser must handle values wrapped in
  single or double quotes (`KEY="value with spaces"`, `KEY='value'`).
- **Image already exists in registry with same tag**: the tool must push
  unconditionally (overwrite) unless `--skip-build` is set. There is no
  "already up to date" check — the operator guarantees freshness by running the
  tool.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The tool MUST read a `.env` file from the repository root at
  startup, parsing `KEY=VALUE` lines into the configuration map.
- **FR-002**: The `.env` file MUST support the following variables: a registry
  endpoint, an image name, and a kubeconfig path. These are the minimum required
  for the full build → push → verify pipeline.
- **FR-003**: The tool MUST support a `.env.example` file committed to the
  repository as the documented configuration contract. It lists every variable
  with placeholder values and inline comments.
- **FR-004**: Precedence for all configuration values MUST be: CLI flag → `.env`
  file → ambient environment variable → compiled default.
- **FR-005**: The tool MUST build the webhook container image from the
  repository's `Dockerfile` using the `docker` CLI, tagging it with the
  fully-qualified registry path resolved from configuration.
- **FR-006**: The tool MUST push the built image to the configured registry
  before deploying manifests.
- **FR-007**: The tool MUST embed the fully-qualified image reference (registry +
  image + tag) into the Deployment manifest at apply time, overriding the bare
  image name in `deploy/deployment.yaml`.
- **FR-008**: The tool MUST support a `--skip-build` flag (and corresponding env
  var) that skips the build + push phases and proceeds directly to deploy +
  verify.
- **FR-009**: The tool MUST fail fast with a clear, actionable error message if
  any required configuration variable is missing or if `docker` is not on `PATH`
  (and `--skip-build` is not set).
- **FR-010**: The `.env` parser MUST handle quoted values (single and double
  quotes) and ignore comment lines starting with `#`.
- **FR-011**: The build+push phase MUST execute before the cluster setup phase.
  A failure in build or push MUST abort the run before any cluster resources are
  created.
- **FR-012**: The kubeconfig path from `.env` MUST be resolved relative to the
  repository root if it is not an absolute path.
- **FR-013**: The tool MUST log each phase of the pipeline (env loaded, image
  building, image pushing, deploying, verifying) with enough detail for an
  operator to follow progress.

### Key Entities

- **Build Configuration**: the set of variables that control the image build:
  registry endpoint, image name, image tag, and the path to the Dockerfile (the
  repo root by default). Resolved from `.env` / CLI / environment.
- **Verify Pipeline**: the extended lifecycle of the tool: load `.env` →
  (optionally build + push image) → connect to cluster → pre-flight → setup →
  scenarios → teardown → report. The build+push phase is the new prefix; the
  rest is the existing spec-005 lifecycle.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can run the full real-infra verification pipeline
  (build image, push to registry, deploy, verify, teardown) with a single
  command and a `.env` file — zero manual steps between `.env` and report.
- **SC-002**: Every configuration variable the tool consumes is documented in a
  committed `.env.example` file with placeholder values and descriptions.
- **SC-003**: A missing required variable causes the tool to exit within 1 second
  with a non-zero exit code and an error message naming the missing variable.
- **SC-004**: The `--skip-build` flag reduces the time-to-report by eliminating
  the Docker build+push phase (savings proportional to build time, typically
  2-10 minutes).

## Assumptions

- The operator has Docker installed and authenticated to the target registry on
  the machine running the tool (the tool calls `docker build` / `docker push`;
  it does not implement registry auth itself).
- The `.env` file lives at the repository root (same directory as `Cargo.toml`
  and `Dockerfile`).
- `.env` is git-ignored and never committed; `.env.example` is committed as the
  template.
- The Dockerfile is already present in the repo root and builds correctly (the
  tool uses it as-is; it does not generate or modify the Dockerfile).
- The image tag defaults to `latest`; custom tags are a plan-phase decision.
- The tool runs on the same machine that has Docker and network access to both
  the registry and the target cluster.
- The existing spec-005 lifecycle (setup → scenarios → teardown → report) is
  unchanged; this spec adds the build+push prefix and the `.env`-driven
  configuration layer.
