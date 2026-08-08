# Contract: Release Workflow (chart packaging + attachment)

## 1. Trigger

The chart packaging job triggers on the same events as the existing image
publish: push of a semver git tag (`v[0-9]*.[0-9]*.[0-9]*` or
`v[0-9]*.[0-9]*.[0-9]*-*`) and `workflow_dispatch`.

## 2. Job structure

```yaml
charts:
  name: Package + attach Helm charts
  needs: quality              # same quality gate as publish
  runs-on: ubuntu-latest
  steps:
    # 1. Checkout (shallow)
    # 2. Set up Helm
    # 3. Stamp chart versions from git tag
    # 4. helm lint both charts (gate)
    # 5. helm package both charts
    # 6. Attach .tgz to the GitHub Release
```

## 3. Version stamping (step 3)

```sh
VERSION="${GITHUB_REF_NAME#v}"   # vX.Y.Z → X.Y.Z
# For workflow_dispatch (no tag), fall back to short SHA:
if [ -z "$VERSION" ] || [ "$VERSION" = "main" ]; then
  VERSION="0.0.0-dev-$(git rev-parse --short HEAD)"
fi

for chart in deploy/charts/webhook deploy/charts/equalizer; do
  sed -i "s/^version:.*/version: \"${VERSION}\"/" "$chart/Chart.yaml"
  sed -i "s/^appVersion:.*/appVersion: \"${VERSION}\"/" "$chart/Chart.yaml"
done
```

## 4. Lint + package (steps 4–5)

```sh
helm lint deploy/charts/webhook
helm lint deploy/charts/equalizer
helm package deploy/charts/webhook --destination .temp/charts/
helm package deploy/charts/equalizer --destination .temp/charts/
```

Output: `emergency-ration-webhook-<version>.tgz` and
`emergency-ration-equalizer-<version>.tgz` in `.temp/charts/`.

A failing `helm lint` exits the job with non-zero, blocking the release.

## 5. Attach to GitHub Release (step 6)

```yaml
- name: Attach charts to release
  uses: softprops/action-gh-release@v2
  with:
    files: |
      .temp/charts/emergency-ration-webhook-*.tgz
      .temp/charts/emergency-ration-equalizer-*.tgz
```

GitHub auto-creates the Release on tag push; this step attaches the `.tgz` files
to it. For `workflow_dispatch`, the step creates a draft release or attaches to
an existing one (softprops/action-gh-release handles both).

## 6. Relationship to the existing publish job

The `charts` job and the `publish` (Docker images) job are independent and can
run in parallel after `quality`. Neither depends on the other. Both gate on the
same quality job. If either fails, the release is incomplete (Principle XI).

## 7. Secrets

The `charts` job needs no additional secrets — GitHub Release attachment uses the
default `GITHUB_TOKEN` automatically provided to the workflow. No Docker Hub
credentials needed for chart packaging.

## 8. ARTIFACTS.md update

`ARTIFACTS.md` gains a "Manifest bundles" section documenting:

| Component | Kustomize path | Helm chart | Release artifact |
|-----------|---------------|------------|-----------------|
| webhook | `deploy/kustomize/webhook/` | `emergency-ration-webhook` `.tgz` | GitHub Release attachment |
| equalizer | `deploy/kustomize/equalizer/` | `emergency-ration-equalizer` `.tgz` | GitHub Release attachment |
