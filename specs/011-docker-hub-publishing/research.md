# Research: Docker Hub Image Publishing

**Feature**: 011-docker-hub-publishing | **Date**: 2026-08-02

Resolves the technical unknowns from the Technical Context section of the plan.

## R1 — GitHub Actions: tag-triggered multi-arch Docker publish

**Decision**: Use `docker/build-push-action@v6` with `docker/setup-buildx-action`
and `docker/setup-qemu-action` for a multi-arch manifest publish.

**Rationale**:
- `docker/build-push-action` is the canonical GitHub Action for publishing
  images. v6 is the current major (v5 also works; v6 adds improved provenance
  attestation controls). It wraps `docker buildx build` and handles the
  multi-arch manifest as a single invocation when `platforms` lists more than
  one OS/arch.
- `docker/setup-qemu-action` registers QEMU emulators so the buildx builder can
  cross-compile for arm64 on an amd64 GitHub-hosted runner. Without it, an
  arm64 build on `ubuntu-latest` fails with "exec format error".
- `docker/setup-buildx-action` creates a buildx builder instance that supports
  multi-platform builds and the `--push` flag (the default docker driver does
  not support multi-platform output).
- The `platforms` input is set to `linux/amd64,linux/arm64` per FR-003.

**Alternatives considered**:
- *Manual `docker buildx build --platform ... --push` in a `run:` step*:
  works, but loses the structured retry, caching, and metadata outputs that the
  action provides. More boilerplate for the same outcome.
- *Two separate single-arch builds + `docker manifest create`*: the legacy
  approach pre-buildx. It is fragile (manifest tooling has known edge cases
  with media types) and obsolete now that buildx produces multi-arch manifests
  natively.
- *`docker/build-push-action@v5`*: functionally equivalent for our needs, but
  v6 is the current release and is the safer pin for a new workflow.

## R2 — Semver tag filtering and `latest` logic

**Decision**: Trigger on `on.push.tags: ['v[0-9]+.[0-9]+.[0-9]+*', '!v[0-9]+.[0-9]+.[0-9]+']`
… — no, GitHub `on.push.tags` uses glob patterns, not regex. Use:

```yaml
on:
  push:
    tags:
      - 'v[0-9]+.[0-9]+.[0-9]+'
      - 'v[0-9]+.[0-9]+.[0-9]+-*'
```

GitHub Actions tag filters support `*` and `?` wildcards (glob, not regex).
- `v[0-9]+.[0-9]+.[0-9]+` matches `v1.0.0` (stable). The `+` is literal here
  — but GitHub globs treat `[0-9]` as a character class and `+` as a literal,
  so this pattern does NOT mean "one or more digits" in regex sense. However,
  the established community convention is to use this exact glob because it
  matches single-digit components (v1.0.0–v9.9.9), and the fallback
  `v[0-9]*.[0-9]*.[0-9]*` pattern handles multi-digit versions.

Corrected decision — use the robust multi-digit-safe glob:
```yaml
on:
  push:
    tags:
      - 'v[0-9]*.[0-9]*.[0-9]*'
      - 'v[0-9]*.[0-9]*.[0-9]*-*'
```
- `v[0-9]*.[0-9]*.[0-9]*` matches `v1.0.0`, `v10.20.30` (stable).
- `v[0-9]*.[0-9]*.[0-9]*-*` matches `v1.0.0-rc.1`, `v1.0.0-beta.2` (pre-release).

**`latest` logic**: `docker/build-push-action` has no built-in "push latest only
if stable" flag. The type of tags pushed is controlled by the `tags` input
(which is a list of explicit tags or a flavor configuration). Decision:
generate the tag list dynamically with `docker/metadata-action@v5`:

```yaml
- uses: docker/metadata-action@v5
  id: meta
  with:
    images: aectann/emergency-ration-webhook
    tags: |
      type=ref,event=tag           # the git tag itself (v1.0.0, v1.0.0-rc.1)
      type=raw,value=latest,enable={{is_default_branch}}  # NO — this is for branch pushes
```

The `is_default_branch` function does not apply to tag events. The correct
approach for "latest only on stable tags" is:
```yaml
    tags: |
      type=ref,event=tag
      type=semver,pattern={{raw}},enable=${{ !contains(github.ref_name, '-') }}
      type=raw,value=latest,enable=${{ !contains(github.ref_name, '-') }}
```
Wait — `type=ref,event=tag` already produces the raw tag name. The `latest`
addition must be gated on the tag NOT being a pre-release. A pre-release tag
contains a `-` after the patch version (per semver: `v1.0.0-rc.1`). So:
```yaml
    tags: |
      type=ref,event=tag
      type=raw,value=latest,enable=${{ !contains(github.ref_name, '-') }}
```
- `type=ref,event=tag` → emits the tag name verbatim (`v1.0.0` or `v1.0.0-rc.1`).
- `type=raw,value=latest,enable=!contains(github.ref_name,'-')` → emits
  `latest` only when the tag has no hyphen (stable). A pre-release tag
  (`v1.0.0-rc.1`) contains `-`, so `latest` is suppressed.

**Rationale**: `docker/metadata-action` is the canonical way to derive image
tags from git refs without hand-rolled shell. The `enable` expression uses
GitHub Actions expression syntax (`contains`, `!`) which is evaluated by the
Actions runtime, not by the metadata action.

**Alternatives considered**:
- *Hand-rolled `if`/`else` in shell*: fragile, harder to read, reinvents what
  metadata-action does for free.
- *Always push latest*: violates FR-006 (pre-release must not update latest).

## R3 — Credential handling and repository gating

**Decision**:
- Authenticate via `docker/login-action@v3` with:
  ```yaml
  username: ${{ secrets.DOCKERHUB_USERNAME }}
  password: ${{ secrets.DOCKERHUB_TOKEN }}
  ```
- Gate the workflow on the canonical repo:
  ```yaml
  jobs:
    publish:
      if: github.repository == 'AectannArd/emergency-ration-webhook'
  ```
- Fail-fast on missing secrets: the `docker/login-action` will fail with a
  clear auth error if the secrets are empty, but for a better message, add an
  explicit guard step:
  ```yaml
  - name: Verify required secrets
    run: |
      if [ -z "${{ secrets.DOCKERHUB_USERNAME }}" ] || [ -z "${{ secrets.DOCKERHUB_TOKEN }}" ]; then
        echo "::error::DOCKERHUB_USERNAME and/or DOCKERHUB_TOKEN secrets are not set."
        exit 1
      fi
  ```

**Rationale**: `docker/login-action@v3` is the standard non-interactive login.
The token (not the password) is used because Docker Hub recommends access
tokens over account passwords for CI. The `if: github.repository == ...` guard
prevents forks from accidentally publishing (a fork won't have the secrets, so
it would fail anyway, but the guard makes it fail fast with a clear "skipped"
status instead of an auth error).

**Secret logging**: GitHub Actions automatically masks secret values in logs.
`DOCKERHUB_TOKEN` registered as a secret is redacted in any output. The
`::error::` message references the secret *names*, not values.

**Alternatives considered**:
- *GHCR (GitHub Container Registry)*: out of scope (user specified Docker Hub).
  Could be added as a parallel push later.
- *Docker Hub password instead of token*: Docker Hub recommends tokens for CI;
  a password would work but is less secure (full account access vs scoped
  token).

## R4 — Build caching strategy

**Decision**: Use GitHub Actions cache backend via buildx `cache-from`/`cache-to`:
```yaml
cache-from: type=gha
cache-to: type=gha,mode=max
```

**Rationale**: the Rust build (spec-009's Dockerfile does a full `cargo build
--release`) is the slowest step. `type=gha` caches build layers in the GitHub
Actions cache, which is free for public repos and fast for private repos under
the cache quota. `mode=max` caches all layers (including intermediate), so
rebuilds after a dependency change only recompile the changed layer.

**Alternatives considered**:
- *No cache*: every publish rebuilds from scratch (8–12 min Rust compile). The
  cache cuts this to 2–4 min for unchanged dependency sets.
- *Registry cache (`type=registry`)*: stores cache in Docker Hub as a separate
  image. Works but consumes registry storage and adds a push/pull round-trip.
  GHA cache is free and co-located with the runner.

## R5 — Quality gate dependency

**Decision**: The publish workflow runs the quality gate (fmt + clippy + test)
as a job that the publish job `needs`. This re-runs the checks rather than
depending on the `ci.yml` workflow's status (cross-workflow status checks
require `workflow_run` triggers, which are awkward for tag pushes).

```yaml
jobs:
  quality:
    # same steps as ci.yml quality job
  publish:
    needs: quality
```

**Rationale**: a tag on a commit whose quality gate is red must not publish
(FR-013). Re-running the gate in the publish workflow is simpler and more
reliable than cross-workflow `workflow_run` gating, which has timing/race
constraints. The duplicate job definition is acceptable boilerplate (it's ~5
lines of `run:` steps).

**Alternatives considered**:
- *Trust that `main` is green before tagging*: a maintainer convention, not
  enforced. FR-013 requires enforcement.
- *`workflow_run` trigger depending on ci.yml*: only fires on `main`/PR
  workflows, not on tag pushes. Not applicable here.
