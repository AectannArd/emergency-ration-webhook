# Quickstart: Docker Hub Image Publishing

**Feature**: 011-docker-hub-publishing | **Date**: 2026-08-02

Runnable validation that the publish workflow works end-to-end. See
[data-model.md](./data-model.md) for tag rules and
[contracts/](./contracts/) for the workflow + secret interface.

## Prerequisites

1. The Docker Hub repository `aectann/emergency-ration-webhook` exists.
2. GitHub secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` are configured
   (see [contracts/secrets.md](./contracts/secrets.md)).
3. The workflow file `.github/workflows/publish.yml` is merged to `main`.

## Scenario A — Stable release via tag push

```bash
# From the repository root, on main with a green CI:
git tag v1.0.0
git push origin v1.0.0
```

**Expected outcome** (within ~15 min):
- The "Publish to Docker Hub" workflow runs.
- `docker pull aectann/emergency-ration-webhook:v1.0.0` succeeds on amd64 and
  arm64 hosts.
- `docker pull aectann/emergency-ration-webhook:latest` resolves to the same
  manifest as `:v1.0.0`.

**Verify multi-arch**:
```bash
docker manifest inspect aectann/emergency-ration-webhook:v1.0.0
# Should list both linux/amd64 and linux/arm64 entries.
```

## Scenario B — Pre-release via tag push

```bash
git tag v1.0.0-rc.1
git push origin v1.0.0-rc.1
```

**Expected outcome**:
- `docker pull aectann/emergency-ration-webhook:v1.0.0-rc.1` succeeds (both
  architectures).
- `latest` is **unchanged** (still points at the previous stable, or does not
  exist if this is the first publish).

## Scenario C — Manual dispatch

GitHub UI → Actions → "Publish to Docker Hub" → Run workflow.

- With `image_tag` input = `nightly` → publishes `:nightly`.
- Without input → publishes `:<short-sha>`.

## Failure modes to check

- **Missing secrets**: temporarily remove a secret, dispatch → workflow fails
  fast with `::error::` naming the missing secret.
- **Fork**: push the same tag on a fork → workflow is skipped (the
  `github.repository` guard).
- **Red quality gate**: introduce a failing test, tag → the `quality` job
  fails and `publish` does not run.
