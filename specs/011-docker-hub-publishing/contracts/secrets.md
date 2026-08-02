# Contract: Docker Hub Secrets

**Feature**: 011-docker-hub-publishing | **Date**: 2026-08-02

## Required Repository Secrets

Configure these in the GitHub repository under **Settings → Secrets and
variables → Actions → New repository secret**.

| Secret Name | Value | Notes |
|-------------|-------|-------|
| `DOCKERHUB_USERNAME` | Docker Hub account username (e.g. `aectann`) | Must have push access to the `aectann/emergency-ration-webhook` repository. |
| `DOCKERHUB_TOKEN` | Docker Hub access token | Generate at Docker Hub → Account Settings → Security → New Access Token. **Do not use the account password.** |

## Verification

After configuring, trigger the workflow manually (Actions → "Publish to Docker
Hub" → Run workflow) to confirm the credentials work before relying on tag
pushes.

## Security Properties

- Secrets are referenced by name in the workflow; their values never appear in
  tracked files.
- GitHub Actions automatically masks secret values in all log output.
- The token is scoped (Docker Hub allows per-token scope). Use the narrowest
  scope that includes push to the target repository.
- If a token is compromised, revoke it at Docker Hub → Security → Revoke. No
  code change or redeploy is needed.
