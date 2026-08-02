# Feature Specification: Docker Hub Image Publishing

**Feature Branch**: `011-docker-hub-publishing`

**Created**: 2026-08-02

**Status**: Draft

**Input**: User description: "add artifact publishing to Docker Hub"

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Semver Release from Git Tag (Priority: P1)

A maintainer wants to cut a release by pushing a semantic-version git tag
(e.g. `v1.0.0`, `v1.2.3`). The tag push triggers a CI workflow that builds the
webhook container image from the repository's `Dockerfile` for both
`linux/amd64` and `linux/arm64`, tags the multi-arch manifest with the exact
version (`v1.0.0`) and the moving `latest` tag, and publishes both to the
Docker Hub repository `aectann/emergency-ration-webhook`. The maintainer does
nothing else — the tag IS the release.

**Why this priority**: the project has no artifact distribution channel today.
Operators who want to deploy the webhook must build the image themselves from
source. A tag-triggered publish to the most common registry (Docker Hub) is the
minimum viable distribution path and is the canonical release mechanism.

**Independent Test**: push a semver tag; confirm both architectures are
published to Docker Hub under the version tag and `latest`, and that the image
is pullable on both amd64 and arm64 hosts.

**Acceptance Scenarios**:

1. **Given** the repository with a passing CI pipeline on `main`, **When** a
   maintainer pushes a tag matching `v[0-9]+.[0-9]+.[0-9]+` (e.g. `v1.0.0`),
   **Then** the publish workflow runs and produces a multi-arch manifest at
   `aectann/emergency-ration-webhook:v1.0.0`.
2. **Given** a semver tag publish, **When** the manifest is pushed, **Then**
   `aectann/emergency-ration-webhook:latest` is also updated to point at the
   same multi-arch manifest as the version tag.
3. **Given** the workflow has run, **When** an operator runs `docker pull
   aectann/emergency-ration-webhook:v1.0.0` on an arm64 host, **Then** the
   correct architecture variant is pulled and runs.
4. **Given** the workflow has run, **When** an operator runs `docker pull
   aectann/emergency-ration-webhook:v1.0.0` on an amd64 host, **Then** the
   correct architecture variant is pulled and runs.

---

### User Story 2 — Pre-Release and Metadata Tags (Priority: P2)

A maintainer wants pre-release tags (`v1.0.0-rc.1`, `v1.0.0-beta.2`) to also
trigger a publish, but pre-release images MUST NOT update `latest` — only the
explicit version tag is pushed. This lets maintainers distribute release
candidates for testing without polluting the stable `latest` pointer.

**Why this priority**: without pre-release tag handling, every RC publish would
overwrite `latest`, pulling testers onto an unreviewed image. This story
protects the `latest → stable` invariant while still distributing pre-release
artifacts. It is a refinement of the P1 publish path, not a separate channel.

**Independent Test**: push a pre-release tag; confirm the version tag is
published but `latest` is unchanged.

**Acceptance Scenarios**:

1. **Given** a tag matching `v[0-9]+.[0-9]+.[0-9]+-<prerelease>`
   (e.g. `v1.0.0-rc.1`), **When** the publish workflow runs, **Then** the
   version-tagged image `aectann/emergency-ration-webhook:v1.0.0-rc.1` is
   published as a multi-arch manifest.
2. **Given** a pre-release tag publish, **When** the manifest is pushed,
   **Then** `latest` is NOT updated (it retains the previous stable release).
3. **Given** a stable tag is published later (e.g. `v1.0.0` after `v1.0.0-rc.1`),
   **When** that workflow runs, **Then** `latest` IS updated to the stable
   release.

---

### User Story 3 — Credentials via GitHub Secrets (Priority: P3)

A maintainer wants to configure Docker Hub credentials once (as GitHub
repository secrets) and never deal with them again. The workflow authenticates
to Docker Hub non-interactively using these secrets. The secrets are never
logged, never appear in workflow logs, and the workflow fails fast with a clear
message if the required secrets are missing.

**Why this priority**: this is the security and usability foundation for P1/P2.
Without stored credentials, the publish workflow cannot run unattended on tag
push. It is P3 (not P1) only because it is implied by every publish — it does
not add a new user-facing capability, it makes the existing capability safe.

**Independent Test**: confirm the workflow references the correct GitHub
secrets and fails with a clear message when they are absent; confirm the secret
values do not appear in any log output.

**Acceptance Scenarios**:

1. **Given** the `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` secrets are set in
   the repository, **When** the publish workflow runs, **Then** it authenticates
   to Docker Hub and pushes without prompting.
2. **Given** one or both secrets are missing, **When** the workflow starts,
   **Then** it fails fast with an error message naming the missing secret(s).
3. **Given** the workflow runs to completion, **When** a maintainer reviews the
   workflow logs, **Then** no secret value (username or token) appears in the
   logs.

---

### Edge Cases

- **Non-semver tag pushed** (e.g. `v1.0`, `release-1`, `v1.0.0.0`): the
  workflow MUST NOT trigger. The tag filter is strict semver (`vMAJOR.MINOR.PATCH`
  optionally followed by a `-prerelease` suffix). An invalid tag is ignored
  silently — there is no failure because the workflow never starts.
- **Tag already published** (re-push of an existing tag): the workflow
  overwrites the existing manifest unconditionally. Docker Hub's registry is
  content-addressable; re-pushing the same tag replaces the manifest. There is
  no "already published" guard — the maintainer guarantees freshness by tagging.
- **Tag pushed to a fork, not upstream**: the workflow only runs on the
  canonical repository. It is gated on `github.repository == 'AectannArd/emergency-ration-webhook'`
  so a fork's tag push does not attempt to publish to the upstream Docker Hub
  repo.
- **Multi-arch build failure on one platform**: if the arm64 build fails, the
  entire publish MUST fail — a partial manifest (one architecture) must not be
  published as the canonical multi-arch image. `docker buildx` with
  `--provenance=false` produces a single manifest; a per-platform failure fails
  the buildx invocation.
- **Docker Hub rate limit or outage**: a push that fails due to a transient
  registry error is retried up to 3 times with backoff (the workflow uses the
  standard `docker/build-push-action` retry behavior). A persistent failure
  surfaces the error to the workflow run status.
- **Workflow triggered manually (`workflow_dispatch`)**: the workflow supports
  manual dispatch with an optional image-tag input, so a maintainer can publish
  or re-publish without creating a git tag. When dispatched without an explicit
  tag input, the workflow uses the short SHA of the dispatched commit.

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The repository MUST contain a GitHub Actions workflow file
  (`.github/workflows/publish.yml`) that publishes the webhook container image
  to Docker Hub.
- **FR-002**: The workflow MUST trigger on git tags matching the semantic
  version pattern `v[0-9]+.[0-9]+.[0-9]+` optionally followed by a `-prerelease`
  suffix (e.g. `v1.0.0`, `v1.0.0-rc.1`).
- **FR-003**: The workflow MUST build the image for both `linux/amd64` and
  `linux/arm64` and publish a single multi-arch OCI manifest (not separate
  per-architecture repositories).
- **FR-004**: The workflow MUST tag the published manifest with the exact git
  tag name (e.g. `v1.0.0`).
- **FR-005**: For stable releases (tags WITHOUT a `-prerelease` suffix), the
  workflow MUST also update the `latest` tag to point at the same manifest as
  the version tag.
- **FR-006**: For pre-release tags (tags WITH a `-prerelease` suffix), the
  workflow MUST NOT update `latest` — only the explicit version tag is pushed.
- **FR-007**: The workflow MUST authenticate to Docker Hub using the GitHub
  repository secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`.
- **FR-008**: The workflow MUST fail fast with a clear error message if
  `DOCKERHUB_USERNAME` or `DOCKERHUB_TOKEN` is not configured.
- **FR-009**: The workflow MUST be gated on the canonical repository
  (`AectannArd/emergency-ration-webhook`) so it does not run on forks.
- **FR-010**: The workflow MUST support manual dispatch
  (`workflow_dispatch`) with an optional image-tag input for ad-hoc publishing.
- **FR-011**: The Docker Hub repository name MUST be
  `aectann/emergency-ration-webhook`.
- **FR-012**: The workflow MUST use the repository's existing `Dockerfile`
  unchanged — it does not generate or modify the Dockerfile.
- **FR-013**: The workflow MUST NOT publish if the `quality` gate (fmt, clippy,
  test) is not green on the tagged commit. The publish job depends on the
  quality gate passing.
- **FR-014**: The workflow file MUST follow `.editorconfig` formatting rules
  (2-space indent for YAML, Unix line endings).
- **FR-015**: The README MUST document the published image location and how to
  pull it, so operators can discover the artifact.

### Key Entities

- **Publish Workflow**: the `.github/workflows/publish.yml` GitHub Actions
  workflow that triggers on semver git tags, builds a multi-arch image, and
  pushes it to Docker Hub. Configured via GitHub secrets.
- **Image Reference**: the fully-qualified Docker Hub reference:
  `aectann/emergency-ration-webhook:<tag>`, where `<tag>` is the git tag name
  (e.g. `v1.0.0`), `latest` (for stable releases), or the dispatch input /
  short SHA (for manual dispatch).
- **Credentials**: the `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` GitHub
  repository secrets used for non-interactive authentication.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Pushing a semver git tag (e.g. `v1.0.0`) to the repository
  results in a multi-arch image published to
  `aectann/emergency-ration-webhook:v1.0.0` within one workflow run (typically
  under 15 minutes).
- **SC-002**: The published multi-arch manifest is pullable and runnable on
  both amd64 and arm64 Linux hosts — `docker pull` selects the correct variant
  automatically.
- **SC-003**: After a stable release, `latest` on Docker Hub points at the same
  manifest as the most recent stable version tag; after a pre-release, `latest`
  is unchanged.
- **SC-004**: An operator can discover the published image and its pull command
  from the README alone, with no need to inspect the workflow file.

## Assumptions

- The Docker Hub repository `aectann/emergency-ration-webhook` exists (or will
  be created by the maintainer) and the GitHub secrets `DOCKERHUB_USERNAME`
  and `DOCKERHUB_TOKEN` are configured with push access to it.
- The existing `Dockerfile` builds correctly for both `linux/amd64` and
  `linux/arm64` — it uses a multi-stage build from `rust:1.89-bookworm` (Debian,
  available for both architectures) and a distroless runtime that is
  multi-arch.
- Docker Hub is the sole distribution registry for this feature. Other
  registries (GHCR, GitVerse container registry) are out of scope for this spec
  and may be added later.
- The `quality` CI job in `.github/workflows/ci.yml` remains the gate; the
  publish workflow re-runs or depends on it rather than introducing a parallel
  quality check.
- Git tags are the release mechanism — there is no GitHub Releases integration
  in this spec (no release notes auto-generation). A GitHub Release MAY be
  created manually after publishing; that is out of scope.
- The workflow runs on GitHub-hosted runners (`ubuntu-latest`), which provide
  QEMU and buildx for cross-architecture builds. No self-hosted runner is
  required.
