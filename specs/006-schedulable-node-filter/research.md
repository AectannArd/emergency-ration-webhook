# Phase 0: Research — Schedulable Node Filter

## R1 — Use `k8s-openapi` `LabelSelector` type (no custom selector)

**Decision**: Store the node-exclusion selector as
`k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector` on the
`ClusterCapacitySpec`.

**Rationale**: `k8s-openapi` 0.28.0 already ships this type with full
`Serialize`/`Deserialize`/`JsonSchema`/`DeepMerge` derives. It is the *same*
type Kubernetes uses everywhere (`Deployment.spec.selector`,
`podAffinity`, `NetworkPolicy`, etc.), so operators already know the syntax.
Using it gives us: (a) correct JSON schema for free (schemars 1.0 impl is
included), (b) correct camelCase serialization (`matchLabels` /
`matchExpressions`), (c) no custom parsing logic to maintain, and (d) the
`DeepMerge` impl for strategic merge patch compatibility.

**Alternatives considered**:
- *Custom selector struct* (e.g. a `HashMap<String, String>` of label→value):
  less expressive (no `Exists`, `NotIn`, `DoesNotExist` operators), requires
  custom schema, and operators would need to learn a new dialect. Rejected.
- *Raw `serde_json::Value`*: no type safety, no schema generation, manual
  matching logic. Rejected.
- *`kube`'s `Selector` type*: kube-rs does not ship its own LabelSelector type;
it uses `k8s-openapi`'s. This IS the kube-rs path.

## R2 — Evaluate label matching via manual comparison (not a kube-rs helper)

**Decision**: Implement label matching in `node_filter.rs` as a pure function
that iterates `matchLabels` and `matchExpressions`, comparing against
`node.metadata.labels`. No external crate.

**Rationale**: Kubernetes does not expose a standalone "does this label set
match this selector?" function in the API; the matching is done server-side by
`kubectl`/apiserver. The matching algorithm is simple and well-specified:

- `matchLabels`: every `{key, value}` pair must be present in the node's labels.
- `matchExpressions`: each requirement is evaluated independently:
  - `In`: node's label value for `key` must be in `values`
  - `NotIn`: node's label value for `key` must NOT be in `values` (or key absent)
  - `Exists`: node must have the `key` label (value irrelevant)
  - `DoesNotExist`: node must NOT have the `key` label
- All requirements (`matchLabels` AND `matchExpressions`) are ANDed. An empty
  selector matches all nodes (no exclusion). A `None` selector means "no
  selector configured" — also matches all (no exclusion).

The algorithm is ~40 lines of pure Rust, trivially unit-testable (Principle
VIII), and avoids pulling in a label-matching crate.

**Alternatives considered**:
- *`kubectl`/client-go does not expose a Rust-callable matching function*. The
  Go implementation lives in `apimachinery/pkg/labels`/`selector.go` but there
  is no equivalent in the Rust ecosystem that is widely trusted.
- *`tokio-k8s` or similar crate*: no such crate in the dependency tree;
  introducing one for a 40-line function violates Principle V (minimal surface).

## R3 — Read the selector from the ClusterCapacity CRD spec (not Allocation CRD)

**Decision**: The `LabelSelector` lives on
`ClusterCapacitySpec.nodeSelector` (an `Option<LabelSelector>`).

**Rationale**: The exclusion is a property of the *supply side* — it determines
which nodes contribute to the capacity aggregate. The `ClusterCapacity` CRD is
the supply CRD, owned by the Node Capacity Controller. Putting it on the
`Allocation` CRD (the demand side) would couple demand configuration to supply
filtering, violating Principle V (separated concerns).

The Node Capacity Controller reads `ClusterCapacitySpec.nodeSelector` on every
reconciliation cycle (it already reads the singleton's status for the
`patch_status` target). Since the CRD is cluster-scoped and the controller
watches it, spec changes propagate through the reflector cache in real-time.

**Alternatives considered**:
- *CLI flag / env var*: not runtime-adjustable without restart. Rejected — the
  project convention (established by `Allocation.budgetPercent`) is
  CRD-spec-based configuration, runtime-adjustable via `kubectl patch`.
- *A new CRD for filter config*: over-engineered for a single optional field.
  Rejected (YAGNI).

## R4 — Invalid selector fallback: log + degrade to unschedulable-only

**Decision**: If the `LabelSelector` on the CRD is structurally invalid
(e.g. `matchExpressions` with an unknown operator like `"Matches"`, or `In`
without values), the controller logs a `warn!` and falls back to
unschedulable-only exclusion (the default). Capacity tracking continues; the
filter is not silently applied with a partial match.

**Rationale**: A structurally-invalid selector is a misconfiguration, not a
runtime error. The controller must not crash (Principle I: a crashed controller
leaves stale capacity → the webhook fails closed on stale data). It must not
silently match-all (that would count excluded nodes, inflating capacity). The
safe fallback is unschedulable-only exclusion, which is always correct.

The validation is done in `node_filter.rs::validate_selector()`, which checks:
- `operator` is one of `In`, `NotIn`, `Exists`, `DoesNotExist`
- `In`/`NotIn` have non-empty `values`
- `Exists`/`DoesNotExist` have empty/absent `values`

**Alternatives considered**:
- *Reject the CRD update at admission time*: would require a validating
  admission webhook for our own CRD — out of scope (YAGNI). The CRD schema
  cannot enforce operator-value consistency via OpenAPIV3Schema alone.
- *Treat as zero-capacity (fail-closed)*: overly aggressive — the selector is
  optional configuration, not a safety-critical input. Unschedulable-only
  exclusion is the safe default that keeps capacity tracking functional.

## R5 — Status observability: excludedNodeCount + reason breakdown

**Decision**: Add three fields to `ClusterCapacityStatus`:
- `excludedNodeCount: i32` — total nodes excluded from the aggregate
- `excludedByUnschedulable: i32` — nodes excluded because `spec.unschedulable = true`
- `excludedBySelector: i32` — nodes excluded because they matched the label selector

Note: a node can be both unschedulable AND selector-matched. The breakdown
counts by *primary* reason: unschedulable is checked first (the default
exclusion), so a node that is both is counted under `excludedByUnschedulable`,
not double-counted. `excludedNodeCount = excludedByUnschedulable +
excludedBySelector`.

**Rationale**: Principle IV requires operators to understand capacity changes
without inspecting metrics. If capacity drops, the operator needs to see
*why* — is it a cordon, a selector, or both? The breakdown makes this visible
in `kubectl describe clustercapacity cluster-capacity`.

**Alternatives considered**:
- *Only total excluded count*: operators can't distinguish cordon from selector.
  Insufficient for debugging.
- *Per-node exclusion log*: noisy and not queryable. The status fields are
  summary counts, which is what operators need at a glance.
- *A separate `ExclusionBreakdown` struct*: over-structured for 3 integer
  fields. Flat fields are simpler and match the existing status style.

## R6 — No new RBAC permissions

**Decision**: No changes to `deploy/rbac.yaml`.

**Rationale**: The controller already has `get/list/watch` on `nodes` (the
watcher reads node objects including `spec.unschedulable` and `metadata.labels`).
The label selector is read from the `ClusterCapacity` CRD spec, which the
controller already has `get/list/watch` on. No new permissions are needed.

## R7 — No new dependencies

**Decision**: `Cargo.toml` `[dependencies]` is unchanged.

**Rationale**: `LabelSelector` and `LabelSelectorRequirement` are already
available via `k8s-openapi = 0.28.0` (features: `latest`, `schemars`). The
label-matching logic is pure Rust (no regex, no parsing crate). `schemars = 1`
(already a dependency) provides the `JsonSchema` trait that `LabelSelector`
implements.

## R8 — CRD schema: additive fields, backward compatible

**Decision**: The `ClusterCapacity` CRD schema changes are purely additive:
- `spec.properties` gains `nodeSelector` (optional, the `LabelSelector` schema)
- `status.properties` gains `excludedNodeCount`, `excludedByUnschedulable`,
  `excludedBySelector` (integer fields)

No existing fields are removed or renamed. Existing `ClusterCapacity` instances
without the new fields continue to work — `nodeSelector` defaults to `None`
(unschedulable-only exclusion), and the new status fields default to `0`.

**Rationale**: CRD schema evolution in Kubernetes follows the standard rule:
additive changes (new optional fields) are backward-compatible. No conversion
webhook or version bump is needed. The `v1` CRD version stays as-is.

**Alternatives considered**:
- *New CRD version `v2`*: massively over-engineered for adding optional fields.
  Rejected.
- *A separate `NodeFilter` CRD*: adds complexity for no benefit (the selector
  is a property of the supply CRD).
