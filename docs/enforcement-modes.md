# Enforcement Modes

[← Back to README](../README.md)

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

**Fail-closed paths reject in both modes** (Constitution Principle I). Dry-run
converts **only** over-budget denials — it never converts an error rejection.
When capacity data is stale or missing, the request is malformed, a quantity
cannot be parsed, or the decision times out or panics, the webhook rejects
regardless of the mode (see [Failure Modes](./failure-modes.md)).

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
[Structured Logging](./observability.md#structured-logging) and
[Prometheus Metrics](./observability.md#prometheus-metrics)). Validation scenarios for both modes
are in [`specs/004-dry-run-mode/quickstart.md`](../specs/004-dry-run-mode/quickstart.md).
