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

This README is the single entry point for installing, configuring, operating, and
troubleshooting the webhook. Deeper design material (full 3-component
architecture, CRD data-model, admission contracts) lives under
[`specs/001-capacity-admission-webhook/`](./specs/001-capacity-admission-webhook/)
and is linked where relevant — but everything an operator needs day-to-day is
here.

## Overview

The webhook is a **validating admission webhook**: the Kubernetes API server
forwards pod `CREATE` and `UPDATE` requests to it, and it answers *allow* or
*deny*. Its single job is to keep the cluster from being over-allocated: if
admitting a pod would push scheduled CPU or memory requests past the configured
percentage of total allocatable capacity, the pod is rejected with a message that
names the offending resource and the exact figures.

It exists because Kubernetes default scheduling only checks whether a pod *fits on
a node right now* — it does not protect an operator-defined total headroom for the
whole cluster. Workloads can quietly overcommit aggregate capacity, leaving no
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

- A Kubernetes cluster (1.34–1.36; see [Kubernetes Compatibility](#kubernetes-compatibility))
- `kubectl` configured against that cluster
- A container runtime that can build an image and get it into the cluster:
  - **Build from source**: the Rust toolchain (MSRV **1.89**), and `docker` to
    build the image, **or**
  - **Pre-built image**: an image in a registry the cluster can pull (no build
    step required — skip [Build the Image](#build-the-image) and point
    `deploy/deployment.yaml` at your image)
- For automated TLS (recommended): [cert-manager](https://cert-manager.io/)
  installed in the cluster. Without it, use the manual Secret path in
  [TLS Provisioning](#tls-provisioning).

### Build the Image

The [`Dockerfile`](./Dockerfile) is a multi-stage build (Rust 1.89 builder on a
distroless runtime base):

```sh
docker build -t capacity-admission-webhook:latest .
```

Push the image to a registry your cluster can reach (or load it locally for a
`kind`/`k3d` cluster), then replace the `ERW_IMAGE_PLACEHOLDER` token in the
`image:` field of [`deploy/deployment.yaml`](./deploy/deployment.yaml) with your
image reference. (The `erw-verify` tool performs this substitution automatically
from `.env` — see [On-Demand Verification](#on-demand-verification-erw-verify).)

> **Image pull policy**: `deploy/deployment.yaml` sets `imagePullPolicy: Always`
> (the image comes from a remote registry). For an air-gapped / offline cluster
> where you load the image locally (`kind`/`k3d`), change it to `IfNotPresent` or
> `Never` so the kubelet does not try to re-pull from a registry it cannot reach.

### Deploy to Kubernetes

Apply the manifests in [`deploy/`](./deploy/). The order below ensures the
`capacity-admission` namespace exists before the namespaced resources, and that
the webhook's own pods are not gated by their own webhook (the
[bootstrap exclusion](#webhook-self-admission-bootstrap)).

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
[Adjusting the Budget at Runtime](#adjusting-the-budget-at-runtime):

```sh
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"budgetPercent":70}}'
```

The controllers never overwrite an existing instance, so any operator-set
`budgetPercent` is preserved across restarts.

> The webhook `Deployment` pods retry until the RBAC `ServiceAccount` and the TLS
> `Secret` exist, so it is normal for them to sit in a brief `CreateContainerConfigError`
> or `CrashLoopBackOff` until steps 3 and 4 complete.

#### TLS Provisioning

The admission endpoint is HTTPS, so a serving certificate is **mandatory**. Two
paths:

**Automated (cert-manager, default).** Apply
[`deploy/cert-setup.yaml`](./deploy/cert-setup.yaml), which declares a self-signed
`Issuer` and a `Certificate` that writes the serving key/cert into the
`capacity-admission-webhook-tls` `Secret` the Deployment mounts:

```sh
kubectl apply -f deploy/cert-setup.yaml
```

The same manifest carries the `cert-manager.io/inject-ca-from` annotation on
[`deploy/webhook-config.yaml`](./deploy/webhook-config.yaml); cert-manager's
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

## Configuration

### CLI Flags & Environment Variables

The webhook reads seven settings from CLI flags, environment variables, and
compiled defaults (source: [`src/config.rs`](./src/config.rs)). The
[`deploy/deployment.yaml`](./deploy/deployment.yaml) `Deployment` supplies these
via container `args`; the values there correspond to the compiled defaults shown
below.

| Flag | Env Var | Type | Default | Description |
|------|---------|------|---------|-------------|
| `--port` | `PORT` | u16 | `8443` | HTTPS port for the admission server |
| `--tls-cert-file` | `TLS_CERT_FILE` | path | `/tls/tls.crt` | TLS certificate path (PEM) |
| `--tls-key-file` | `TLS_KEY_FILE` | path | `/tls/tls.key` | TLS private key path (PEM) |
| `--decision-timeout-ms` | `DECISION_TIMEOUT_MS` | u64 | `100` | Admission decision timeout (ms); fails closed on expiry |
| `--capacity-freshness-timeout-secs` | `CAPACITY_FRESHNESS_TIMEOUT_SECS` | u64 | `30` | Max age (s) of capacity data before treated as stale |
| `--namespace` | `NAMESPACE` | string | `capacity-admission` | Namespace for the webhook and its CRDs |
| `--metrics-port` | `METRICS_PORT` | u16 | `9090` | HTTP port for `/metrics` and `/healthz` |

### Precedence

For each setting, the first available source wins:

1. **CLI flag** — `--flag value` on the command line
2. **Environment variable** — `FLAG_NAME`
3. **Compiled default** — the value in the table above

If a flag or env var is present but its value cannot be parsed as the expected
type, the webhook falls back to the default rather than failing to start
(FR-008).

> **Custom namespace**: the default is `capacity-admission`. Changing it requires
> updating the namespace consistently in the Deployment, RBAC, the webhook
> config's `namespaceSelector`, and the `--namespace` flag — otherwise the webhook
> will not find its CRDs.

### Allocation CRD

**Identity**: `allocations.emergency-ration.dev` (short name `alloc`), API group
`emergency-ration.dev/v1`, kind `Allocation`. Cluster-scoped singleton, convention
instance name **`cluster-allocation`**. Source:
[`src/crd/allocation.rs`](./src/crd/allocation.rs). The instance is auto-created
by the Allocation Controller with `spec.budgetPercent: 80` if absent, and an
existing instance is never overwritten (an operator-set budget is preserved).

**Spec** (the user-configurable fields):

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `budgetPercent` | integer | 0–100 | Max allocation as % of total allocatable. Applied to CPU and RAM independently. **80** is the auto-created default; change it with `kubectl patch` (see [Adjusting the Budget at Runtime](#adjusting-the-budget-at-runtime)). |
| `enforcementMode` | string enum | `enforce` \| `dry-run` | Enforcement mode (spec-004). `enforce` (default) rejects over-budget pods; `dry-run` admits them with a warning instead. Fail-closed paths reject in both modes. Absent → `enforce`. See [Enforcement Modes](#enforcement-modes-enforce--dry-run). |
| `excludedNamespaces` | array of strings | optional | List of namespace names whose pods are exempt from capacity admission (spec-008). A pod whose namespace matches ANY entry is admitted without a budget check (OR semantics with `excludedPriorityClasses`). Absent or empty → no namespaces exempted. See [Workload Exclusion](#workload-exclusion). |
| `excludedPriorityClasses` | array of strings | optional | List of priority class names whose pods are exempt from capacity admission (spec-008). Matched against `pod.spec.priorityClassName` as a **string match** (no PriorityClass resource resolution). A pod matching either list is exempt. Absent or empty → no priority classes exempted. See [Workload Exclusion](#workload-exclusion). |

**Status** (controller-computed — read-only for operators):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `allocatedCpuMilli` | integer | milli-CPUs | Sum of pod CPU requests |
| `allocatedMemoryBytes` | integer | bytes | Sum of pod memory requests |
| `ceilingCpuMilli` | integer | milli-CPUs | `floor(totalAllocatableCpuMilli × budgetPercent / 100)` |
| `ceilingMemoryBytes` | integer | bytes | Budget ceiling for memory |
| `utilizationPercentCpu` | number | ratio 0–1+ | `allocated / ceiling` for CPU |
| `utilizationPercentMemory` | number | ratio 0–1+ | `allocated / ceiling` for memory |
| `lastUpdated` | string | RFC 3339 | Last recomputation timestamp |

### ClusterCapacity CRD

**Identity**: `clustercapacities.emergency-ration.dev` (short name `cc`), API group
`emergency-ration.dev/v1`, kind `ClusterCapacity`. Cluster-scoped singleton,
convention instance name **`cluster-capacity`**. It is supply-side: the Node
Capacity Controller sums every node's `.status.allocatable` into its `status`,
refreshing on each node event. Source:
[`src/crd/cluster_capacity.rs`](./src/crd/cluster_capacity.rs). The instance is
created automatically and an existing one is never overwritten — so an
operator-set `nodeSelectors` list is preserved across restarts.

**Spec** (the user-configurable field):

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `nodeSelectors` | array (LabelSelector) | optional | List of label selectors. A node matching ANY selector is excluded from the capacity aggregate (OR semantics, spec-007). Each entry uses standard Kubernetes `LabelSelector` (`matchLabels` + `matchExpressions`). Absent or empty → only unschedulable nodes are excluded. See [Node Exclusion](#node-exclusion). |

**Status** (controller-computed — read-only for operators):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `totalAllocatableCpuMilli` | integer | milli-CPUs | Total allocatable CPU across counted nodes |
| `totalAllocatableMemoryBytes` | integer | bytes | Total allocatable memory across counted nodes |
| `nodeCount` | integer | count | Number of nodes counted toward the aggregate |
| `lastUpdated` | string | RFC 3339 | Last recomputation timestamp |
| `excludedNodeCount` | integer | count | Total nodes excluded (`excludedByUnschedulable + excludedBySelector`) |
| `excludedByUnschedulable` | integer | count | Nodes excluded because `spec.unschedulable = true` |
| `excludedBySelector` | integer | count | Nodes excluded because they matched `spec.nodeSelectors` |

### Node Exclusion

The capacity aggregate does **not** count every node — it excludes nodes the
kube-scheduler cannot place workloads on, so the budget reflects capacity the
cluster can actually use. There are two exclusion layers (spec-006 / spec-007):

1. **Unschedulable nodes (default, always on).** Any node with
   `spec.unschedulable = true` (cordoned nodes, and typically control-plane
   masters) is excluded. This cannot be disabled. It fixes a phantom-capacity
   bug where cordoned nodes inflated the supply pool.
2. **Label-selector exclusion (optional).** Set `spec.nodeSelectors` to a list of
   label selectors. A node matching **any** selector in the list is excluded (OR
   semantics); within each selector, `matchLabels` and `matchExpressions` are
   ANDed (standard Kubernetes `LabelSelector` semantics). An empty selector is a
   no-op (it excludes nothing).

A node is counted only if it passes **both** layers. When all nodes are excluded,
capacity drops to zero and the webhook fails closed on every admission (correct —
no verifiable capacity).

The status reports the breakdown: `excludedNodeCount`, `excludedByUnschedulable`,
`excludedBySelector`. A node is counted once per layer it fails — a node both
unschedulable and selector-matched counts under `excludedByUnschedulable` only,
and a node matching multiple selectors counts under `excludedBySelector` once
(never double-counted).

#### Exclude control-plane nodes by label

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchExpressions:
          - key: node-role.kubernetes.io/control-plane
            operator: Exists
'
```

#### Exclude control-plane AND experimental nodes (OR — spec-007)

A list of selectors excludes the union of their matches. Here control-plane
nodes and experimental nodes are both excluded:

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchExpressions:
          - key: node-role.kubernetes.io/control-plane
            operator: Exists
      - matchExpressions:
          - key: node-type/experimental
            operator: Exists
'
```

#### Exclude nodes by label value

```sh
kubectl patch clustercapacity cluster-capacity --type=merge -p '
  spec:
    nodeSelectors:
      - matchLabels:
          dedicated: system
'
```

The selectors are read from the spec on every reconciliation, so a patch takes
effect on the next node event **without a restart**. Each selector is validated
independently: a structurally invalid entry (unknown operator, `In` without
values) is logged with a warning and skipped, while the remaining valid
selectors still apply — capacity tracking continues, and a corrected selector
takes effect immediately. If every selector is invalid, the controller falls
back to unschedulable-only exclusion for that cycle.

#### Migrating from the singular `nodeSelector` (spec-006 → spec-007)

The `nodeSelector` field (a single `LabelSelector`) was renamed to
`nodeSelectors` (a list). Wrap your existing selector in a list — the field is
optional and defaults to no selectors, so no data-migration webhook is needed:

```sh
# Before (spec-006): spec.nodeSelector: { matchExpressions: [...] }
# After  (spec-007): spec.nodeSelectors: [ { matchExpressions: [...] } ]
kubectl patch clustercapacity cluster-capacity --type=json -p '
  [{"op": "move", "from": "/spec/nodeSelector", "path": "/spec/nodeSelectors/0"}]
'
```

#### Remove all selectors (revert to unschedulable-only)

```sh
kubectl patch clustercapacity cluster-capacity --type=json -p '
  [{"op": "remove", "path": "/spec/nodeSelectors"}]
'
```

#### Inspect the exclusion breakdown

```sh
kubectl get clustercapacity cluster-capacity -o jsonpath='{.status}'
# {"totalAllocatableCpuMilli":16000,...,"nodeCount":2,
#  "excludedNodeCount":2,"excludedByUnschedulable":0,"excludedBySelector":2,...}
```

### Adjusting the Budget at Runtime

The budget lives in the `Allocation` CRD `spec.budgetPercent`, which the webhook
reads from its in-process cache. Patching it takes effect on subsequent admission
decisions **without a restart** (FR-009):

```sh
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"budgetPercent":70}}'
```

The Allocation Controller recomputes the per-resource ceilings (`floor(total ×
budgetPercent / 100)`) within its reconcile window and the webhook picks up the
new ceilings on the next decision.

### Enforcement Modes (Enforce / Dry-Run)

The webhook has two enforcement modes, toggled by the optional
`spec.enforcementMode` field on the Allocation singleton (spec-004). Like
`budgetPercent`, the mode is read from the webhook's in-process cache, so a spec
patch takes effect on subsequent decisions **without a restart** (FR-002).

| Value | Behaviour |
|-------|-----------|
| `enforce` *(default)* | Over-budget pods are **rejected** (`allowed: false`, HTTP 403). This is the fail-closed budget guardian. |
| `dry-run` | Over-budget pods are **admitted** (`allowed: true`) carrying the would-be rejection as an admission **warning**, so the webhook can be installed in an audit / shadow configuration. Within-budget pods are admitted normally; fail-closed paths still reject (see below). |

Absent or unrecognised values resolve to `enforce` (FR-003). The auto-created
singleton seeds `enforcementMode: enforce` (FR-010).

**Fail-closed paths reject in both modes** (Constitution Principle I,
NON-NEGOTIABLE). Dry-run converts **only** over-budget denials — it never
converts an error rejection. When capacity data is stale or missing, the request
is malformed, a quantity cannot be parsed, or the decision times out or panics,
the webhook rejects regardless of the mode (see [Failure Modes](#failure-modes)).

Switch the mode at runtime with `kubectl patch`:

```sh
# Enter dry-run (admit over-budget pods with a warning).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"enforcementMode":"dry-run"}}'

# Confirm it took effect.
kubectl get allocation cluster-allocation -o jsonpath='{.spec.enforcementMode}'
# → dry-run

# Return to enforce (reject over-budget pods).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"enforcementMode":"enforce"}}'
```

In dry-run mode an over-budget `kubectl run` reports a `Warning` (the
would-be rejection message, prefixed `Budget violations (dry-run):`) while the
pod is still created. A dry-run decision is logged as `decision=dry_run_deny`
and counted under the `verdict="dry_run_deny"` metric series (see
[Structured Logging](#structured-logging) and
[Prometheus Metrics](#prometheus-metrics)). Validation scenarios for both modes
are in [`specs/004-dry-run-mode/quickstart.md`](./specs/004-dry-run-mode/quickstart.md).

### Workload Exclusion

The Allocation singleton also carries two **optional exclusion lists** (spec-008):
`spec.excludedNamespaces` and `spec.excludedPriorityClasses`. A pod matching
**either** list (OR semantics) is admitted **without a budget check** — an
explicit, operator-configured bypass of the capacity gate, for workloads that
must never be gated (system components, the webhook's own namespace, etc.).

- **Check order** (first match wins, subsequent checks skipped): the webhook's
  own namespace (`capacity-admission`, FR-007) → `excludedNamespaces` →
  `excludedPriorityClasses`.
- **Priority class is a string match** on `pod.spec.priorityClassName`. The
  webhook does **not** resolve `PriorityClass` resources or their preemption
  values — it only compares the name. An absent or empty-string
  `priorityClassName` never matches.
- **Excluded pods are still counted.** The Allocation Controller is unchanged:
  excluded pods still contribute to `allocatedCpuMilli` /
  `allocatedMemoryBytes`. Exclusion is an admission-gate bypass, **not** an
  accounting exclusion — the excluded consumption stays visible in the gauges.
- **Backward compatible.** Both fields are optional; an absent or empty list
  exempts nothing, so a pre-spec-008 Allocation behaves exactly as before.
- **Fail-closed paths still reject.** The exemption check runs only **after**
  the Allocation singleton and its status are found. Missing allocation, missing
  status, stale data, timeout, and panic all reject before the exemption check
  — exclusion never weakens a fail-closed path. An exempt decision is an
  **explicit allow**, not a fail-open path (see [Failure Modes](#failure-modes)).

Patch the lists at runtime — they take effect on the next decision **without a
restart** (read from the webhook's in-process cache, like `budgetPercent`):

```sh
# Exclude a namespace (e.g. monitoring stack, never budget-gated).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedNamespaces":["monitoring"]}}'

# Exclude a priority class (e.g. critical system pods).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedPriorityClasses":["system-node-critical"]}}'

# Set both at once (OR semantics — either match exempts).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"excludedNamespaces":["kube-system"],"excludedPriorityClasses":["system-node-critical"]}}'

# Remove all exclusions (revert to budget-gating everything except the webhook's own ns).
kubectl patch allocation cluster-allocation --type=json \
  -p '[{"op":"remove","path":"/spec/excludedNamespaces"},{"op":"remove","path":"/spec/excludedPriorityClasses"}]'
```

An exempt decision is logged as `decision=exempt` with `exemption_reason` set to
`namespace`, `priority_class`, or `webhook_namespace`, and counted under
`capacity_admission_exemptions_total{reason="..."}` — **not** the verdicts
counter (see [Structured Logging](#structured-logging) and
[Prometheus Metrics](#prometheus-metrics)).

> **Cold start / self-admission.** Before the webhook's Allocation cache is
> populated it cannot read the CRD exclusion lists, so the
> `ValidatingWebhookConfiguration` keeps a `namespaceSelector` that skips the
> webhook's **own** namespace as apiserver-level defence-in-depth (FR-009). Once
> the Allocation is cached, the webhook also self-exempts its own namespace at
> runtime (FR-007). All other namespace/priority-class exclusions are CRD-based
> and operator-configured.

### Budget Edge Cases

- **`budgetPercent: 0`** is a **circuit-breaker**: the ceiling is `0` for both
  resources, so every pod requesting more than zero CPU or memory is rejected.
- **`budgetPercent: 100`** guards against **physical overcommit**: the ceiling
  equals total allocatable, so only requests that would exceed the cluster's
  actual physical capacity are denied.
- The ceiling is **inclusive**: `projected == ceiling` is admitted;
  `projected == ceiling + 1` is denied. See [Rejection Messages](#rejection-messages).

These are documented behaviours, not bugs. (Edge cases per
[`specs/001-capacity-admission-webhook/spec.md`](./specs/001-capacity-admission-webhook/spec.md).)

## Metrics & Observability

### HTTP Endpoints

The webhook serves three endpoints across two ports (source:
[`src/main.rs`](./src/main.rs), [`src/webhook/handler.rs`](./src/webhook/handler.rs),
[`deploy/deployment.yaml`](./deploy/deployment.yaml)):

| Endpoint | Protocol | Port | Path | Purpose |
|----------|----------|------|------|---------|
| Admission webhook | HTTPS (TLS) | 8443 | `/validate` | AdmissionReview processing (referenced by the ValidatingWebhookConfiguration) |
| Metrics | HTTP (plaintext) | 9090 | `/metrics` | Prometheus scrape |
| Health | HTTP (plaintext) | 9090 | `/healthz` | kubelet liveness/readiness probe |

The `/metrics` and `/healthz` endpoints share plaintext HTTP port 9090 so
Prometheus and the kubelet can reach them without TLS; the admission endpoint is
on a separate HTTPS port (8443).

> **Metrics port exposure**: port 9090 is plaintext HTTP with no authentication.
> Do not expose it externally without an additional network policy or auth layer
> — anyone who can reach it can read cluster capacity figures.

### Prometheus Metrics

All metrics are registered on a single registry and prefixed
`capacity_admission_` (source: [`src/metrics.rs`](./src/metrics.rs)). Every series
is pre-created at startup, so a scrape sees the full surface at zero before the
first decision. Label vocabularies: `resource ∈ {cpu, memory}`,
`verdict ∈ {allow, deny, dry_run_deny, error}`,
`reason ∈ {namespace, priority_class, webhook_namespace}` (spec-008).

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `capacity_admission_verdicts_total` | counter | `resource`, `verdict` | Admission decisions by resource and verdict (allow/deny/dry_run_deny/error). `dry_run_deny` is a dry-run mode would-be-rejection (spec-004); query `verdict=~"deny\|dry_run_deny"` for the combined view. Budget decisions only — an exempt decision does **not** increment this counter (see `capacity_admission_exemptions_total`). |
| `capacity_admission_exemptions_total` | counter | `reason` | Admissions bypassing the budget via the exclusion policy (spec-008). `reason ∈ {namespace, priority_class, webhook_namespace}`. An exempt decision increments this counter and **not** `capacity_admission_verdicts_total`, keeping the verdict counter semantically budget-only. |
| `capacity_admission_decision_duration_seconds` | histogram | — | Admission decision latency (seconds). Buckets: 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 1.0 |
| `capacity_admission_capacity_freshness_seconds` | gauge | — | Seconds since the Allocation CRD status was last refreshed |
| `capacity_admission_allocation_ratio` | gauge | `resource` | Allocated / ceiling ratio per resource (0.0–1.0+) |
| `capacity_admission_total_allocatable` | gauge | `resource` | Total allocatable capacity per resource |
| `capacity_admission_current_allocation` | gauge | `resource` | Currently allocated capacity per resource |
| `capacity_admission_ceiling` | gauge | `resource` | Budget ceiling per resource |

A scrape after deploying returns all eight families, each with `# HELP` and
`# TYPE` lines:

```sh
kubectl -n capacity-admission port-forward svc/capacity-admission-webhook 9090:metrics &
curl -s localhost:9090/metrics
```

### Structured Logging

The webhook uses [`tracing`](https://docs.rs/tracing) with structured fields
(target `capacity_admission`). The log level is taken from the `RUST_LOG`
environment variable and **defaults to `info`** (source: [`src/main.rs`](./src/main.rs);
the `Deployment` sets `RUST_LOG=info`).

Every admission decision emits one structured event carrying the workload
identity, the decision, and the capacity figures used. Key fields:

| Field | Meaning |
|-------|---------|
| `workload` | `<namespace>/<name>` of the triggering workload |
| `operation` | `CREATE`, `UPDATE`, `DELETE`, or `CONNECT` |
| `decision` | `allow`, `deny`, `dry_run_deny` (spec-004), `exempt` (spec-008), or `error` |
| `resource_type` | `cpu` or `memory` (one event per resource on allow/deny) |
| `allocated` / `requested` / `projected` / `ceiling` | Capacity figures for the resource |
| `budget_percent` | The active `budgetPercent` used for the decision |
| `enforcement_mode` | The active `enforcementMode` for the decision — `enforce` or `dry_run` (spec-004; present on every decision) |
| `exemption_reason` | On `exempt`: the criterion that triggered the bypass — `namespace`, `priority_class`, or `webhook_namespace` (spec-008) |
| `freshness_seconds` | Age of the cached Allocation status |
| `latency_ms` | Decision latency in milliseconds |
| `reason` | On `deny`/`dry_run_deny`: `<resource>_over_budget`; on `error`: the failure slug |

- An **allow** logs at INFO (one event per resource).
- A **deny** logs at WARN (one event per resource, `reason` names the violated resource).
- A **dry_run_deny** logs at WARN with `decision=dry_run_deny` — a dry-run mode
  would-be-rejection (the pod was admitted with a warning; spec-004).
- An **exempt** logs at INFO with `decision=exempt` and `exemption_reason` — the
  pod was admitted by the exclusion policy with no budget check (spec-008). One
  event (no per-resource figures).
- An **error** logs at ERROR with the failure `reason`.

Example (admission allowed, CPU line):

```text
2026-07-26T14:32:05.123Z  INFO capacity_admission: admission allowed \
  workload=default/api-server operation=CREATE decision=allow resource_type=cpu \
  allocated=70000 requested=5000 projected=75000 ceiling=80000 \
  budget_percent=80 freshness_seconds=2 latency_ms=3
```

### Rejection Messages

When a pod exceeds the budget, the AdmissionResponse carries a human-readable
`message` built from one violation per exceeded resource (source:
[`src/webhook/error.rs`](./src/webhook/error.rs)). Each line names the resource,
the current allocation, the requested increment, the projected total, and the
ceiling. When both CPU and RAM exceed, **both** lines are emitted, newline
separated, **CPU first**.

Format per resource:

```text
CPU budget exceeded: allocated <a>m, requested <r>m, projected <p>m, ceiling <c>m
memory budget exceeded: allocated <a> bytes, requested <r> bytes, projected <p> bytes, ceiling <c> bytes
```

Example — a pod whose projected CPU would reach 85000m against an 80000m ceiling:

```text
CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m
```

CPU is reported in milli-cores (`m`); memory in bytes. An over-budget rejection
returns HTTP `403`; a malformed request returns `400`; all other fail-closed
rejections return `500` (see [Failure Modes](#failure-modes)).

## Failure Modes

The webhook is **fail-closed**: every degradation path rejects (`allowed: false`)
rather than admitting under uncertainty (Constitution Principle I). The
`ValidatingWebhookConfiguration` uses `failurePolicy: Fail`, so the API server
itself rejects if the webhook is unreachable. There is no best-effort or
silent-admit path.

**Every fail-closed path below rejects in both enforcement modes** (FR-006 /
spec-004): dry-run converts **only** over-budget denials, never error rejections.
The one mode-sensitive row is the budget over-commit itself — see the last row.

| Condition | Outcome | Logged reason (`reason` slug) · HTTP |
|-----------|---------|--------------------------------------|
| Capacity data stale (age > freshness timeout) | Reject (both modes) | `capacity_data_stale` · 500 |
| `ClusterCapacity` or `Allocation` CRD not populated | Reject (both modes) | `capacity_data_missing` · 500 |
| Capacity state not initialised (controller cold start / empty caches) | Reject (both modes) | `capacity_data_missing` · 500 |
| Malformed / undeserialisable admission request | Reject (both modes) | `deserialisation_failure` · 400 |
| Unparseable resource quantity in the pod spec | Reject (both modes) | `quantity_parse_failure` · 400 |
| Decision timeout exceeded | Reject (both modes) | `timeout` · 500 |
| Internal panic / unknown error (catch-all) | Reject (both modes) | `internal_error` / `unknown` · 500 |
| Pod projected allocation over the budget — `enforce` | Reject | `over_budget` · 403 |
| Pod projected allocation over the budget — `dry-run` | **Admit** with a warning (`dry_run_deny` · `verdict="dry_run_deny"`) | `over_budget` (WARN) |

> **Exempt decisions are an explicit allow, not a fail-closed path** (spec-008). A
> pod admitted by the [Workload Exclusion](#workload-exclusion) policy (`decision=exempt`)
> is not a degradation outcome — it is an operator-configured bypass that runs
> only *after* the Allocation singleton and its status are found. None of the
> fail-closed rows above is weakened by exclusion config; the exemption check
> never fires on a missing allocation, missing status, stale data, timeout, or
> panic.

Source: [`src/webhook/error.rs`](./src/webhook/error.rs) (variant → message/code
mapping) and [`src/webhook/handler.rs`](./src/webhook/handler.rs) (the
panic/timeout/unknown guards). The catch-all guarantees there is no third,
undefined category: every error maps to a rejection (Constitution Principle III).

## Kubernetes Compatibility

The webhook supports an **N-2 window**: the three most recent Kubernetes releases
(Constitution Principle VII). As of the current implementation, CI tests against
**1.34, 1.35, and 1.36** (source:
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml) `e2e` matrix).

All Kubernetes APIs the webhook uses are GA/stable across the window:

- `admissionregistration.k8s.io/v1` — `ValidatingWebhookConfiguration`
- `apiextensions.k8s.io/v1` — `CustomResourceDefinition`
- core `v1` — `Pod`, `Node`

Deprecating support for an older release is a deliberate, documented decision,
not drift.

### Webhook Self-Admission (Bootstrap)

The webhook's own pods run in the namespace `capacity-admission`. The
`ValidatingWebhookConfiguration` carries a `namespaceSelector` that skips the
webhook's **own** namespace only (key `kubernetes.io/metadata.name`, operator
`NotIn`, value `capacity-admission`) as apiserver-level defence-in-depth, so the
webhook never blocks its own deployment — even during cold start before its
Allocation cache is populated (FR-009).

Once the Allocation cache is populated, the webhook also self-exempts its own
namespace at runtime via `check_exemption` (FR-007). All other namespace and
priority-class exclusions are **operator-configured on the Allocation CRD**
(`spec.excludedNamespaces` / `spec.excludedPriorityClasses`, spec-008) and take
effect without re-deploying the `ValidatingWebhookConfiguration` — see
[Workload Exclusion](#workload-exclusion).

## Architecture

`emergency-ration-webhook` is a 3-component operator in a single process, linked
by two cluster-scoped CRDs as shared state (Constitution Principle V). Full design
detail is in
[`specs/001-capacity-admission-webhook/data-model.md`](./specs/001-capacity-admission-webhook/data-model.md);
this is the operator-facing summary.

```text
  Node API  ──▶  Node Capacity Controller   ──▶  ClusterCapacity (status)
                  (sum of .status.allocatable)     cluster-capacity  [supply]
                                                          │
                                                          ▼
  Pod API   ──▶  Allocation Controller      ◀──  Allocation (spec.budgetPercent)
                  (sum of pod requests              cluster-allocation
                   + ceiling(supply, budget))  ──▶  Allocation (status)
                                                          │  [allocated, ceiling]
                                                          ▼
  API server ──▶ Admission Webhook          ──▶  AdmissionResponse (allow/deny)
                  POST /validate (HTTPS)
                  reads Allocation status only (in-process cache, no network)
```

- **Node Capacity Controller** — watches nodes and writes their summed
  `.status.allocatable` into `ClusterCapacity` `status` (the supply side).
  Read-only on nodes; it never interrupts node lifecycle.
- **Allocation Controller** — sums pod resource requests (non-terminal pods) and
  the budget from `Allocation.spec.budgetPercent`, computes the per-resource
  ceilings, and writes the result into `Allocation` `status` (the demand side).
- **Admission Webhook** — reads `Allocation` `status` from an in-process cache
  (no network on the hot path) and admits or denies pod `CREATE`/`UPDATE` against
  the budget.

## On-Demand Verification (`erw-verify`)

`erw-verify` is a second binary in this crate — an operator-facing tool that
installs the full webhook stack against a **clean, throwaway** Kubernetes cluster,
runs an enforcement verification matrix, tears down everything it installed, and
prints a human-readable or JSON report. It is the integration-test harness for the
admission guarantee: point it at a disposable cluster and it proves the webhook
admits/denies correctly on real infrastructure. It is **not** deployed into the
cluster — only the webhook Deployment is (via the applied manifests).

> **Throwaway cluster only.** The tool actively mutates the installation — it
> patches the budget to `0`/`100`, flips enforcement mode to `dry-run`, and (in a
> later phase) kills webhook pods and deletes CRDs. A pre-flight check refuses to
> run if the `default` namespace contains any pods. Only run it against a cluster
> you are willing to throw away.

### Build

```sh
cargo build --bin erw-verify --release   # binary at target/release/erw-verify
```

The binary embeds the `deploy/*.yaml` manifests at compile time via `include_str!`,
so it applies the exact manifests from the repository — no external files at
runtime (except `.env`, read from the repo root at startup).

### Configure (`.env`)

`erw-verify` reads a `.env` file from the repo root (spec-009). Copy the template
and fill in your values — this is the single configuration contract for the full
build → push → verify pipeline:

```sh
cp .env.example .env
# then edit .env: set ERW_REGISTRY (and optionally the image name, tag, kubeconfig)
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `ERW_REGISTRY` | Yes (unless `--skip-build`) | — | Registry endpoint without a scheme (e.g. `cr.yandex/crppbh5k4v76t4ml9u8f`) |
| `ERW_IMAGE_NAME` | No | `capacity-admission-webhook` | Image name within the registry |
| `ERW_IMAGE_TAG` | No | `latest` | Image tag |
| `ERW_KUBECONFIG` | No | inferred | Path to the target kubeconfig (relative to repo root or absolute) |
| `ERW_SKIP_BUILD` | No | off | `1`/`true` skips the Docker build+push phase |

`.env` is git-ignored and never committed; `.env.example` is the committed
template. Each variable follows the precedence: CLI flag → `.env` file → ambient
env var → compiled default. See [CLI Flags](#cli-flags) for the flag overrides.

### Run the full pipeline

With `.env` configured, a single command builds the webhook image from the repo
`Dockerfile`, pushes it to the registry, substitutes the fully-qualified
reference into the Deployment it applies, runs every verification scenario, and
tears down:

```sh
./target/release/erw-verify
```

The resolved image reference is `{ERW_REGISTRY}/{ERW_IMAGE_NAME}:{ERW_IMAGE_TAG}`
(e.g. `cr.yandex/crppbh5k4v76t4ml9u8f/capacity-admission-webhook:latest`). The
build+push phase runs **before** any cluster resource is created, so a build or
push failure aborts cleanly (exit 2) with nothing to clean up.

To iterate on the test suite without rebuilding (the slow step), skip build+push
and reuse an already-pushed image:

```sh
./target/release/erw-verify --skip-build
```

### Usage

```sh
# Human-readable report (default): coloured per-scenario output + summary.
./target/release/erw-verify --kubeconfig ~/.kube/config

# Machine-readable JSON for CI / automation.
./target/release/erw-verify --kubeconfig ~/.kube/config --json > report.json
echo $?   # 0 = all passed, 1 = a scenario failed

# Leave the installation in place for debugging when a scenario fails.
./target/release/erw-verify --kubeconfig ~/.kube/config --keep-on-failure
```

### CLI Flags

| Flag | Env var (`.env`) | Default | Description |
|------|------------------|---------|-------------|
| `--registry <host>` | `ERW_REGISTRY` | — | Registry endpoint without a scheme (e.g. `cr.yandex/<id>`). Required unless `--skip-build`. |
| `--image-name <name>` | `ERW_IMAGE_NAME` | `capacity-admission-webhook` | Image name within the registry. |
| `--image-tag <tag>` | `ERW_IMAGE_TAG` | `latest` | Image tag. |
| `--skip-build` | `ERW_SKIP_BUILD` (`1`/`true`) | off | Skip the Docker build+push phase; reuse an already-pushed image. Docker is not required when set. |
| `--kubeconfig <path>` | `ERW_KUBECONFIG` (`.env`) / `KUBECONFIG` (ambient) | inferred | Path to the target kubeconfig (relative to repo root or absolute). |
| `--json` | — | off | Emit the report as machine-readable JSON instead of coloured terminal text. |
| `--keep-on-failure` | — | off | Skip teardown if a scenario fails, leaving the installation in place for debugging. Without it, teardown always runs — even on failure. |
| `--timeout-secs <N>` | `VERIFY_TIMEOUT_SECS` | `120` | Timeout (seconds) for setup readiness waits (pods Ready + capacity state populated). Must be > 0. |

For every setting, the first available source wins: **CLI flag → `.env` file →
ambient env var → compiled default** (FR-004). A missing `ERW_REGISTRY` (when not
`--skip-build`) or a missing `docker` binary fails fast with exit code 4 before any
network action.

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | All scenarios passed and teardown succeeded. |
| `1` | One or more scenarios failed (teardown still attempted unless `--keep-on-failure`). |
| `2` | Setup error: cluster unreachable, pre-flight check failed (cluster not empty), manifests failed to apply, readiness timed out, **or a build/push failure** (spec-009). Scenarios do not run. |
| `3` | Teardown partial failure: scenarios may have passed, but the cluster was not fully cleaned up — inspect manually. |
| `4` | **Configuration error** (spec-009): a required `.env`/flag value is missing (`ERW_REGISTRY`), or `docker` is not on `PATH` (and `--skip-build` is not set). |

When multiple conditions apply, the most severe wins (setup `2` > scenario `1` >
teardown `3`; `4` short-circuits before the run begins). Errors are printed to
**stderr** with an `ERROR:` prefix, independent of `--json`; the JSON report is
only emitted once the tool reaches the report phase.

### Scenario Inventory

The tool runs a fixed set of enforcement scenarios. Each prints a ✓/✗/○ block with
timing and a detail line; the report ends with a summary and the exit code.

| ID | Scenario | Asserts |
|----|----------|---------|
| S1 | within-budget pod admitted | a small pod is admitted |
| S2 | over-budget pod denied | a huge pod is rejected with HTTP 403 |
| S3 | budgetPercent 0 (circuit-breaker) | a zero budget denies every non-zero request |
| S4 | budgetPercent 100 (physical guard) | only genuine over-physical-commit is denied |
| S5 | runtime budget adjustment | a budget patch takes effect with no webhook restart |
| S6 | dry-run mode | an over-budget pod is admitted and the `dry_run_deny` counter increments |
| S7 | capacity tracking accuracy | ClusterCapacity status matches an independent node sum |
| S8 | metrics + health endpoints | `/healthz` and `/metrics` respond via the API proxy |

Three degradation scenarios (S9-S11 — kill webhook pods, delete CRD instances,
induce stale capacity data) are planned for a follow-up phase (US2); see
[`specs/005-on-demand-verification/`](./specs/005-on-demand-verification/) and the
[quickstart](./specs/005-on-demand-verification/quickstart.md) for the full design.

## Development

### Build

```sh
cargo build               # debug build
cargo build --release     # release build (what the Dockerfile / CI produce)
```

The MSRV is **1.89** (edition 2024), recorded in [`Cargo.toml`](./Cargo.toml).

### Tests

```sh
cargo test                            # unit + integration + BDD + verify (mocked apiserver)
cargo test -- --ignored               # end-to-end tests (need a live k3d/kind cluster)
```

Unit and integration tests use a `tower-test`-mocked API server; BDD scenarios
run via `cucumber-rs` under `tests/bdd/`. The `erw-verify` tool's pure modules
(report rendering, CLI arg parsing) have unit tests under `tests/verify/` that run
with no cluster. E2E tests are marked `#[ignore]` so a plain `cargo test` does not
require a cluster; `erw-verify`'s scenarios are themselves integration tests that
run against a real cluster (see [On-Demand Verification](#on-demand-verification-erw-verify)).

### Quality Gate

Before merge, all of the following must be green (the same gate CI enforces in
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml)):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`README.md` (and every other file) must also comply with
[`.editorconfig`](./.editorconfig) — enforced by CI's `editorconfig` job.

### Project Structure

```text
src/
├── main.rs              # binary entry point: wires the 3 components, binds HTTPS + HTTP servers
├── lib.rs               # crate facade (re-exports modules for tests)
├── config.rs            # CLI flag / env-var parsing and precedence
├── metrics.rs           # the 8 Prometheus metrics on one registry
├── time_util.rs         # RFC 3339 parsing / formatting
├── crd/
│   ├── allocation.rs        # Allocation CRD (spec.budgetPercent + status)
│   └── cluster_capacity.rs  # ClusterCapacity CRD (status only)
├── controllers/
│   ├── node_capacity.rs     # supply side: nodes → ClusterCapacity status
│   ├── node_filter.rs       # spec-006: node-exclusion filter (unschedulable + label selector)
│   └── allocation.rs        # demand side: pods + supply → Allocation status
├── resources/
│   └── quantity.rs          # Kubernetes resource-quantity parsing (cpu→milli, memory→bytes)
└── webhook/
    ├── handler.rs           # axum routes (/validate, /metrics, /healthz), decision orchestration, logging
    ├── admission.rs         # pure budget check (inclusive ceiling)
    └── error.rs             # fail-closed error → AdmissionResponse mapping, rejection messages
src/bin/erw-verify/          # on-demand verification tool (spec-005): separate binary
├── main.rs                  # orchestration + exit codes
├── args.rs                  # CLI flag / env-var parsing
├── client.rs                # kube::Client from kubeconfig
├── setup.rs                 # apply manifests, self-signed TLS cert (rcgen), caBundle, readiness, pre-flight
├── teardown.rs              # reverse-order deletion
├── report.rs                # pure human/JSON report rendering
└── scenarios/               # enforcement scenarios S1-S8 (degradation S9-S11, later)
deploy/                      # Kubernetes manifests (crds, rbac, deployment, webhook-config, cert-setup)
tests/                       # integration (tower-test mocked apiserver) + BDD (cucumber-rs) + verify (unit)
```

## License

Licensed under **Apache-2.0** — see [`LICENSE`](./LICENSE).
