# Quickstart — Validation Scenarios for Kustomize + Helm Manifest Bundles (spec-015)

This guide maps each spec user story to runnable validation scenarios. Run these
after implementation to prove the feature works end-to-end.

## Prerequisites

- A `kind` cluster (or any reachable Kubernetes 1.34+ cluster)
- `kubectl` with `kustomize` support (`kubectl kustomize`)
- `helm` 3.x
- The webhook + equalizer Docker images loaded into the cluster (for kind:
  `kind load docker-image capacity-admission-webhook:e2e` /
  `capacity-equalizer:e2e`)
- Rust toolchain (for `erw-verify` build)

## US1 — Helm chart installation

### V1: helm install webhook

```sh
# Render (dry-run) to verify templates produce valid YAML.
helm template deploy/charts/webhook > /tmp/webhook-rendered.yaml

# Lint (must pass with zero errors).
helm lint deploy/charts/webhook

# Install into a kind cluster with a locally-loaded image.
helm install erw deploy/charts/webhook \
  --set image.repository=capacity-admission-webhook \
  --set image.tag=e2e

# Verify the webhook reaches Ready and enforces the budget.
kubectl -n capacity-admission wait --for=condition=Ready pod -l app=capacity-admission-webhook --timeout=60s
kubectl -n default run smoke-over --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never
# Expected: pod rejected (budget enforcement).
```

**Expected outcome**: webhook Deployment reaches Ready; over-budget pod is
rejected.

### V2: helm install equalizer

```sh
helm lint deploy/charts/equalizer
helm template deploy/charts/equalizer > /tmp/eq-rendered.yaml

helm install eq deploy/charts/equalizer \
  --set image.repository=capacity-equalizer \
  --set image.tag=e2e

kubectl -n capacity-equalizer wait --for=condition=Ready pod -l app=capacity-equalizer --timeout=60s
```

**Expected outcome**: equalizer Deployment reaches Ready.

### V3: Kustomize ↔ Helm parity

```sh
# Render both formats.
kustomize build deploy/kustomize/webhook > /tmp/k-out.yaml
helm template deploy/charts/webhook > /tmp/h-out.yaml

# Compare critical fields (run the parity script from CI, or manual inspection):
# For each resource kind:name pair, verify apiVersion, kind, name, namespace,
# failurePolicy, sideEffects, RBAC verbs, container ports, and image field match.
```

**Expected outcome**: all critical fields match between Kustomize and Helm
rendered output. Minor formatting/label differences acceptable.

## US2 — Kustomize deployment

### V4: kustomize build + apply (webhook)

```sh
# Build and apply with image override for the kind-loaded image.
kubectl kustomize deploy/kustomize/webhook \
  | sed 's|capacity-admission-webhook:latest|capacity-admission-webhook:e2e|' \
  | kubectl apply -f -

# Verify.
kubectl -n capacity-admission wait --for=condition=Ready pod -l app=capacity-admission-webhook --timeout=60s
kubectl get validatingwebhookconfiguration capacity-admission.emergency-ration.dev
```

**Expected outcome**: all 11 webhook resources created; Deployment reaches Ready;
webhook is registered.

### V5: kustomize build + apply (equalizer)

```sh
kubectl kustomize deploy/kustomize/equalizer \
  | sed 's|capacity-equalizer:latest|capacity-equalizer:e2e|' \
  | kubectl apply -f -

kubectl -n capacity-equalizer wait --for=condition=Ready pod -l app=capacity-equalizer --timeout=60s
```

**Expected outcome**: all 6 equalizer resources created; Deployment reaches Ready.

### V6: Kustomize ↔ pre-migration parity

Before deleting the raw manifests, verify the Kustomize output is field-by-field
equivalent to the raw YAML (except image):

```sh
# Snapshot the raw manifests (pre-migration).
cat deploy/crds.yaml deploy/rbac.yaml deploy/deployment.yaml deploy/webhook-config.yaml deploy/cert-setup.yaml > /tmp/raw-webhook.yaml

# Render Kustomize.
kustomize build deploy/kustomize/webhook > /tmp/kuz-webhook.yaml

# Compare resource-by-resource (script or manual).
# Permitted differences: image field (placeholder → resolved), path in comments.
```

**Expected outcome**: zero differences in any contract-critical field
(failurePolicy, sideEffects, RBAC verbs, namespace, names, ports, probes,
securityContext).

## US3 — Release artifacts

### V7: chart packaging

```sh
# Simulate the release workflow locally.
sed -i 's/^version:.*/version: "1.0.0"/' deploy/charts/webhook/Chart.yaml
sed -i 's/^version:.*/version: "1.0.0"/' deploy/charts/equalizer/Chart.yaml

helm package deploy/charts/webhook --destination /tmp/charts/
helm package deploy/charts/equalizer --destination /tmp/charts/

ls /tmp/charts/
# Expected: emergency-ration-webhook-1.0.0.tgz
#           emergency-ration-equalizer-1.0.0.tgz

# Verify the packaged chart lints and installs.
helm lint /tmp/charts/emergency-ration-webhook-1.0.0.tgz
helm install test-w /tmp/charts/emergency-ration-webhook-1.0.0.tgz --set image.repository=capacity-admission-webhook --set image.tag=e2e
```

**Expected outcome**: two versioned `.tgz` packages, both lint-clean,
installable from the archive.

### V8: release workflow (CI)

Push a pre-release tag and observe the GitHub Release:

```sh
git tag v1.0.0-rc.1
git push origin v1.0.0-rc.1
# Wait for the publish workflow to complete.
gh release view v1.0.0-rc.1 --json assets --jq '.assets[].name'
```

**Expected outcome**: release assets include
`emergency-ration-webhook-1.0.0-rc.1.tgz` and
`emergency-ration-equalizer-1.0.0-rc.1.tgz`.

## US4 — erw-verify + CI migration

### V9: erw-verify with Kustomize-rendered manifests

```sh
# Build erw-verify (requires kustomize on PATH).
cargo build --bin erw-verify

# Run against a kind cluster with a locally-loaded image.
cargo run --bin erw-verify -- --skip-build --kubeconfig ~/.kube/config
```

**Expected outcome**: all S1–S11 scenarios pass; zero behavioral regression
versus pre-migration.

### V10: CI E2E jobs pass

Both CI workflows pass on a PR after migration:

```sh
# Triggered automatically on PR push.
# ci.yml: webhook E2E via kustomize build
# equalizer-e2e.yml: both stacks via kustomize build
```

**Expected outcome**: all CI jobs green (Principle XI).

### V11: raw manifests deleted

```sh
# After all consumers migrated, verify the raw files are gone.
ls deploy/*.yaml 2>/dev/null       # Expected: no such file
ls deploy/equalizer/ 2>/dev/null   # Expected: no such file/directory

# Verify no source code references the old paths.
grep -rn 'deploy/deployment.yaml\|deploy/rbac.yaml\|deploy/crds.yaml\|deploy/webhook-config.yaml\|ERW_IMAGE_PLACEHOLDER\|ERW_EQUALIZER_IMAGE_PLACEHOLDER' src/ .github/ docs/ README.md CONTRIBUTING.md
# Expected: zero matches.
```

**Expected outcome**: zero references to deleted paths; zero placeholder tokens.

## US5 — Documentation completeness

### V12: documentation sweep

```sh
# No references to deleted root manifest paths.
grep -rn 'deploy/deployment.yaml\|deploy/rbac.yaml\|deploy/crds.yaml\|deploy/webhook-config.yaml\|deploy/cert-setup.yaml\|deploy/equalizer/' \
  README.md docs/ CONTRIBUTING.md .github/workflows/

# Expected: zero matches (all migrated to deploy/kustomize/ or deploy/charts/).

# New docs/manifest-bundles.md exists and is linked from README.
test -f docs/manifest-bundles.md && echo "EXISTS"
grep 'manifest-bundles' README.md
```

**Expected outcome**: zero stale path references; new article exists and is
linked; ARTIFACTS.md lists manifest bundles.
