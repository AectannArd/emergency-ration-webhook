# Manifest Bundles — Installation via Kustomize and Helm

[← Back to README](../README.md)

The project ships two templated manifest formats for each component so an
operator can install the webhook or equalizer without hand-editing YAML:

- **Kustomize** — the manifest source of truth, rendered at apply time with
  `kustomize build` / `kubectl kustomize`.
- **Helm** — a parameterized chart packaged as a versioned `.tgz` attached to
  each [GitHub Release](https://github.com/AectannArd/emergency-ration-webhook/releases).

Both formats produce field-equivalent resources; choose whichever fits your
workflow. For raw manifest reference, see the
[Deployment Guide](./deployment.md).

## Component inventory

| Component | Kustomize bundle | Helm chart |
|-----------|-----------------|------------|
| Webhook | [`deploy/kustomize/webhook/`](../deploy/kustomize/webhook/) | `emergency-ration-webhook` |
| Equalizer | [`deploy/kustomize/equalizer/`](../deploy/kustomize/equalizer/) | `emergency-ration-equalizer` |

## Kustomize

### Install the webhook

```sh
# Override the image to your registry/tag, then build + apply.
kubectl kustomize deploy/kustomize/webhook \
  | sed 's|aectann/emergency-ration-webhook:latest|aectann/emergency-ration-webhook:v1.0.0|' \
  | kubectl apply -f -
```

Or use the standalone `kustomize` binary with `kustomize edit set image`:

```sh
cd deploy/kustomize/webhook
kustomize edit set image capacity-admission-webhook=aectann/emergency-ration-webhook:v1.0.0
kustomize build | kubectl apply -f -
```

The bundle applies all 11 webhook resources (Namespace, Deployment, Service,
two CRDs, RBAC, ValidatingWebhookConfiguration, cert-manager Issuer +
Certificate) in the correct dependency order.

### Install the equalizer

```sh
kubectl kustomize deploy/kustomize/equalizer \
  | sed 's|aectann/emergency-ration-equalizer:latest|aectann/emergency-ration-equalizer:v1.0.0|' \
  | kubectl apply -f -
```

After deploying the equalizer, apply an `EqualizerConfig` singleton with your
target clusters — see the
[example config](../deploy/kustomize/equalizer/example-config.yaml) and the
[Equalizer guide](./equalizer.md).

### Image override mechanism

The `kustomization.yaml` `images:` directive declares the default image:

```yaml
images:
  - name: capacity-admission-webhook
    newName: aectann/emergency-ration-webhook
    newTag: latest
```

Override via `kustomize edit set image`, a sed on the rendered output, or your
own overlay.

## Helm

### Install the webhook

Download the chart `.tgz` from the
[latest release](https://github.com/AectannArd/emergency-ration-webhook/releases),
then install:

```sh
helm install erw ./emergency-ration-webhook-<version>.tgz \
  --set image.repository=aectann/emergency-ration-webhook \
  --set image.tag=v1.0.0
```

Or install directly from the repository at a specific tag:

```sh
helm install erw deploy/charts/webhook \
  --set image.tag=v1.0.0
```

### Install the equalizer

```sh
helm install eq ./emergency-ration-equalizer-<version>.tgz \
  --set image.tag=v1.0.0
```

### Values reference — webhook

| Value | Default | Description |
|-------|---------|-------------|
| `image.repository` | `aectann/emergency-ration-webhook` | Container image repository |
| `image.tag` | `latest` | Container image tag |
| `image.pullPolicy` | `Always` | Kubernetes image pull policy |
| `namespace` | `capacity-admission` | Namespace for namespaced resources |
| `replicas` | `2` | Deployment replica count |
| `budget.defaultPercent` | `80` | Default budget for the auto-created Allocation singleton |
| `resources.requests.cpu` | `100m` | Container CPU request |
| `resources.requests.memory` | `128Mi` | Container memory request |
| `resources.limits.cpu` | `500m` | Container CPU limit |
| `resources.limits.memory` | `256Mi` | Container memory limit |
| `certManager.enabled` | `true` | Deploy cert-manager Issuer + Certificate (set false for manual TLS) |

### Values reference — equalizer

| Value | Default | Description |
|-------|---------|-------------|
| `image.repository` | `aectann/emergency-ration-equalizer` | Container image repository |
| `image.tag` | `latest` | Container image tag |
| `image.pullPolicy` | `Always` | Kubernetes image pull policy |
| `namespace` | `capacity-equalizer` | Namespace for namespaced resources |
| `reconcile.intervalSeconds` | `10` | `EQUALIZER_RECONCILE_INTERVAL_SECS` env value |
| `resources.requests.cpu` | `50m` | Container CPU request |
| `resources.requests.memory` | `64Mi` | Container memory request |
| `resources.limits.cpu` | `250m` | Container CPU limit |
| `resources.limits.memory` | `128Mi` | Container memory limit |

### Chart versioning

The chart `version` field in `Chart.yaml` is `0.0.0-dev` in-repo; the release
workflow stamps it from the git tag at package time (`v1.0.0` → `1.0.0`). Each
tagged release attaches two `.tgz` files to the GitHub Release:

- `emergency-ration-webhook-<version>.tgz`
- `emergency-ration-equalizer-<version>.tgz`

## TLS provisioning

Both formats default to cert-manager for TLS (the webhook chart's
`certManager.enabled: true` value; the Kustomize bundle always includes the
cert-manager resources). For manual TLS (no cert-manager), set
`certManager.enabled: false` in Helm or remove the cert-setup resources in your
Kustomize overlay, then provide the TLS Secret manually — see the
[Deployment Guide → TLS Provisioning](./deployment.md#tls-provisioning).
