# Contract: Target-Cluster API Interaction (spec-013)

**Status**: AUTHORITATIVE — describes how the equalizer reads and writes target
clusters' CRDs via kube-rs clients constructed from kubeconfig Secrets.

---

## 1. Multi-cluster client construction

### 1.1 Reading the kubeconfig Secret

The equalizer reads the kubeconfig bytes from a Secret in the **home** cluster
(the cluster where the equalizer pod runs):

```rust
// Home-cluster client (in-cluster SA or kubeconfig).
let home_client = Client::try_default().await?;

// Read the Secret for target cluster X.
let secrets: Api<Secret> = Api::namespaced(home_client.clone(), &secret_ref.namespace);
let secret = secrets.get(&secret_ref.name).await?;
let kubeconfig_bytes = secret.data
    .get(&secret_ref.key)
    .ok_or("key not found in Secret")?
    .0.as_slice(); // Secret data values are ByteString (base64-decoded by kube-rs)
```

### 1.2 Constructing the target client

```rust
use kube::config::{KubeConfigOptions, Kubeconfig};

let kc = Kubeconfig::read_from_yaml(std::str::from_utf8(kubeconfig_bytes)?)?;
let config = Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default()).await?;
let target_client = Client::try_from(config)?;
```

**Precondition**: `rustls::crypto::ring::default_provider().install_default()`
MUST be called before any client construction (main.rs first line, per CI
failure catalog Layer 2).

**Error handling**: if the Secret is missing → `ConfigError`. If the kubeconfig
YAML is malformed → `ConfigError`. If the target API server is unreachable →
`Unreachable`. All errors are recorded in the cluster's status observation;
the reconcile continues with the remaining clusters.

---

## 2. Reading target cluster state

For each reachable target cluster, the equalizer reads TWO CRD singletons:

### 2.1 Allocation singleton (utilization)

```rust
let allocations: Api<Allocation> = Api::all(target_client.clone());
let alloc = allocations.get(CLUSTER_ALLOCATION_NAME).await?; // "cluster-allocation"
let status = alloc.status.ok_or("Allocation has no status")?;

let cpu_util = status.utilization_percent_cpu;     // f64
let mem_util = status.utilization_percent_memory;   // f64
```

### 2.2 ClusterCapacity singleton (total allocatable)

```rust
let capacities: Api<ClusterCapacity> = Api::all(target_client.clone());
let cc = capacities.get(CLUSTER_CAPACITY_NAME).await?; // "cluster-capacity"
let status = cc.status.ok_or("ClusterCapacity has no status")?;

let total_cpu_m = status.total_allocatable_cpu_milli;    // i64
let total_mem_b = status.total_allocatable_memory_bytes;  // i64
```

**Precondition**: the target cluster MUST have the emergency-ration-webhook
installed and running (Allocation + ClusterCapacity CRDs exist with populated
status). If the CRDs are absent → `Unreachable` (the GET 404s).

---

## 3. Writing target cluster budgets

The equalizer patches ONLY the per-resource override fields on the target's
Allocation spec (spec-012 fields). It uses a strategic-merge patch so only the
named fields change:

### 3.1 The patch

```rust
let patch_body = serde_json::json!({
    "spec": {
        "cpuBudgetPercent": computed_cpu_budget,
        "memoryBudgetPercent": computed_memory_budget
    }
});

let allocations: Api<Allocation> = Api::all(target_client.clone());
allocations
    .patch(CLUSTER_ALLOCATION_NAME, &PatchParams::default(), &Patch::Merge(&patch_body))
    .await?;
```

### 3.2 What is NOT patched

- `budgetPercent` — NEVER (FR-007). The strategic-merge patch above contains only
  the two override keys, so `budgetPercent` is left at whatever the operator set.
- `status` — NEVER. The target cluster's Allocation Controller owns the status.
- `enforcementMode` — NEVER. Enforcement is the target cluster's concern.

### 3.3 Patch idempotency

The patch is idempotent: if the computed budgets haven't changed since the last
cycle, patching the same values is a no-op (K8s merge-patch with identical
values produces no change). The equalizer MAY skip the patch if the computed
budgets equal the currently-observed budgets (optimization, not a requirement).

---

## 4. Required RBAC in target clusters

The kubeconfig embedded in each Secret MUST authenticate as an identity with:

```yaml
# In EACH target cluster:
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: equalizer-target
rules:
  - apiGroups: ["emergency-ration.dev"]
    resources: ["allocations"]
    verbs: ["get", "patch"]        # read status, patch spec overrides
  - apiGroups: ["emergency-ration.dev"]
    resources: ["clustercapacities"]
    verbs: ["get"]                 # read status only
```

The operator creates this ClusterRole + a ServiceAccount + RoleBinding in each
target cluster, then generates the kubeconfig for that ServiceAccount and embeds
it in a Secret in the home cluster. The deploy manifest includes an example.

---

## 5. Concurrency model

All target-cluster reads (step 2 of the reconcile loop) run concurrently via
`tokio::join_all` — each target is an independent async task. Similarly, all
patches (step 4) run concurrently. This keeps reconcile latency proportional to
the slowest target, not the sum of all targets. A per-target timeout
(configurable, default 30s) prevents a single slow target from blocking the
cycle.
