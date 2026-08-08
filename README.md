# emergency-ration-webhook

> A Kubernetes validating admission webhook that enforces a configurable cluster
> capacity budget for CPU and RAM — **fail-closed by design**. CI: GitHub Actions ·
> License: Apache-2.0 · Kubernetes: 1.34–1.36 (N-2 support window)

`emergency-ration-webhook` prevents cluster overcommit. It tracks how much CPU and
memory the workloads **scheduled** in a cluster have requested against a percentage
budget of the cluster's total allocatable capacity, and rejects any pod admission
that would push a resource past its budget. Once it is installed, no pod in a
monitored namespace can be created or updated without first passing the budget
check.

For everything else — configuration, operations, and troubleshooting — see the
[Documentation](#documentation) index below, or browse [`docs/`](./docs/). Deeper
design material lives under
[`specs/001-capacity-admission-webhook/`](./specs/001-capacity-admission-webhook/).

## Overview

The webhook is a **validating admission webhook**: the Kubernetes API server
forwards pod `CREATE` and `UPDATE` requests to it, and it answers *allow* or
*deny*. Its single job is to keep the cluster from being over-allocated: if
admitting a pod would push scheduled CPU or memory requests past the configured
percentage of total allocatable capacity, the pod is rejected with a message that
names the offending resource and the exact figures.

It exists because Kubernetes default scheduling only checks whether a pod *fits on
a node right now* — it does not protect an operator-defined total headroom for the
whole cluster. Workloads can quietly overcommit aggregate capacities, leaving no
buffer for failures, upgrades, or spikes. `emergency-ration-webhook` turns that
headroom into a hard, auditable budget.

The defining property is **fail-closed** (Constitution Principle I): whenever the
webhook cannot authoritatively verify that a workload fits — stale capacity data,
missing state, a decision timeout, a malformed request, or an internal panic — it
**rejects**. A denial is always a safe outcome; admitting under degraded
knowledge is never safe. The `ValidatingWebhookConfiguration` uses
`failurePolicy: Fail`, so the API server itself rejects if the webhook is
unreachable. There is no "best-effort" or silent-admit path. The full feature
specification is in
[`specs/001-capacity-admission-webhook/spec.md`](./specs/001-capacity-admission-webhook/spec.md).

## Quick Start

This section takes an operator from a fresh clone to a running, budget-enforcing
webhook in a Kubernetes cluster.

### Prerequisites

- A Kubernetes cluster (1.34–1.36; see [Kubernetes Compatibility](./docs/kubernetes-compatibility.md))
- `kubectl` configured against that cluster
- A container runtime that can build an image and get it into the cluster:
  - **Build from source**: the Rust toolchain (MSRV **1.89**), and `docker` to
    build the image, **or**
  - **Pre-built image**: pull the published image from Docker Hub — no build
    step required (see [Published Image](#published-image); skip
    [Build the Image](./docs/deployment.md#build-the-image))
- For automated TLS (recommended): [cert-manager](https://cert-manager.io/)
  installed in the cluster. Without it, use the manual Secret path in
  [TLS Provisioning](./docs/deployment.md#tls-provisioning).

### Published Image

A pre-built, multi-arch (`linux/amd64` + `linux/arm64`) image is published to
Docker Hub on every release tag:

```sh
docker pull aectann/emergency-ration-webhook:v1.0.0   # a specific release
docker pull aectann/emergency-ration-webhook:latest   # the latest stable release
```

The webhook Kustomize bundle defaults to `aectann/emergency-ration-webhook:latest`;
override it with the reference above when applying (see
[Deploy to Kubernetes](#deploy-to-kubernetes)). For the full tag conventions
(`vX.Y.Z`, pre-releases, `latest` tracking), see the
[Deployment Guide](./docs/deployment.md#published-image); to build from source
instead, see [Build the Image](./docs/deployment.md#build-the-image).

### Deploy to Kubernetes

Apply the webhook Kustomize bundle in
[`deploy/kustomize/webhook/`](./deploy/kustomize/webhook/) — the manifest source of
truth. It bundles the `capacity-admission` namespace, CRDs, RBAC, Deployment,
Service, and ValidatingWebhookConfiguration in dependency order, so the namespace
exists before the namespaced resources and the webhook's own pods are not gated by
their own webhook (the
[bootstrap exclusion](./docs/failure-modes.md#webhook-self-admission-bootstrap)):

```sh
kubectl kustomize deploy/kustomize/webhook | kubectl apply -f -
```

The bundle defaults to the published image (`aectann/emergency-ration-webhook:latest`).
To pin a specific tag or use your own image, override it on the rendered output:

```sh
kubectl kustomize deploy/kustomize/webhook \
  | sed 's|aectann/emergency-ration-webhook:latest|aectann/emergency-ration-webhook:v1.0.0|' \
  | kubectl apply -f -
```

**TLS certificate** — provision the serving certificate the webhook mounts at
`/tls`. With [cert-manager](https://cert-manager.io/) installed, the bundle's
`cert-setup.yaml` issues it automatically and injects the `caBundle`. Without
cert-manager, use the manual Secret path in
[TLS Provisioning](./docs/deployment.md#tls-provisioning).

**Singletons & budget** — you're done. On startup the controllers auto-create
both singleton instances; **neither needs to be created manually**:

   - `cluster-capacity` (`ClusterCapacity`, empty spec) — the supply side.
   - `cluster-allocation` (`Allocation`, `spec.budgetPercent: 80`) — the demand
     side. **80%** is the auto-created default, leaving 20% headroom.

   To change the budget at runtime, patch the Allocation spec — see
   [Adjusting the Budget at Runtime](./docs/configuration.md#adjusting-the-budget-at-runtime):

   ```sh
   kubectl patch allocation cluster-allocation --type=merge \
     -p '{"spec":{"budgetPercent":70}}'
   ```

The controllers never overwrite an existing instance, so any operator-set
`budgetPercent` is preserved across restarts.

> The webhook `Deployment` pods retry until the RBAC `ServiceAccount` and the TLS
> `Secret` exist, so it is normal for them to sit in a brief `CreateContainerConfigError`
> or `CrashLoopBackOff` until steps 3 and 4 complete.

### Verify

Once the controllers have reconciled (a few seconds), check the installation:

```sh
# Both webhook replicas are Ready.
kubectl -n capacity-admission get pods -l app=capacity-admission-webhook

# The webhook is registered.
kubectl get validatingwebhookconfiguration capacity-admission.emergency-ration.dev

# The controllers have populated capacity state.
kubectl get clustercapacities.emergency-ration.dev cluster-capacity -o yaml
kubectl get allocations.emergency-ration.dev cluster-allocation -o yaml
```

Reach the plaintext health endpoint over a port-forward (the metrics port is
HTTP, not TLS):

```sh
kubectl -n capacity-admission port-forward svc/capacity-admission-webhook 9090:metrics &
curl -s localhost:9090/healthz   # → ok
```

Finally, confirm the budget is enforced. A small pod is admitted; an over-budget
request is denied with a message citing the violated resource:

```sh
# Admitted — small requests, well within budget.
kubectl -n default run smoke-ok --image=nginx \
  --requests='cpu=10m,memory=10Mi' --restart=Never

# Rejected — exceeds the budget (fail-closed).
kubectl -n default run smoke-over --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never
```

## Documentation

The reference is split into one article per capability under [`docs/`](./docs/) —
see the [documentation index](./docs/README.md) for the full list. For build
instructions, testing, and project structure, see
[`CONTRIBUTING.md`](./CONTRIBUTING.md).

### Getting Started

- **[Deployment Guide](./docs/deployment.md)** — building the image, the 6-step
  deploy sequence, and TLS provisioning (cert-manager or manual Secret).

### Configuration

- **[Configuration Reference](./docs/configuration.md)** — CLI flags and env vars,
  the `Allocation` and `ClusterCapacity` CRD field tables, runtime budget
  adjustment, per-resource overrides, and budget edge cases.
- **[Node Exclusion](./docs/node-exclusion.md)** — the two-layer node filter
  (unschedulable + label selectors), the spec-006→007 migration, and selector
  examples.
- **[Enforcement Modes](./docs/enforcement-modes.md)** — `enforce` vs `dry-run`,
  the fail-closed-in-both-modes contract, and runtime switching.
- **[Workload Exclusion](./docs/workload-exclusion.md)** — namespace and
  priority-class exemption lists, the check order, and the still-counted
  semantics.

### Operations

- **[Metrics & Observability](./docs/observability.md)** — HTTP endpoints, the 8
  Prometheus metrics, structured logging fields, and the rejection message
  format.
- **[Failure Modes](./docs/failure-modes.md)** — every degradation path, the
  fail-closed contract, and the webhook self-admission bootstrap.
- **[Kubernetes Compatibility](./docs/kubernetes-compatibility.md)** — the N-2
  support window, the CI version matrix, and the GA APIs the webhook depends on.

### Architecture

- **[Architecture](./docs/architecture.md)** — the 3-component operator data flow
  and the two CRDs that link them.

### Tooling

- **[On-Demand Verification (erw-verify)](./docs/erw-verify.md)** — the
  throwaway-cluster verification tool: build, configure, run, CLI flags, exit
  codes, and the S1–S11 / E1–E5 scenario inventory.

### Equalizer

- **[Multi-Cluster Capacity Equalizer](./docs/equalizer.md)** — the separate
  `capacity-equalizer` binary that balances cumulative capacity across a fleet of
  clusters.

## License

Licensed under **Apache-2.0** — see [`LICENSE`](./LICENSE).
