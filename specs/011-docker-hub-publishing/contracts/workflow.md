# Contract: Publish Workflow (`.github/workflows/publish.yml`)

**Feature**: 011-docker-hub-publishing | **Date**: 2026-08-02

The contract for the GitHub Actions workflow that publishes the webhook image
to Docker Hub. This is the interface the implementation (Claude Code) must
satisfy.

## Triggers

| Event | Filter | Purpose |
|-------|--------|---------|
| `push.tags` | `v[0-9]*.[0-9]*.[0-9]*` | Stable semver tag → publish with `latest`. |
| `push.tags` | `v[0-9]*.[0-9]*.[0-9]*-*` | Pre-release semver tag → publish WITHOUT `latest`. |
| `workflow_dispatch` | input `image_tag` (optional, default = short SHA) | Manual dispatch for ad-hoc publish. |

**Non-semver tags** (e.g. `v1.0`, `release-1`) do not match either glob and do
not trigger the workflow.

## Jobs

### `quality`

Re-runs the fmt + clippy + test gate (identical steps to `ci.yml`'s `quality`
job). The `publish` job depends on this passing.

### `publish`

- **Condition**: `github.repository == 'AectannArd/emergency-ration-webhook'`
  (does not run on forks).
- **Needs**: `quality`.
- **Runner**: `ubuntu-latest`.
- **Steps**:
  1. Checkout (shallow, `fetch-depth: 1` — no history needed for a build).
  2. Verify `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` secrets are non-empty
     (fail fast with `::error::` if missing).
  3. `docker/setup-qemu-action@v3` — register QEMU for arm64 cross-build.
  4. `docker/setup-buildx-action@v3` — create a multi-platform buildx builder.
  5. `docker/login-action@v3` — authenticate to Docker Hub.
  6. `docker/metadata-action@v5` — derive image tags (see data-model.md).
  7. `docker/build-push-action@v6` — build `linux/amd64,linux/arm64`, tag per
     metadata, push, cache via `type=gha`.

## Tag Derivation (metadata-action configuration)

```yaml
images: aectann/emergency-ration-webhook
tags: |
  type=ref,event=tag
  type=raw,value=latest,enable=${{ github.event_name == 'push' && !contains(github.ref_name, '-') }}
  type=raw,value=${{ inputs.image_tag }},enable=${{ github.event_name == 'workflow_dispatch' && inputs.image_tag != '' }}
  type=sha,format=short,enable=${{ github.event_name == 'workflow_dispatch' && inputs.image_tag == '' }}
```

- On tag push: emits the tag name + `latest` (if stable). The `latest` rule is
  gated on `github.event_name == 'push'` so a manual dispatch never updates
  `latest` (it is reserved for stable git-tag releases only).
- On manual dispatch with `image_tag`: emits the input value.
- On manual dispatch without `image_tag`: emits `sha-<short>` as the fallback
  tag (via `type=sha,format=short`).

## Caching

```yaml
cache-from: type=gha
cache-to: type=gha,mode=max
```

## Error Conditions

- Missing secret → step 2 fails with `::error::` naming the missing secret(s).
- Login failure → `docker/login-action` fails (invalid token).
- Build failure (either arch) → `docker/build-push-action` fails; the
  multi-arch manifest is not pushed (buildx is atomic across platforms).
- Push failure (transient) → `docker/build-push-action` retries internally;
  persistent failure surfaces to the workflow run status.
