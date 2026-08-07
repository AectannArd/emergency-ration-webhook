# Contract: EqualizerConfig CRD (spec-013)

**Status**: AUTHORITATIVE — the implementing agent MUST satisfy this contract.

**Scope**: the new `EqualizerConfig` CRD introduced by spec-013
(`emergency-ration.dev/v1`, kind `EqualizerConfig`, cluster-scoped singleton
`fleet-equalizer`).

---

## 1. CRD identity

- **Group**: `emergency-ration.dev`
- **Version**: `v1`
- **Kind**: `EqualizerConfig`
- **Scope**: `Cluster` (not namespaced)
- **Short name**: `eqconf`
- **Singleton instance**: `fleet-equalizer` (convention; the equalizer reads this
  name by default, like the webhook reads `cluster-allocation`).

---

## 2. Spec fields (operator-configurable)

### 2.1 `cpuTargetBudgetPercent` (REQUIRED)

- **Type**: `integer`, required.
- **Range**: `minimum: 0, maximum: 100`.
- **Role**: the cumulative CPU budget target. The fleet average CPU utilization
  converges to this value. Equalized independently from memory (FR-014).
- **Serialisation**: `cpuTargetBudgetPercent` (camelCase).

### 2.2 `memoryTargetBudgetPercent` (REQUIRED)

- **Type**: `integer`, required.
- **Range**: `minimum: 0, maximum: 100`.
- **Role**: the cumulative memory budget target. Independent from CPU.
- **Serialisation**: `memoryTargetBudgetPercent` (camelCase).

### 2.3 `targets` (REQUIRED, non-empty array)

- **Type**: `array` of `TargetCluster` objects, required, minimum 1 item.
- **Role**: the list of target clusters the equalizer manages (FR-003). Every
  cluster — including the one the equalizer runs in — is specified here.

#### 2.3.1 `TargetCluster.name`

- **Type**: `string`, required.
- **Role**: human-readable cluster name. MUST be unique within `targets[]`.

#### 2.3.2 `TargetCluster.kubeconfigSecretRef` (REQUIRED)

- **Type**: `object` (`SecretRef`), required.
- **Role**: reference to the Secret containing this cluster's kubeconfig.

##### 2.3.2.1 `kubeconfigSecretRef.name`

- **Type**: `string`, required. The Secret's name.

##### 2.3.2.2 `kubeconfigSecretRef.key`

- **Type**: `string`, optional, default `"kubeconfig"`.
- **Role**: the key within the Secret whose value is the kubeconfig YAML.

##### 2.3.2.3 `kubeconfigSecretRef.namespace`

- **Type**: `string`, required. The namespace where the Secret lives.

---

## 3. Status fields (controller-computed)

### 3.1 `clusters` (array of ClusterObservation)

Per-cluster observations from the last reconcile cycle (FR-010). Each entry:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Cluster name (matches spec.targets[].name) |
| `cpuUtilizationPercent` | number (f64) | Observed CPU utilization (from Allocation.status) |
| `memoryUtilizationPercent` | number (f64) | Observed memory utilization |
| `totalAllocatableCpuMilli` | integer | Observed total allocatable CPU, milli (from ClusterCapacity.status) |
| `totalAllocatableMemoryBytes` | integer | Observed total allocatable memory, bytes |
| `computedCpuBudgetPercent` | integer | Computed CPU budget the equalizer applied (or would apply if reachable) |
| `computedMemoryBudgetPercent` | integer | Computed memory budget |
| `state` | enum: `healthy` / `over` / `unreachable` / `config-error` | Cluster state in the equalization (kebab-case serialisation) |
| `lastError` | string, optional | Last error message (present iff state is `unreachable` or `config-error`) |
| `lastObserved` | string | Timestamp (RFC 3339) of last successful observation |

### 3.2 `condition` (enum)

Overall fleet condition (FR-011):

| Value | When |
|-------|------|
| `healthy` | All clusters at or below their computed budgets; no over-cluster compensation active |
| `compensating` | At least one cluster is over target; others are compensating |
| `degraded` | One or more clusters are unreachable or in config error |

Serialised kebab-case.

### 3.3 `lastReconciled` (string)

Timestamp (RFC 3339) of the last successful reconcile cycle.

---

## 4. Controller behaviour

### 4.1 Reconcile cycle (every 10s, configurable)

1. READ the EqualizerConfig spec.
2. FOR EACH target (concurrent): read kubeconfig Secret → build client → GET
   Allocation status + ClusterCapacity status. On failure, record
   `Unreachable` or `ConfigError`.
3. COMPUTE budgets via `equalize()` per resource dimension.
4. FOR EACH reachable cluster (concurrent): PATCH `Allocation.spec` with the
   computed `cpuBudgetPercent` + `memoryBudgetPercent`.
5. WRITE EqualizerConfig.status.

### 4.2 Singleton auto-creation

The equalizer does NOT auto-create the EqualizerConfig singleton (unlike the
webhook's Allocation singleton). The operator MUST create it — the equalizer
logs a warning and idles if `fleet-equalizer` is absent.

### 4.3 What the equalizer does NOT touch

- `Allocation.spec.budgetPercent` — NEVER modified (FR-007).
- `Allocation.status` — NEVER modified (that's the target cluster's controller's job).
- `ClusterCapacity` — read-only.
- Target cluster's webhook, admission decisions, node controllers — untouched.
- The equalizer only writes `Allocation.spec.cpuBudgetPercent` /
  `memoryBudgetPercent` on reachable target clusters.

---

## 5. CRD manifest

`deploy/equalizer/crds.yaml` — generated from `EqualizerConfig::crd()`. The CRD
object is `apiextensions.k8s.io/v1`, scope `Cluster`. The schema includes the
spec fields (§2) with range constraints on the budget targets, and the status
fields (§3).
