# Contract: Kustomize Bundle

## 1. Layout

```
deploy/kustomize/webhook/
├── kustomization.yaml
├── crds.yaml              # ClusterCapacity + Allocation CRDs (11 resources total)
├── deployment.yaml        # Namespace + Deployment + Service
├── rbac.yaml              # ServiceAccount + ClusterRole + ClusterRoleBinding
├── webhook-config.yaml    # ValidatingWebhookConfiguration
└── cert-setup.yaml        # cert-manager Issuer + Certificate

deploy/kustomize/equalizer/
├── kustomization.yaml
├── crds.yaml              # EqualizerConfig CRD (6 resources total)
├── deployment.yaml        # Namespace + Deployment
├── rbac.yaml              # ServiceAccount + ClusterRole + ClusterRoleBinding
└── example-config.yaml    # EqualizerConfig + kubeconfig Secret EXAMPLE (NOT in resources:)
```

## 2. kustomization.yaml contract

### 2.1 Webhook

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - crds.yaml
  - deployment.yaml
  - rbac.yaml
  - webhook-config.yaml
  - cert-setup.yaml

images:
  - name: capacity-admission-webhook
    newName: aectann/emergency-ration-webhook
    newTag: latest
```

### 2.2 Equalizer

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - crds.yaml
  - deployment.yaml
  - rbac.yaml

images:
  - name: capacity-equalizer
    newName: aectann/emergency-ration-equalizer
    newTag: latest
```

The `example-config.yaml` is deliberately NOT listed in `resources:` — it is
documentation, not a deployable resource (contains placeholder kubeconfig data).

## 3. Image override contract

The `images:` directive in `kustomization.yaml` declares the default
(Docker Hub) image. Override at apply time:

```sh
# Option A: kustomize edit (modifies kustomization.yaml on disk)
cd deploy/kustomize/webhook
kustomize edit set image capacity-admission-webhook=aectann/emergency-ration-webhook:v1.0.0
kustomize build | kubectl apply -f -

# Option B: sed on rendered output (CI pattern — no file modification)
kubectl kustomize deploy/kustomize/webhook \
  | sed 's|capacity-admission-webhook:latest|capacity-admission-webhook:v1.0.0|' \
  | kubectl apply -f -
```

The `name:` in `images:` (`capacity-admission-webhook`) is the image reference
in the Deployment's `container.image:` field. The `newName`/`newTag` are the
default resolved values.

## 4. Resource content contract

Every YAML file in the bundle MUST be byte-for-byte identical to the
corresponding pre-migration raw manifest, EXCEPT:

- The `image:` field uses `capacity-admission-webhook:placeholder` (webhook) or
  `capacity-equalizer:placeholder` (equalizer) instead of
  `ERW_IMAGE_PLACEHOLDER` / `ERW_EQUALIZER_IMAGE_PLACEHOLDER`. The
  `kustomization.yaml` `images:` directive resolves this to the real reference.
- Header comments may be updated to reference the new path.

No field may be added, removed, or changed beyond the image field. The parity
test (research R9) enforces this.

## 5. Build output

```sh
kustomize build deploy/kustomize/webhook
```

Produces a single YAML stream of all 11 webhook resources, in `resources:` order,
with the image field resolved to `aectann/emergency-ration-webhook:latest` (or
the overridden value). The output is valid `kubectl apply -f -` input.

## 6. Namespace handling

The Namespace resource is included in `deployment.yaml` and listed in
`resources:`. Kustomize does not apply a namespace transformer by default —
resources keep their hardcoded namespace (`capacity-admission` /
`capacity-equalizer`). Operators who need a different namespace override it in
their own overlay; the in-repo bundle uses the canonical namespaces.
