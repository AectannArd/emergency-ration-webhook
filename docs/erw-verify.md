# On-Demand Verification (erw-verify)

[← Back to README](../README.md)

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

## Build

```sh
cargo build --bin erw-verify --release   # binary at target/release/erw-verify
```

The binary embeds the `deploy/*.yaml` manifests at compile time via `include_str!`,
so it applies the exact manifests from the repository — no external files at
runtime (except `.env`, read from the repo root at startup).

## Configure (`.env`)

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

## Run the full pipeline

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

## Usage

```sh
# Human-readable report (default): coloured per-scenario output + summary.
./target/release/erw-verify --kubeconfig ~/.kube/config

# Machine-readable JSON for CI / automation.
./target/release/erw-verify --kubeconfig ~/.kube/config --json > report.json
echo $?   # 0 = all passed, 1 = a scenario failed

# Leave the installation in place for debugging when a scenario fails.
./target/release/erw-verify --kubeconfig ~/.kube/config --keep-on-failure
```

## CLI Flags

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

## Exit Codes

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

## Scenario Inventory

The tool runs a fixed set of scenarios across three groups. Each prints a ✓/✗/○
block with timing and a detail line; the report ends with a summary and the exit
code. Enforcement and degradation scenarios always run; equalizer scenarios
require target-cluster kubeconfigs via `ERW_EQUALIZER_TARGET_KUBECONFIG_*` env
vars (skipped otherwise).

### Enforcement (S1–S9)

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
| S9 | per-resource asymmetric budgets | asymmetric `cpuBudgetPercent`/`memoryBudgetPercent` deny on memory only |

### Degradation (S10–S11)

| ID | Scenario | Asserts |
|----|----------|---------|
| S10 | degradation + restore | killing webhook pods and deleting CRD instances is recovered from, and capacity-data-missing rejection transitions back to normal admission |
| S11 | stale capacity data | stale ClusterCapacity data is rejected (fail-closed) and recovers when fresh data arrives |

See [`specs/010-s10-degradation-restore-fix/`](../specs/010-s10-degradation-restore-fix/)
for the degradation scenario design.

### Equalizer / cross-cluster (E1–E5)

Opt-in: requires `ERW_EQUALIZER_TARGET_KUBECONFIG_1`, `_2`, etc. pointing to
target-cluster kubeconfig files. Skipped (○, not ✗) when absent — standard
single-cluster runs are unaffected. See
[CONTRIBUTING.md § Cross-cluster verification](../CONTRIBUTING.md#cross-cluster-verification-e1-e5).

| ID | Scenario | Asserts |
|----|----------|---------|
| E1 | all clusters within target | the equalizer patches every cluster's `cpuBudgetPercent` to the target when all are under |
| E2 | over-cluster compensation | pushing one cluster over target freezes it and lowers the others to compensate |
| E3 | unreachable cluster handling | corrupting a kubeconfig Secret marks the cluster `Unreachable`; others remain managed |
| E4 | EqualizerConfig status shape | all per-cluster observation fields + fleet condition are populated |
| E5 | cleanup | EqualizerConfig, equalizer Deployment, and kubeconfig Secrets are removed |

See [`specs/013-multi-cluster-capacity-equalizer/`](../specs/013-multi-cluster-capacity-equalizer/)
for the equalizer scenario design.
