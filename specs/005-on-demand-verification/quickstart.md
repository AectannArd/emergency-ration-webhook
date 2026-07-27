# Quickstart — On-Demand Infrastructure Verification

> Validation guide for spec-005. Maps each user story to runnable verification
> commands and expected outcomes. This is a **run guide**, not an implementation
> tutorial — the tool IS the verification.

## Prerequisites

- A **clean, throwaway** Kubernetes cluster reachable via a kubeconfig. The tool
  actively degrades the webhook installation (kills pods, deletes CRDs) — it
  must only run against a disposable cluster.
- The Rust toolchain (MSRV 1.89).
- The cluster must have at least one node (the webhook's controllers need nodes
  to compute capacity).

## Build the Tool

```sh
cargo build --bin erw-verify --release
```

The binary is at `target/release/erw-verify`.

> The tool embeds the `deploy/*.yaml` manifests at compile time via
> `include_str!`, so it applies the exact manifests from the repository — no
> external file paths needed at runtime.

## Build the Webhook Image

The tool installs the webhook from the `deploy/` manifests, which reference the
image `capacity-admission-webhook:latest`. The target cluster must be able to
pull this image. Build and load it:

```sh
docker build -t capacity-admission-webhook:latest .

# For a kind cluster:
kind load docker-image capacity-admission-webhook:latest --name <your-cluster>

# For a remote cluster: push to a registry and update deploy/deployment.yaml
# image: before building erw-verify, OR pass the image override via...
# (image override is a plan-phase decision — see Edge Case below)
```

## Run the Verification

### Basic run (human-readable report)

```sh
./target/release/erw-verify --kubeconfig ~/.kube/config
```

Expected output: a colored per-scenario report with 11 scenarios (8 enforcement
+ 3 degradation), a summary line, and exit code 0.

### JSON output (for CI / automation)

```sh
./target/release/erw-verify --kubeconfig ~/.kube/config --json > report.json
echo $?  # 0 = all passed, 1 = scenario failure
```

Expected: valid JSON on stdout (see [contracts/cli.md](./contracts/cli.md) for
the schema), with `summary.failed == 0` and `exit_code == 0`.

### Debug a failure (keep the installation)

```sh
./target/release/erw-verify --kubeconfig ~/.kube/config --keep-on-failure
# On failure, the tool skips teardown. Inspect the live state:
kubectl -n capacity-admission get pods
kubectl get allocation cluster-allocation -o yaml
kubectl get clustercapacity cluster-capacity -o yaml
# Clean up manually when done:
kubectl delete validatingwebhookconfiguration capacity-admission.emergency-ration.dev
kubectl delete namespace capacity-admission
kubectl delete crd allocations.emergency-ration.dev clustercapacities.emergency-ration.dev
```

## Validation Scenarios (mapped to user stories)

### User Story 1 — Verify Enforcement (S1–S8)

| Scenario | Expected Outcome | How to Verify |
|----------|----------------|---------------|
| S1 | Small pod admitted | Report shows ✓; cluster has no leftover test pods (teardown deletes them) |
| S2 | Over-budget pod denied | Report shows ✓; detail confirms "rejected with HTTP 403" |
| S3 | budgetPercent 0 → all pods rejected | Report shows ✓; circuit-breaker behavior confirmed |
| S4 | budgetPercent 100 → only physical overcommit denied | Report shows ✓ |
| S5 | Budget patched at runtime, new ceiling enforced without restart | Report shows ✓ |
| S6 | Dry-run mode → over-budget pod admitted + warning | Report shows ✓; detail confirms metrics counter `dry_run_deny` |
| S7 | CRD capacity status matches actual node allocatable | Report shows ✓; detail shows the computed sums match |
| S8 | /metrics and /healthz respond | Report shows ✓ |

### User Story 2 — Verify Fail-Closed Under Degradation (S9–S11)

| Scenario | Expected Outcome | How to Verify |
|----------|----------------|---------------|
| S9 | Webhook pods killed → pod submission rejected | Report shows ✓; detail confirms API server rejection (webhook unreachable) |
| S10 | CRD instances deleted → pod submission rejected | Report shows ✓; detail confirms "capacity_data_missing" rejection |
| S11 | Stale capacity data → pod submission rejected | Report shows ✓; detail confirms "capacity_data_stale" rejection |

After each degradation scenario, the tool restores the webhook to health before
the next scenario runs.

### User Story 3 — Machine-Readable Output

```sh
# Validate JSON structure
./target/release/erw-verify --kubeconfig ~/.kube/config --json | jq '.scenarios | length'
# Expected: 11

# Validate exit code semantics
./target/release/erw-verify --kubeconfig ~/.kube/config --json; echo $?
# Expected: 0 (all passed)
```

## Unit Test Validation (no cluster needed)

The tool's pure modules (report rendering, arg parsing) are unit-tested without
a cluster:

```sh
cargo test --test report    # report rendering (human + JSON), exit codes
cargo test --test args      # CLI arg parsing edge cases
```

These run as part of the normal `cargo test` quality gate and need no cluster.

## Expected Wall-Clock Time

A complete run (setup + 11 scenarios + teardown) on a single-node `kind` cluster
is expected to take 3–5 minutes. Most time is spent waiting for pod readiness
and controller reconciliation, not in the tool's own logic (SC-006 target: < 10
minutes).

## Edge Case: Remote Registry Image

If the target cluster pulls from a remote registry (not a local `kind` load),
the tool needs to patch the Deployment image before applying it — the same
`sed` step the CI workflow does. Whether this is a CLI flag (`--image
<registry/image:tag>`) or the operator pre-edits `deploy/deployment.yaml` before
building `erw-verify` is a plan-phase refinement. For v1, the operator edits
`deploy/deployment.yaml` to point at their registry, rebuilds `erw-verify`, and
the tool applies the edited manifest.
