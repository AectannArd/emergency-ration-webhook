# Data Model — Kustomize + Helm Manifest Bundles (spec-015)

This feature has no runtime data model (no CRDs, no state machines). Instead,
the "data" is the **manifest inventory** — the exact set of Kubernetes resources
each bundle must render, their source location, and the consumer impact matrix.

## 1. Manifest inventory — resource mapping

Every resource in the pre-migration raw manifests MUST appear in both the
Kustomize bundle and the Helm chart with field-level equivalence. The only
permitted difference is the image field (templated in both new formats).

### 1.1 Webhook component

| # | Kind | Name | Source (pre-migration) | Kustomize target | Helm template |
|---|------|------|------------------------|------------------|---------------|
| W1 | Namespace | `capacity-admission` | `deploy/deployment.yaml` | `deploy/kustomize/webhook/deployment.yaml` | `templates/namespace.yaml` |
| W2 | Deployment | `capacity-admission-webhook` | `deploy/deployment.yaml` | `deploy/kustomize/webhook/deployment.yaml` | `templates/deployment.yaml` |
| W3 | Service | `capacity-admission-webhook` | `deploy/deployment.yaml` | `deploy/kustomize/webhook/deployment.yaml` | `templates/service.yaml` |
| W4 | CRD (ClusterCapacity) | `clustercapacities.emergency-ration.dev` | `deploy/crds.yaml` | `deploy/kustomize/webhook/crds.yaml` | `templates/crds.yaml` |
| W5 | CRD (Allocation) | `allocations.emergency-ration.dev` | `deploy/crds.yaml` | `deploy/kustomize/webhook/crds.yaml` | `templates/crds.yaml` |
| W6 | ServiceAccount | `capacity-admission-webhook` | `deploy/rbac.yaml` | `deploy/kustomize/webhook/rbac.yaml` | `templates/rbac.yaml` |
| W7 | ClusterRole | `capacity-admission-webhook` | `deploy/rbac.yaml` | `deploy/kustomize/webhook/rbac.yaml` | `templates/rbac.yaml` |
| W8 | ClusterRoleBinding | `capacity-admission-webhook` | `deploy/rbac.yaml` | `deploy/kustomize/webhook/rbac.yaml` | `templates/rbac.yaml` |
| W9 | ValidatingWebhookConfiguration | `capacity-admission.emergency-ration.dev` | `deploy/webhook-config.yaml` | `deploy/kustomize/webhook/webhook-config.yaml` | `templates/webhook-config.yaml` |
| W10 | Issuer (cert-manager) | `capacity-admission-self-signed` | `deploy/cert-setup.yaml` | `deploy/kustomize/webhook/cert-setup.yaml` | `templates/cert-setup.yaml` |
| W11 | Certificate (cert-manager) | `capacity-admission-webhook` | `deploy/cert-setup.yaml` | `deploy/kustomize/webhook/cert-setup.yaml` | `templates/cert-setup.yaml` |

**Total: 11 resources.**

### 1.2 Equalizer component

| # | Kind | Name | Source (pre-migration) | Kustomize target | Helm template |
|---|------|------|------------------------|------------------|---------------|
| E1 | Namespace | `capacity-equalizer` | `deploy/equalizer/deployment.yaml` | `deploy/kustomize/equalizer/deployment.yaml` | `templates/namespace.yaml` |
| E2 | Deployment | `capacity-equalizer` | `deploy/equalizer/deployment.yaml` | `deploy/kustomize/equalizer/deployment.yaml` | `templates/deployment.yaml` |
| E3 | CRD (EqualizerConfig) | `equalizerconfigs.emergency-ration.dev` | `deploy/equalizer/crds.yaml` | `deploy/kustomize/equalizer/crds.yaml` | `templates/crds.yaml` |
| E4 | ServiceAccount | `capacity-equalizer` | `deploy/equalizer/rbac.yaml` | `deploy/kustomize/equalizer/rbac.yaml` | `templates/rbac.yaml` |
| E5 | ClusterRole | `capacity-equalizer` | `deploy/equalizer/rbac.yaml` | `deploy/kustomize/equalizer/rbac.yaml` | `templates/rbac.yaml` |
| E6 | ClusterRoleBinding | `capacity-equalizer` | `deploy/equalizer/rbac.yaml` | `deploy/kustomize/equalizer/rbac.yaml` | `templates/rbac.yaml` |

**Total: 6 resources.** The `equalizer-config.example.yaml` (Secrets +
EqualizerConfig example) is migrated as documentation (not a default-applied
resource) — see research R10.

## 2. Critical fields preserved verbatim

These fields are constitutionally protected (Principles I, III, V) or
operationally load-bearing. The Kustomize and Helm bundles MUST render them
identically to the pre-migration raw manifests.

### 2.1 ValidatingWebhookConfiguration (W9)

| Field | Value | Why critical |
|-------|-------|-------------|
| `failurePolicy` | `Fail` | Principle I (fail-closed) |
| `sideEffects` | `None` | v1 admission contract |
| `timeoutSeconds` | `5` | apiserver-level timeout |
| `matchPolicy` | `Exact` | admission semantics |
| `admissionReviewVersions` | `["v1"]` | API version |
| `clientConfig.service.path` | `/validate` | webhook route |
| `clientConfig.service.port` | `8443` | webhook HTTPS port |
| `namespaceSelector` (exclude own ns) | NotIn capacity-admission | bootstrap exclusion |

### 2.2 RBAC verbs (W7, E5)

The ClusterRole verb lists MUST be identical — these are the least-privilege
boundary (Principle V).

**Webhook ClusterRole** (W7): nodes [get,list,watch], pods [get,list,watch],
clustercapacities [get,list,watch,create], clustercapacities/status
[get,update,patch], allocations [get,list,watch,create], allocations/status
[get,update,patch].

**Equalizer ClusterRole** (E5): equalizerconfigs
[get,list,watch,create,update,patch,delete], equalizerconfigs/status
[get,update,patch], secrets [get].

### 2.3 Container specification (W2, E2)

| Field | Webhook value | Equalizer value |
|-------|---------------|-----------------|
| `securityContext.runAsNonRoot` | `true` | `true` |
| `securityContext.runAsUser` | `65532` | `65532` |
| `securityContext.readOnlyRootFilesystem` | `true` | `true` |
| `securityContext.capabilities.drop` | `[ALL]` | `[ALL]` |
| `image` | **templated** (was `ERW_IMAGE_PLACEHOLDER`) | **templated** (was `ERW_EQUALIZER_IMAGE_PLACEHOLDER`) |
| `imagePullPolicy` | `Always` | `Always` |

## 3. Consumer impact matrix

Each consumer of the raw manifests must be migrated before the raw files can be
deleted.

| Consumer | Current mechanism | Migration target | Risk |
|----------|-------------------|------------------|------|
| `erw-verify` (`setup.rs`) | `include_str!("../../../deploy/*.yaml")` — 4 files embedded at compile time | `build.rs` runs `kustomize build` → `OUT_DIR` → `include_str!` (research R5) | **High** — build-time `kustomize` dependency; image-substitution logic retargeted |
| `ci.yml` E2E job | `sed 's\|ERW_IMAGE_PLACEHOLDER\|...' deploy/deployment.yaml \| kubectl apply` | `kubectl kustomize deploy/kustomize/webhook \| sed (image) \| kubectl apply` (research R8) | Medium — pattern change, but functionally equivalent |
| `equalizer-e2e.yml` | `kubectl apply -f deploy/equalizer/{crds,rbac}.yaml` + `sed ... deploy/equalizer/deployment.yaml` | Same kustomize pattern for both webhook + equalizer stacks (research R8) | Medium — two stacks to migrate |
| README Quick Start | `kubectl apply -f deploy/deployment.yaml` (6 steps) | `helm install` or `kustomize build ... \| kubectl apply` | Low — documentation rewrite |
| `docs/deployment.md` | References `deploy/deployment.yaml` etc. | Rewrite to bundle paths | Low — documentation rewrite |
| `CONTRIBUTING.md` | References `deploy/deployment.yaml` + publish docs | Rewrite to Kustomize + chart packaging | Low — documentation rewrite |
| Integration tests (`tests/`) | Do NOT consume deploy manifests (mocked apiserver fixtures) | **No change** (FR-020) | None — verification-only |

## 4. Image reference evolution

| Phase | Webhook image in manifests | Mechanism |
|-------|---------------------------|-----------|
| Pre-migration | `ERW_IMAGE_PLACEHOLDER` (literal token) | `sed` replacement by CI / erw-verify |
| Post-migration (Kustomize) | `capacity-admission-webhook:placeholder` | `kustomize edit set image` or rendered-output sed |
| Post-migration (Helm) | `{{ .Values.image.repository }}:{{ .Values.image.tag }}` | Helm values at install time |

The `ERW_IMAGE_PLACEHOLDER` and `ERW_EQUALIZER_IMAGE_PLACEHOLDER` tokens are
**deleted entirely** from the repository — no file retains them after migration.

## 5. File deletion checklist

These files are deleted in the migration phase (after all consumers are migrated):

- `deploy/deployment.yaml`
- `deploy/rbac.yaml`
- `deploy/crds.yaml`
- `deploy/webhook-config.yaml`
- `deploy/cert-setup.yaml`
- `deploy/equalizer/deployment.yaml`
- `deploy/equalizer/rbac.yaml`
- `deploy/equalizer/crds.yaml`
- `deploy/equalizer/equalizer-config.example.yaml` (content migrated to
  `deploy/kustomize/equalizer/example-config.yaml` + chart, then original
  deleted)
- `deploy/equalizer/` directory (emptied and removed)

After deletion, `deploy/` contains only: `kustomize/`, `charts/`.
