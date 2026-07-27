# Contract: CLI Interface (`erw-verify`)

> The on-demand verification tool's external interface — its command-line
> flags, exit codes, and report formats. This is the contract an operator or CI
> pipeline interacts with.

## Synopsis

```text
erw-verify [--kubeconfig <path>] [--json] [--keep-on-failure] [--timeout-secs <N>]
```

## Flags

| Flag | Env Var | Type | Default | Description |
|------|---------|------|---------|-------------|
| `--kubeconfig` | `KUBECONFIG` | path | `~/.kube/config` | Path to the kubeconfig for the target cluster. Precedence: flag > `KUBECONFIG` env var > default `~/.kube/config`. |
| `--json` | — | flag (boolean) | off | Emit the report as machine-readable JSON instead of human-readable terminal text. |
| `--keep-on-failure` | — | flag (boolean) | off | Skip teardown if a scenario fails, leaving the webhook installation in place for debugging. Without this flag, teardown always runs — even on failure. |
| `--timeout-secs` | `VERIFY_TIMEOUT_SECS` | u64 | `120` | Timeout (seconds) for setup readiness waits (pod Ready + capacity state populated). |

### Precedence

For `--kubeconfig` and `--timeout-secs`, the first available source wins:
1. **CLI flag**
2. **Environment variable**
3. **Compiled default**

Boolean flags (`--json`, `--keep-on-failure`) are present-or-absent; they take
no value and have no env-var equivalent.

## Exit Codes

| Code | Meaning | When |
|------|---------|------|
| `0` | Success | All scenarios passed AND teardown succeeded. |
| `1` | Scenario failure | One or more verification scenarios failed. Teardown was still attempted (unless `--keep-on-failure`). |
| `2` | Setup error | Cluster unreachable, pre-flight check failed (cluster not empty), manifests failed to apply, or readiness timeout exceeded. Scenarios are NOT run. |
| `3` | Teardown failure | Scenarios may have passed, but teardown could not fully clean up the cluster. The operator must manually inspect and clean up. |

When multiple conditions apply, the most severe wins: setup error (2) > scenario
failure (1) > teardown failure (3). A non-zero exit means the run was not fully
successful.

## Report Format — Human-Readable (default)

Output goes to stdout. The report has three sections:

### 1. Run Header

```text
emergency-ration-webhook — on-demand verification
Cluster: <cluster-url from kubeconfig>
Started: 2026-07-27T14:32:05Z
```

### 2. Scenario Results

Each scenario prints one block. PASS scenarios use a green ✓, FAIL scenarios
use a red ✗, SKIPPED scenarios use a grey ○:

```text
✓ S1  within-budget pod admitted                        [1.2s]
  pod default/erw-smoke-ok created

✗ S2  over-budget pod denied                             [0.8s]
  expected: pod rejected with HTTP 403 (over budget)
  actual:   pod was admitted (HTTP 200)
  detail:   the webhook may not have the budget enforced
```

### 3. Summary

```text
────────────────────────────────────────────────
 Results: 10 passed, 1 failed, 0 skipped (11 total)
 Duration: 4m 32s
 Exit code: 1
────────────────────────────────────────────────
```

## Report Format — JSON (`--json`)

Valid JSON object emitted to stdout (no other output):

```json
{
  "cluster": "https://10.0.0.1:6443",
  "started": "2026-07-27T14:32:05Z",
  "duration_secs": 272.4,
  "scenarios": [
    {
      "id": "S1",
      "name": "within-budget pod admitted",
      "group": "enforcement",
      "status": "pass",
      "duration_secs": 1.2,
      "detail": "pod default/erw-smoke-ok created"
    },
    {
      "id": "S2",
      "name": "over-budget pod denied",
      "group": "enforcement",
      "status": "fail",
      "duration_secs": 0.8,
      "detail": "expected: pod rejected with HTTP 403; actual: pod admitted (HTTP 200)"
    }
  ],
  "summary": {
    "total": 11,
    "passed": 10,
    "failed": 1,
    "skipped": 0
  },
  "exit_code": 1
}
```

## Scenario Inventory

The tool runs a fixed set of 11 scenarios in two phases:

| ID | Group | Name |
|----|-------|------|
| S1 | enforcement | within-budget pod admitted |
| S2 | enforcement | over-budget pod denied |
| S3 | enforcement | budgetPercent 0 (circuit-breaker) |
| S4 | enforcement | budgetPercent 100 (physical overcommit guard) |
| S5 | enforcement | runtime budget adjustment (no restart) |
| S6 | enforcement | dry-run mode (admit + warning) |
| S7 | enforcement | capacity tracking accuracy (CRD vs nodes) |
| S8 | enforcement | metrics + health endpoints respond |
| S9 | degradation | webhook pods killed → admission rejected |
| S10 | degradation | CRD instances deleted → admission rejected |
| S11 | degradation | stale capacity → admission rejected |

## Error Output

Errors (setup failures, fatal errors) are printed to **stderr** with a clear
prefix, independent of the `--json` flag:

```text
ERROR: cluster unreachable: failed to connect to https://10.0.0.1:6443
       (is the kubeconfig correct? is the cluster running?)
```

The JSON report (when `--json`) is only emitted when the tool reaches the
report phase — if setup fails before any scenario runs, the tool prints the
error to stderr and exits with code 2 without emitting JSON.
