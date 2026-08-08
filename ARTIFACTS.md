# Build Artifacts

This document enumerates every binary the repository produces and whether it is
published as a Docker image. It is the single source of truth for the
artifact-to-image mapping; when a new `[[bin]]` is added to `Cargo.toml`, this
file MUST be updated in the same change (Constitution Principle XIV).

## Artifact inventory

| Binary | Source | Docker image | Dockerfile | Image repository |
|--------|--------|:---:|---|---|
| `capacity-admission-webhook` | `src/main.rs` | Yes | `Dockerfile` | `aectann/emergency-ration-webhook` |
| `capacity-equalizer` | `src/bin/capacity-equalizer/main.rs` | Yes | `Dockerfile.equalizer` | `aectann/emergency-ration-equalizer` |
| `erw-verify` | `src/bin/erw-verify/main.rs` | No | — | — |

## Detail

### capacity-admission-webhook — the admission webhook

The core validating admission webhook that enforces a cluster-wide capacity
budget for CPU and RAM. Deployed as a `Deployment` behind a `Service`, served
over HTTPS on port 8443, with plaintext metrics/probe on port 9090.

- **Image**: `aectann/emergency-ration-webhook`
- **Dockerfile**: [`Dockerfile`](../Dockerfile) (multi-stage, distroless runtime)
- **Publishing**: tag-triggered multi-arch build (amd64 + arm64) via
  [`.github/workflows/publish.yml`](../.github/workflows/publish.yml)
- **Deployment manifest**: [`deploy/deployment.yaml`](../deploy/deployment.yaml)

### capacity-equalizer — multi-cluster capacity equalizer

A separate controller deployed in one of N Kubernetes clusters. Reads
`EqualizerConfig` CRDs and reconciles cumulative capacity across the configured
fleet of clusters by adjusting per-resource budget percentages.

- **Image**: `aectann/emergency-ration-equalizer`
- **Dockerfile**: [`Dockerfile.equalizer`](../Dockerfile.equalizer) (multi-stage,
  distroless runtime)
- **Publishing**: tag-triggered multi-arch build (amd64 + arm64) via
  [`.github/workflows/publish.yml`](../.github/workflows/publish.yml) (matrix
  entry `capacity-equalizer`)
- **Deployment manifest**: [`deploy/equalizer/deployment.yaml`](../deploy/equalizer/deployment.yaml)

### erw-verify — on-demand infrastructure verification tool

A CLI tool (not a server) that deploys the webhook into a real cluster, runs a
suite of admission scenarios against it, and reports pass/fail. Intended to be
run by an operator or CI against a live Kubernetes cluster — not deployed as a
long-running workload.

- **Image**: none (CLI tool, run from a local build or `cargo run`)
- **Build**: `cargo build --release --bin erw-verify`
- **Usage**: see [CONTRIBUTING.md](../CONTRIBUTING.md) and the
  [verification scenarios](../docs/erw-verify.md#scenario-inventory) in the
  verification guide
