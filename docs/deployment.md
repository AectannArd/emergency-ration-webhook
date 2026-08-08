# Deployment Guide

[← Back to README](../README.md)

## Published Image

A pre-built, multi-arch image is published to Docker Hub on every release tag,
so you can skip [Build the Image](#build-the-image) and pull directly. The image
supports **linux/amd64** and **linux/arm64** — `docker pull` selects the correct
variant for your host automatically:

```sh
docker pull aectann/emergency-ration-webhook:v1.0.0   # a specific release
docker pull aectann/emergency-ration-webhook:latest   # the latest stable release
```

The webhook Kustomize bundle defaults to `aectann/emergency-ration-webhook:latest`.
To deploy a specific tag, override it when applying (see
[Deploy to Kubernetes](#deploy-to-kubernetes)). The `erw-verify` tool performs this
override automatically from `.env` — see
[On-Demand Verification](./erw-verify.md).

**Tag conventions** (releases are cut by pushing a git tag — see
[`CONTRIBUTING.md`](../CONTRIBUTING.md) under *Publishing / Releases*):

- `vX.Y.Z` (e.g. `v1.0.0`) — a stable release. Also updates `latest`.
- `vX.Y.Z-rc.N` / `vX.Y.Z-beta.N` (e.g. `v1.0.0-rc.1`) — a pre-release. **Does
  not** update `latest`, so `latest` always tracks the newest stable image.

## Build the Image

The [`Dockerfile`](../Dockerfile) is a multi-stage build (Rust 1.89 builder on a
distroless runtime base):

```sh
docker build -t capacity-admission-webhook:latest .
```

Push the image to a registry your cluster can reach (or load it locally for a
`kind`/`k3d` cluster), then override the bundle default image when applying (see
[Deploy to Kubernetes](#deploy-to-kubernetes)). (The `erw-verify` tool performs
this override automatically from `.env` — see
[On-Demand Verification](./erw-verify.md).)

> **Image pull policy**: the webhook Kustomize bundle sets `imagePullPolicy: Always`
> (the image comes from a remote registry). For an air-gapped / offline cluster
> where you load the image locally (`kind`/`k3d`), change it to `IfNotPresent` or
> `Never` in the rendered output so the kubelet does not try to re-pull from a
> registry it cannot reach.

## Deploy to Kubernetes

Apply the webhook Kustomize bundle in
[`deploy/kustomize/webhook/`](../deploy/kustomize/webhook/) — the manifest source of
truth. It bundles the `capacity-admission` namespace, CRDs, RBAC, Deployment,
Service, and ValidatingWebhookConfiguration in dependency order, so the namespace
exists before the namespaced resources and the webhook's own pods are not gated by
their own webhook (the
[bootstrap exclusion](./failure-modes.md#webhook-self-admission-bootstrap)):

```sh
kubectl kustomize deploy/kustomize/webhook | kubectl apply -f -
```

The bundle's `images:` directive resolves the Deployment image to its default
(`aectann/emergency-ration-webhook:latest`). To pin a specific tag or use your own
image, override it on the rendered output:

```sh
kubectl kustomize deploy/kustomize/webhook \
  | sed 's|aectann/emergency-ration-webhook:latest|<your-image-reference>|' \
  | kubectl apply -f -
```

**TLS certificate** — provision the serving certificate the webhook mounts at
`/tls`. Follow [TLS Provisioning](#tls-provisioning) (cert-manager is the default,
included in the bundle; a manual Secret is the fallback).

**Singletons & budget** — you're done. On startup the controllers auto-create
both singleton instances; **neither needs to be created manually**:

- `cluster-capacity` (`ClusterCapacity`, empty spec) — the supply side, refreshed
  by the Node Capacity Controller from every node's `.status.allocatable`.
- `cluster-allocation` (`Allocation`, `spec.budgetPercent: 80`) — the demand side.
  **80%** is the auto-created default, leaving 20% headroom for system daemons,
  node overhead, and spikes.

To change the budget at runtime, patch the Allocation spec — see
[Adjusting the Budget at Runtime](./configuration.md#adjusting-the-budget-at-runtime):

```sh
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"budgetPercent":70}}'
```

The controllers never overwrite an existing instance, so any operator-set
`budgetPercent` is preserved across restarts.

> The webhook `Deployment` pods retry until the RBAC `ServiceAccount` and the TLS
> `Secret` exist, so it is normal for them to sit in a brief `CreateContainerConfigError`
> or `CrashLoopBackOff` until the TLS certificate is provisioned.

### TLS Provisioning

The admission endpoint is HTTPS, so a serving certificate is **mandatory**. Two
paths:

**Automated (cert-manager, default).** The webhook Kustomize bundle includes
`cert-setup.yaml`, which declares a self-signed `Issuer` and a `Certificate` that
writes the serving key/cert into the `capacity-admission-webhook-tls` `Secret` the
Deployment mounts. Applying the bundle (above) with cert-manager installed issues
the cert automatically. The same resources work with the
`cert-manager.io/inject-ca-from` annotation on the bundle's
ValidatingWebhookConfiguration; cert-manager's ca-injector then populates the
webhook's `clientConfig.caBundle` automatically.

**Manual Secret (no cert-manager).** When cert-manager is not installed (e.g. in
CI), filter the bundle's cert-manager resources out and provision TLS yourself:
generate a key + cert whose SANs cover the in-cluster Service DNS, create the
`Secret` before applying the Deployment, then inject the cert into the webhook's
`caBundle`. The rendered ValidatingWebhookConfiguration has no `caBundle` line
(kustomize drops the source comment), so inject it with a strategic
`kubectl patch` — it merges `webhooks[]` by name, setting only
`clientConfig.caBundle` without clobbering `failurePolicy`/`rules`:

```sh
# 1. Generate the self-signed cert (SANs = the in-cluster Service DNS).
cat > csr.conf <<'EOF'
[req]
req_extensions = v3_req
distinguished_name = req_distinguished_name
[v3_req]
subjectAltName = @alt_names
[alt_names]
DNS.1 = capacity-admission-webhook
DNS.2 = capacity-admission-webhook.capacity-admission
DNS.3 = capacity-admission-webhook.capacity-admission.svc
[req_distinguished_name]
CN = capacity-admission-webhook
EOF
openssl req -x509 -newkey rsa:2048 -nodes -keyout tls.key -out tls.crt \
  -days 365 -subj "/CN=capacity-admission-webhook" -config csr.conf -extensions v3_req

# 2. Create the namespace + TLS Secret BEFORE the Deployment (pods mount /tls).
kubectl create namespace capacity-admission --dry-run=client -o yaml | kubectl apply -f -
kubectl -n capacity-admission create secret tls capacity-admission-webhook-tls \
  --cert=tls.crt --key=tls.key

# 3. Apply the bundle without the cert-manager Issuer/Certificate.
kubectl kustomize deploy/kustomize/webhook \
  | awk 'BEGIN{RS="\n---\n"; ORS="\n---\n"} !/apiVersion: cert-manager\.io/' \
  | kubectl apply -f -

# 4. Inject the CA bundle (strategic patch merges webhooks[] by name).
CABUNDLE="$(base64 -w0 tls.crt)"
kubectl patch validatingwebhookconfiguration capacity-admission.emergency-ration.dev \
  --type=strategic \
  -p '{"webhooks":[{"name":"capacity-admission.emergency-ration.dev","clientConfig":{"caBundle":"'"${CABUNDLE}"'"}}]}'
```
