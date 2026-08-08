# Configuration Reference

[← Back to README](../README.md)

## CLI Flags & Environment Variables

The webhook reads seven settings from CLI flags, environment variables, and
compiled defaults (source: [`src/config.rs`](../src/config.rs)). The
[`deploy/deployment.yaml`](../deploy/deployment.yaml) `Deployment` supplies these
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

## Precedence

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

## Allocation CRD

**Identity**: `allocations.emergency-ration.dev` (short name `alloc`), API group
`emergency-ration.dev/v1`, kind `Allocation`. Cluster-scoped singleton, convention
instance name **`cluster-allocation`**. Source:
[`src/crd/allocation.rs`](../src/crd/allocation.rs). The instance is auto-created
by the Allocation Controller with `spec.budgetPercent: 80` if absent, and an
existing instance is never overwritten (an operator-set budget is preserved).

**Spec** (the user-configurable fields):

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `budgetPercent` | integer | 0–100 | Max allocation as % of total allocatable. Applied to CPU and RAM independently. **80** is the auto-created default; change it with `kubectl patch` (see [Adjusting the Budget at Runtime](#adjusting-the-budget-at-runtime)). |
| `enforcementMode` | string enum | `enforce` \| `dry-run` | Enforcement mode (spec-004). `enforce` (default) rejects over-budget pods; `dry-run` admits them with a warning instead. Fail-closed paths reject in both modes. Absent → `enforce`. See [Enforcement Modes](./enforcement-modes.md). |
| `excludedNamespaces` | array of strings | optional | List of namespace names whose pods are exempt from capacity admission (spec-008). A pod whose namespace matches ANY entry is admitted without a budget check (OR semantics with `excludedPriorityClasses`). Absent or empty → no namespaces exempted. See [Workload Exclusion](./workload-exclusion.md). |
| `excludedPriorityClasses` | array of strings | optional | List of priority class names whose pods are exempt from capacity admission (spec-008). Matched against `pod.spec.priorityClassName` as a **string match** (no PriorityClass resource resolution). A pod matching either list is exempt. Absent or empty → no priority classes exempted. See [Workload Exclusion](./workload-exclusion.md). |
| `cpuBudgetPercent` | integer | 0–100, optional | Per-resource CPU budget override (spec-012). When present, the CPU ceiling is derived from this value instead of `budgetPercent`; when absent, CPU falls back to `budgetPercent`. Set both resources independently — see [Per-Resource Budget Overrides](#per-resource-budget-overrides-spec-012). |
| `memoryBudgetPercent` | integer | 0–100, optional | Per-resource memory budget override (spec-012). Symmetric to `cpuBudgetPercent` for RAM: when present, the memory ceiling uses this value instead of `budgetPercent`. |

**Status** (controller-computed — read-only for operators):

| Field | Type | Unit | Description |
|-------|------|------|-------------|
| `allocatedCpuMilli` | integer | milli-CPUs | Sum of pod CPU requests |
| `allocatedMemoryBytes` | integer | bytes | Sum of pod memory requests |
| `ceilingCpuMilli` | integer | milli-CPUs | `floor(totalAllocatableCpuMilli × effectiveCpuBudgetPercent / 100)` |
| `ceilingMemoryBytes` | integer | bytes | Budget ceiling for memory |
| `utilizationPercentCpu` | number | ratio 0–1+ | `allocated / ceiling` for CPU |
| `utilizationPercentMemory` | number | ratio 0–1+ | `allocated / ceiling` for memory |
| `lastUpdated` | string | RFC 3339 | Last recomputation timestamp |
| `effectiveCpuBudgetPercent` | integer | % | The effective CPU budget the controller used to compute `ceilingCpuMilli` (spec-012): `cpuBudgetPercent` if set, else `budgetPercent`. Exposed for observability — see [Per-Resource Budget Overrides](#per-resource-budget-overrides-spec-012). |
| `effectiveMemoryBudgetPercent` | integer | % | Effective memory budget (spec-012): `memoryBudgetPercent` if set, else `budgetPercent`. |

## ClusterCapacity CRD

**Identity**: `clustercapacities.emergency-ration.dev` (short name `cc`), API group
`emergency-ration.dev/v1`, kind `ClusterCapacity`. Cluster-scoped singleton,
convention instance name **`cluster-capacity`**. It is supply-side: the Node
Capacity Controller sums every node's `.status.allocatable` into its `status`,
refreshing on each node event. Source:
[`src/crd/cluster_capacity.rs`](../src/crd/cluster_capacity.rs). The instance is
created automatically and an existing one is never overwritten — so an
operator-set `nodeSelectors` list is preserved across restarts.

**Spec** (the user-configurable field):

| Field | Type | Constraint | Description |
|-------|------|------------|-------------|
| `nodeSelectors` | array (LabelSelector) | optional | List of label selectors. A node matching ANY selector is excluded from the capacity aggregate (OR semantics, spec-007). Each entry uses standard Kubernetes `LabelSelector` (`matchLabels` + `matchExpressions`). Absent or empty → only unschedulable nodes are excluded. See [Node Exclusion](./node-exclusion.md). |

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

## Adjusting the Budget at Runtime

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

> Need to tune CPU and RAM separately? See
> [Per-Resource Budget Overrides](#per-resource-budget-overrides-spec-012) for
> `cpuBudgetPercent` / `memoryBudgetPercent`.

## Per-Resource Budget Overrides (spec-012)

`budgetPercent` applies a single budget to both CPU and RAM. Two **optional** spec
fields — `cpuBudgetPercent` and `memoryBudgetPercent` — override it for their
respective resource, so you can protect one resource more tightly than the other
(admit CPU liberally while guarding memory, for example). Each resource resolves
**independently**: its override if set, else `budgetPercent` as the fallback.

- `cpuBudgetPercent: 95` + `memoryBudgetPercent: 30` → the CPU ceiling is 95% of
  total allocatable CPU and the memory ceiling is 30% of total allocatable memory.
- With both absent, behaviour is **byte-identical** to the legacy single-budget
  controller (backward compatible — a pre-spec-012 singleton is unaffected).
- `budgetPercent` stays **required**: it is the fallback for any resource without
  an override, and the only budget when neither override is set.

The Allocation Controller resolves the effective budgets and writes both the
ceilings and the effective percentages into the status on its next reconcile
(≤2 s); the webhook reads them from its in-process cache, so a patch takes effect
**without a restart**.

Set asymmetric overrides with `kubectl patch`:

```sh
# CPU liberal (95%), memory tight (30%).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"cpuBudgetPercent":95,"memoryBudgetPercent":30}}'

# Inspect the resolved ceilings + effective budgets.
kubectl get allocation cluster-allocation -o jsonpath='{.status}'
# {"ceilingCpuMilli":...,"ceilingMemoryBytes":...,
#  "effectiveCpuBudgetPercent":95,"effectiveMemoryBudgetPercent":30,...}

# Remove the overrides (revert to budgetPercent for both resources).
kubectl patch allocation cluster-allocation --type=json \
  -p '[{"op":"remove","path":"/spec/cpuBudgetPercent"},{"op":"remove","path":"/spec/memoryBudgetPercent"}]'
```

The webhook reads the effective budgets from the Allocation **status** (not by
re-resolving the spec) and emits them as `effective_cpu_budget_percent` /
`effective_memory_budget_percent` on every budget-resolved decision (see
[Structured Logging](./observability.md#structured-logging)). The `erw-verify` S9 scenario validates
the asymmetric path against a live cluster (see
[Scenario Inventory](./erw-verify.md#scenario-inventory)).

## Budget Edge Cases

- **`budgetPercent: 0`** is a **circuit-breaker**: the ceiling is `0` for both
  resources, so every pod requesting more than zero CPU or memory is rejected.
- **`budgetPercent: 100`** guards against **physical overcommit**: the ceiling
  equals total allocatable, so only requests that would exceed the cluster's
  actual physical capacity are denied.
- The ceiling is **inclusive**: `projected == ceiling` is admitted;
  `projected == ceiling + 1` is denied. See [Rejection Messages](./observability.md#rejection-messages).

These are documented behaviours, not bugs. (Edge cases per
[`specs/001-capacity-admission-webhook/spec.md`](../specs/001-capacity-admission-webhook/spec.md).)
