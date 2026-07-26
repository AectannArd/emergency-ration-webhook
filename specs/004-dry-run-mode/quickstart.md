# Quickstart: Dry-Run Enforcement Mode Validation

**Feature**: spec-004 (dry-run mode) | **Date**: 2026-07-27

This guide maps each spec user story to runnable validation scenarios. It is a
validation guide, not an implementation tutorial.

---

## Prerequisites

- A working `emergency-ration-webhook` installation (see the main
  [README Quick Start](../../001-capacity-admission-webhook/))
- The webhook running with an enforced budget and populated capacity state
- `kubectl` configured against the cluster

---

## Scenario 1 — Switch to Dry-Run Mode (US1)

**Validates**: FR-001, FR-002, FR-004 — mode toggle, runtime adjustment, dry-run
admit of over-budget pod.

```sh
# 1. Switch the webhook to dry-run mode (no restart).
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"enforcementMode":"dry-run"}}'

# 2. Verify the mode was applied.
kubectl get allocation cluster-allocation -o jsonpath='{.spec.enforcementMode}'
# Expected output: dry-run

# 3. Submit a pod that exceeds the budget.
kubectl -n default run dry-run-test --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never

# 4. Expected: the pod is ADMITTED (created), but kubectl shows a Warning.
#    The warning contains the would-be rejection message.
#    Example:
#    Warning: Budget violations (dry-run): CPU budget exceeded: ...
```

**Pass criteria**: the pod reaches `Running` (or `Pending` if unschedulable),
not `Failed`. The warning message is visible in `kubectl get events`.

---

## Scenario 2 — Dry-Run Does Not Block Within-Budget Pods (US1)

**Validates**: FR-004 — within-budget pods are admitted normally in dry-run.

```sh
# Still in dry-run mode.
kubectl -n default run dry-run-ok --image=nginx \
  --requests='cpu=10m,memory=10Mi' --restart=Never

# Expected: pod admitted with no warning (the budget is not violated).
```

**Pass criteria**: the pod is created. No warning appears in `kubectl` output
or events.

---

## Scenario 3 — Fail-Closed Paths Reject in Dry-Run (US2)

**Validates**: FR-006 — fail-closed paths reject regardless of mode.

This is best validated via integration tests (it is hard to trigger stale data
on a live cluster without disrupting it). Run:

```sh
cargo test --test dry_run -- dry_run_fail_closed
```

**Pass criteria**: the test asserts that when capacity data is stale/missing,
the admission response is `allowed: false` even when `enforcementMode` is
`dry-run`.

---

## Scenario 4 — Switch Back to Enforce Mode (US1)

**Validates**: FR-002 — runtime mode switch takes effect without restart.

```sh
# 1. Switch back to enforce mode.
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"enforcementMode":"enforce"}}'

# 2. Submit the same over-budget pod.
kubectl -n default run enforce-test --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never

# 3. Expected: the pod is REJECTED.
#    Error message: CPU budget exceeded: allocated ..., requested ..., projected ..., ceiling ...
```

**Pass criteria**: `kubectl` reports `Error from server (OverBudget): ...` and
the pod is not created.

---

## Scenario 5 — Dry-Run Observability (US3)

**Validates**: FR-007, FR-008, FR-009 — dry-run decisions distinguishable in
logs and metrics.

```sh
# 1. Switch to dry-run mode.
kubectl patch allocation cluster-allocation --type=merge \
  -p '{"spec":{"enforcementMode":"dry-run"}}'

# 2. Submit an over-budget pod (generates a dry_run_deny).
kubectl -n default run obs-test --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never

# 3. Check metrics for the dry_run_deny verdict.
kubectl -n capacity-admission port-forward svc/capacity-admission-webhook 9090:metrics &
curl -s localhost:9090/metrics | grep dry_run_deny

# Expected: the verdict counter shows dry_run_deny > 0.
# capacity_admission_verdicts_total{resource="cpu",verdict="dry_run_deny"} 1
# capacity_admission_verdicts_total{resource="memory",verdict="dry_run_deny"} 1

# 4. Check structured logs for the enforcement_mode field.
kubectl -n capacity-admission logs deploy/capacity-admission-webhook | grep dry_run_deny
# Expected: a WARN-level log entry with decision=dry_run_deny enforcement_mode=dry_run
```

**Pass criteria**: the metrics endpoint exposes `dry_run_deny` series, and the
log entry carries `enforcement_mode=dry_run`.

---

## Scenario 6 — Integration / BDD Tests

**Validates**: all FRs via automated tests.

```sh
# Unit + integration tests (mocked apiserver).
cargo test --test dry_run

# BDD scenarios.
cargo test --test dry_run_bdd

# Full quality gate.
cargo fmt --check
 cargo clippy --all-targets -- -D warnings
cargo test
```

**Pass criteria**: all tests pass. The quality gate is green.

---

## Scenario 7 — Absent/Invalid Mode Defaults to Enforce (Edge Case)

**Validates**: FR-003 — absent or unrecognised enforcementMode treated as enforce.

```sh
# 1. Remove the enforcementMode field (simulate a pre-feature Allocation).
kubectl patch allocation cluster-allocation --type=json \
  -p '[{"op":"remove","path":"/spec/enforcementMode"}]'

# 2. Submit an over-budget pod.
kubectl -n default run enforce-default --image=nginx \
  --requests='cpu=999,memory=999Gi' --restart=Never

# 3. Expected: the pod is REJECTED (absent field → enforce).
```

**Pass criteria**: the pod is rejected, confirming the safe default.
