# Data Model: Docker Hub Image Publishing

**Feature**: 011-docker-hub-publishing | **Date**: 2026-08-02

This feature has no runtime data model — it is a CI workflow, not a library or
service. The "entities" are the workflow's inputs (triggers, secrets) and
outputs (image tags). This document captures the derived tag rules and the
secret contract.

## Tag Derivation Rules

The image tag set is derived from the triggering event by `docker/metadata-action`.

| Trigger | `github.ref_name` | Tags emitted | `latest`? |
|---------|-------------------|--------------|-----------|
| Tag `v1.0.0` (stable) | `v1.0.0` | `v1.0.0`, `latest` | ✅ yes |
| Tag `v1.0.0-rc.1` (pre-release) | `v1.0.0-rc.1` | `v1.0.0-rc.1` | ❌ no |
| Tag `v2.3.4` (stable) | `v2.3.4` | `v2.3.4`, `latest` | ✅ yes |
| Tag `v2.3.4-beta.2` (pre-release) | `v2.3.4-beta.2` | `v2.3.4-beta.2` | ❌ no |
| `workflow_dispatch` (manual, tag input=`nightly`) | n/a | `nightly` | ❌ no |
| `workflow_dispatch` (manual, no input) | n/a | `<short-sha>` | ❌ no |

### Rule: stable vs pre-release

A tag is **stable** if and only if it matches `v[0-9]*.[0-9]*.[0-9]*` with NO
hyphen. A tag is **pre-release** if it contains a `-` after the patch version
(e.g. `-rc.1`, `-beta.2`). The `latest` tag is pushed only for stable tags.

Implementation: `enable=${{ !contains(github.ref_name, '-') }}`.

## Secret Contract

| Secret | Required | Scope | Description |
|--------|----------|-------|-------------|
| `DOCKERHUB_USERNAME` | Yes | repo | Docker Hub account username with push access to `aectann/emergency-ration-webhook`. |
| `DOCKERHUB_TOKEN` | Yes | repo | Docker Hub access token (not the account password). Scoped to the repository or read/write. |

**Lifecycle**: secrets are configured once by a repository admin and never
appear in tracked files, workflow logs (GitHub auto-masks), or commit history.
The workflow references them by name; it never reads their values into
variables that could be logged.

## Workflow Inputs (`workflow_dispatch` only)

| Input | Required | Default | Description |
|-------|----------|---------|-------------|
| `image_tag` | No | `<short-sha>` | The Docker Hub tag to publish under. If omitted, the commit's short SHA is used. |

## No Persistent State

This feature creates no CRDs, no files, no database entries, and no
long-running processes. The workflow is stateless: it runs, publishes, and
terminates. The only durable artifact is the image manifest on Docker Hub.
