# Contract: Helm Chart

## 1. Chart inventory

| Chart | Directory | Chart name | Components |
|-------|-----------|------------|------------|
| Webhook | `deploy/charts/webhook/` | `emergency-ration-webhook` | 11 resources (Namespace, Deployment, Service, 2 CRDs, SA, ClusterRole, CRB, VWC, Issuer, Certificate) |
| Equalizer | `deploy/charts/equalizer/` | `emergency-ration-equalizer` | 6 resources (Namespace, Deployment, CRD, SA, ClusterRole, CRB) |

Two independent charts. Each installs into its own namespace. No umbrella chart,
no subcharts, no dependencies on other charts.

## 2. Chart.yaml contract

### 2.1 Webhook (`deploy/charts/webhook/Chart.yaml`)

```yaml
apiVersion: v2
name: emergency-ration-webhook
description: A Kubernetes validating admission webhook that enforces a configurable cluster capacity budget for CPU and RAM.
type: application
version: 0.0.0-dev        # stamped at release time (research R7)
appVersion: "0.0.0-dev"   # stamped at release time
```

### 2.2 Equalizer (`deploy/charts/equalizer/Chart.yaml`)

```yaml
apiVersion: v2
name: emergency-ration-equalizer
description: Multi-cluster capacity equalizer controller that balances cumulative capacity across a fleet of Kubernetes clusters.
type: application
version: 0.0.0-dev
appVersion: "0.0.0-dev"
```

`apiVersion: v2` (Helm 3). `type: application` (not library). The `version`
field is set to `0.0.0-dev` in-repo; the release workflow stamps the real semver
at package time (research R7).

## 3. Values schema

### 3.1 Webhook (`deploy/charts/webhook/values.yaml`)

```yaml
# Container image (override to point at a different registry/tag).
image:
  repository: aectann/emergency-ration-webhook
  tag: latest
  pullPolicy: Always

# Namespace for all namespaced resources (CRDs + ClusterRole are cluster-scoped).
namespace: capacity-admission

# Webhook deployment.
replicas: 2

# Default budget percent for the auto-created Allocation singleton.
budget:
  defaultPercent: 80

# Container resources (matches deploy/deployment.yaml defaults).
resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    cpu: 500m
    memory: 256Mi

# TLS — cert-manager is the default; set to false for manual Secret.
certManager:
  enabled: true
```

### 3.2 Equalizer (`deploy/charts/equalizer/values.yaml`)

```yaml
# Container image.
image:
  repository: aectann/emergency-ration-equalizer
  tag: latest
  pullPolicy: Always

# Namespace.
namespace: capacity-equalizer

# Reconcile interval (EQUALIZER_RECONCILE_INTERVAL_SECS env).
reconcile:
  intervalSeconds: 10

# Container resources (matches deploy/equalizer/deployment.yaml defaults).
resources:
  requests:
    cpu: 50m
    memory: 64Mi
  limits:
    cpu: 250m
    memory: 128Mi
```

The EqualizerConfig singleton and kubeconfig Secrets are NOT templated in the
chart — they are operator-specific runtime config. The chart ships an
`example-config.yaml` (or commented template) showing how to create them after
install.

## 4. Template contract

Each template file produces one or more resources via Go templates. Every
template MUST use `.Values` for parameterized fields (image, namespace, replicas,
resources, budget).

### 4.1 Critical field preservation

These fields are hardcoded in templates (NOT values-exposed) and MUST match the
pre-migration manifests exactly:

- `failurePolicy: Fail` (ValidatingWebhookConfiguration)
- `sideEffects: None`
- `timeoutSeconds: 5`
- `matchPolicy: Exact`
- `admissionReviewVersions: ["v1"]`
- All RBAC verb lists (data-model §2.2)
- `securityContext` values (runAsNonRoot, runAsUser 65532, etc.)
- Container ports (8443 webhook, 9090 metrics)
- Probe paths (/healthz) and ports (metrics)
- Volume mount paths (/tls)
- The cert-manager annotation
  (`cert-manager.io/inject-ca-from: capacity-admission/capacity-admission-webhook`)
- The namespaceSelector (NotIn capacity-admission)

### 4.2 Conditional: cert-manager

The cert-manager resources (Issuer + Certificate) are wrapped in:
```gotemplate
{{- if .Values.certManager.enabled }}
...
{{- end }}
```
When `certManager.enabled: false`, the operator must provide the TLS Secret
manually (same fallback as the pre-migration `cert-setup.yaml` comments).

## 5. _helpers.tpl

A `_helpers.tpl` file defines reusable named templates:
- `webhook.name` / `equalizer.name` — the resource base name
- `webhook.labels` / `equalizer.labels` — standard labels
- `webhook.namespace` / `equalizer.namespace` — `.Values.namespace`

## 6. Validation gate

`helm lint deploy/charts/<component>` MUST pass with zero errors. This is:
1. A local developer check.
2. A CI gate (ci.yml quality job).
3. A release gate (publish.yml — failure blocks the chart from being attached).

## 7. Packaging (release)

```sh
helm package deploy/charts/webhook   # → emergency-ration-webhook-<version>.tgz
helm package deploy/charts/equalizer # → emergency-ration-equalizer-<version>.tgz
```

The `.tgz` is attached to the GitHub Release (contract `release-workflow.md`).

## 8. Parity with Kustomize

The chart templates, when rendered via `helm template`, MUST produce
field-equivalent resources to `kustomize build deploy/kustomize/<component>` on
all critical fields (data-model §2). A parity test enforces this (research R9).
Minor formatting/label differences are acceptable; contract-critical fields are
not.
