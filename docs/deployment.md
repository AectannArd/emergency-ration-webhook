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

To deploy it, replace the `ERW_IMAGE_PLACEHOLDER` token in the `image:` field of
[`deploy/deployment.yaml`](../deploy/deployment.yaml) with the reference above
(e.g. `aectann/emergency-ration-webhook:v1.0.0`). The `erw-verify` tool performs
this substitution automatically from `.env` — see
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
`kind`/`k3d` cluster), then replace the `ERW_IMAGE_PLACEHOLDER` token in the
`image:` field of [`deploy/deployment.yaml`](../deploy/deployment.yaml) with your
image reference. (The `erw-verify` tool performs this substitution automatically
from `.env` — see [On-Demand Verification](./erw-verify.md).)

> **Image pull policy**: `deploy/deployment.yaml` sets `imagePullPolicy: Always`
> (the image comes from a remote registry). For an air-gapped / offline cluster
> where you load the image locally (`kind`/`k3d`), change it to `IfNotPresent` or
> `Never` so the kubelet does not try to re-pull from a registry it cannot reach.

## Deploy to Kubernetes

Apply the manifests in [`deploy/`](../deploy/). The order below ensures the
`capacity-admission` namespace exists before the namespaced resources, and that
the webhook's own pods are not gated by their own webhook (the
[bootstrap exclusion](./failure-modes.md#webhook-self-admission-bootstrap)).

**1. CRDs** — register the `ClusterCapacity` and `Allocation` custom resources:

```sh
kubectl apply -f deploy/crds.yaml
```

**2. Namespace + Deployment + Service** — creates namespace `capacity-admission`,
a 2-replica `Deployment`, and the `Service` exposing the webhook:

```sh
kubectl apply -f deploy/deployment.yaml
```

**3. RBAC** — `ServiceAccount`, least-privilege `ClusterRole` (read on nodes and
pods; read/write on the two CRDs' `/status`), and the binding:

```sh
kubectl apply -f deploy/rbac.yaml
```

**4. TLS certificate** — provision the serving certificate the webhook mounts at
`/tls`. Follow [TLS Provisioning](#tls-provisioning) (cert-manager is the default;
a manual Secret is the fallback).

**5. ValidatingWebhookConfiguration** — registers the webhook with the API server:

```sh
kubectl apply -f deploy/webhook-config.yaml
```

**6. Singletons & budget** — you're done. On startup the controllers auto-create
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
> or `CrashLoopBackOff` until steps 3 and 4 complete.

### TLS Provisioning

The admission endpoint is HTTPS, so a serving certificate is **mandatory**. Two
paths:

**Automated (cert-manager, default).** Apply
[`deploy/cert-setup.yaml`](../deploy/cert-setup.yaml), which declares a self-signed
`Issuer` and a `Certificate` that writes the serving key/cert into the
`capacity-admission-webhook-tls` `Secret` the Deployment mounts:

```sh
kubectl apply -f deploy/cert-setup.yaml
```

The same manifest carries the `cert-manager.io/inject-ca-from` annotation on
[`deploy/webhook-config.yaml`](../deploy/webhook-config.yaml); cert-manager's
ca-injector then populates the webhook's `clientConfig.caBundle` automatically.

**Manual Secret (no cert-manager).** Generate a key + cert whose SANs cover the
in-cluster Service DNS, create the `Secret`, and base64-encode the cert into the
webhook config's `caBundle`:

```sh
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

kubectl -n capacity-admission create secret tls capacity-admission-webhook-tls \
  --cert=tls.crt --key=tls.key

# Inject the CA bundle into the webhook config (replacing the placeholder):
CABUNDLE="$(base64 -w0 tls.crt)"
sed "s|# caBundle: .*|caBundle: ${CABUNDLE}|" deploy/webhook-config.yaml \
  | kubectl apply -f -
```
