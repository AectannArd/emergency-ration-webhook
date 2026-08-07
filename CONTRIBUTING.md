# Contributing to emergency-ration-webhook

This guide covers everything a contributor needs to work on the repository:
setting up a development environment, building the webhook and verification
tool, running the test suite, and submitting changes.

For operator-facing documentation (deployment, configuration, admission
behaviour, metrics), see the [`README.md`](./README.md).

## Development Environment

### Prerequisites

- **Rust toolchain** — MSRV **1.89** (edition 2024), recorded in
  [`Cargo.toml`](./Cargo.toml).
- **Docker** — for building the container image (not required for unit tests).
- **kubectl** + a Kubernetes cluster — only for E2E tests and `erw-verify`
  runs against a real cluster.

### Clone

```sh
git clone https://github.com/AectannArd/emergency-ration-webhook.git
cd emergency-ration-webhook
```

## Building

### Webhook Binary

```sh
cargo build               # debug build
cargo build --release     # release build (what the Dockerfile / CI produce)
```

### Container Image

The [`Dockerfile`](./Dockerfile) is a multi-stage build (Rust 1.89 builder on a
distroless runtime base):

```sh
docker build -t capacity-admission-webhook:latest .
```

Push the image to a registry your cluster can reach (or load it locally for a
`kind`/`k3d` cluster), then update the `image:` field in
[`deploy/deployment.yaml`](./deploy/deployment.yaml) to point at it.

> **Air-gapped / offline clusters**: `deploy/deployment.yaml` sets
> `imagePullPolicy: IfNotPresent`, so the image must already be present in the
> cluster (in a registry or loaded locally) before the Deployment goes healthy.

For a `kind` cluster (used in CI), load the image locally:

```sh
docker build -t capacity-admission-webhook:latest .
kind load docker-image capacity-admission-webhook:latest --name <your-cluster>
```

### Verification Tool (`erw-verify`)

```sh
cargo build --bin erw-verify --release   # binary at target/release/erw-verify
```

The binary embeds the `deploy/*.yaml` manifests at compile time via `include_str!`,
so it applies the exact manifests from the repository — no external files at
runtime. The target cluster must be able to pull the webhook image
(`capacity-admission-webhook:latest` by default); for a `kind` cluster, build and
load it first (see [Container Image](#container-image) above).

For a remote registry, point `deploy/deployment.yaml` at your image and rebuild
`erw-verify` before running.

## Testing

```sh
cargo test                            # unit + integration + BDD + verify (mocked apiserver)
cargo test -- --ignored               # end-to-end tests (need a live k3d/kind cluster)
```

Unit and integration tests use a `tower-test`-mocked API server; BDD scenarios
run via `cucumber-rs` under `tests/bdd/`. The `erw-verify` tool's pure modules
(report rendering, CLI arg parsing) have unit tests under `tests/verify/` that run
with no cluster. E2E tests are marked `#[ignore]` so a plain `cargo test` does not
require a cluster; `erw-verify`'s scenarios are themselves integration tests that
run against a real cluster.

## Quality Gate

Before merge, all of the following must be green (the same gate CI enforces in
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml)):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`README.md`, `CONTRIBUTING.md`, and every other file must also comply with
[`.editorconfig`](./.editorconfig) — enforced by CI's `editorconfig` job.

## Development Workflow

This repository uses **GitHub Spec Kit** for spec-driven development, with a
deliberate two-agent split:

| Phase | Agent | Role |
|-------|-------|------|
| Constitution, Clarify, Specify, Plan, Tasks | Hermes Agent | Planning |
| Implement, Test | Claude Code | Implementation |

### Spec-driven process

1. Features are specified (`/speckit-specify`) and planned (`/speckit-plan`)
   before implementation. Implementation MUST cite the plan.
2. Every spec is implemented on a dedicated feature branch
   (`spec/<feature>`) and merged into `main` **only** via a pull request.
3. Development follows strict test-first (TDD) — Red-Green-Refactor
   (Constitution Principle VIII). Tests are written before implementation
   and watched to fail.
4. A task or feature is not complete until CI passes on the merge branch —
   all jobs, not just the Rust quality gate (Constitution Principle XI).

See [`AGENTS.md`](./AGENTS.md) for the full agent-split workflow.

### Code Style

- Mechanical formatting (indent, line endings, final newline, trailing
  whitespace) is declared in [`.editorconfig`](./.editorconfig) and enforced
  by CI. Rust formatting follows `rustfmt` (see
  [`rustfmt.toml`](./rustfmt.toml)).
- No host paths or machine-specific paths in tracked files. The repository is
  portable across development setups.

## Project Structure

```text
src/
├── main.rs              # binary entry point: wires the 3 components, binds HTTPS + HTTP servers
├── lib.rs               # crate facade (re-exports modules for tests)
├── config.rs            # CLI flag / env-var parsing and precedence
├── metrics.rs           # the 8 Prometheus metrics on one registry
├── time_util.rs         # RFC 3339 parsing / formatting
├── crd/
│   ├── allocation.rs        # Allocation CRD (spec.budgetPercent + status)
│   └── cluster_capacity.rs  # ClusterCapacity CRD (status only)
├── controllers/
│   ├── node_capacity.rs     # supply side: nodes → ClusterCapacity status
│   ├── node_filter.rs       # spec-006: node-exclusion filter (unschedulable + label selector)
│   └── allocation.rs        # demand side: pods + supply → Allocation status
├── resources/
│   └── quantity.rs          # Kubernetes resource-quantity parsing (cpu→milli, memory→bytes)
└── webhook/
    ├── handler.rs           # axum routes (/validate, /metrics, /healthz), decision orchestration, logging
    ├── admission.rs         # pure budget check (inclusive ceiling)
    └── error.rs             # fail-closed error → AdmissionResponse mapping, rejection messages
src/bin/erw-verify/          # on-demand verification tool (spec-005): separate binary
├── main.rs                  # orchestration + exit codes
├── args.rs                  # CLI flag / env-var parsing
├── client.rs                # kube::Client from kubeconfig
├── setup.rs                 # apply manifests, self-signed TLS cert (rcgen), caBundle, readiness, pre-flight
├── teardown.rs              # reverse-order deletion
├── report.rs                # pure human/JSON report rendering
└── scenarios/               # enforcement scenarios S1-S9
deploy/                      # Kubernetes manifests (crds, rbac, deployment, webhook-config, cert-setup)
tests/                       # integration (tower-test mocked apiserver) + BDD (cucumber-rs) + verify (unit)
```

## Submitting Changes

1. Create a feature branch from `main` (e.g. `spec/<feature-name>` or
   `fix/<bug-name>`).
2. Make your changes, following the [Quality Gate](#quality-gate) above.
3. Open a pull request against `main`. CI must pass before merge.
4. Update documentation in the same PR:
   - Operator-facing changes → [`README.md`](./README.md)
   - Contributor/workflow changes → this file (`CONTRIBUTING.md`)

## Publishing / Releases

Container images are published to Docker Hub automatically — there is no manual
build-and-push step. A release is cut by pushing a semantic-version git tag:

```sh
git tag v1.0.0          # stable release
git tag v1.0.0-rc.1     # pre-release (release candidate)
git push origin v1.0.0
```

The tag push triggers [`.github/workflows/publish.yml`](./.github/workflows/publish.yml),
which re-runs the [Quality Gate](#quality-gate) (so a red gate blocks publishing),
builds the image for **linux/amd64** and **linux/arm64** from the repository's
[`Dockerfile`](./Dockerfile) (unchanged), and pushes a single multi-arch manifest
to `aectann/emergency-ration-webhook` on Docker Hub.

- **Stable tags** (`vX.Y.Z`, no hyphen) also update the moving `latest` tag.
- **Pre-release tags** (`vX.Y.Z-rc.N`, `vX.Y.Z-beta.N`) publish only the explicit
  version tag — they do **not** touch `latest`.
- **Non-semver tags** (e.g. `v1.0`, `release-1`) match neither tag filter and do
  not trigger a publish.

The workflow is gated on the canonical repository
(`AectannArd/emergency-ration-webhook`), so a tag pushed on a fork is a no-op.

### Required secrets

Publishing authenticates to Docker Hub via two repository secrets — configure
them under **Settings → Secrets and variables → Actions**:

| Secret | Value |
|--------|-------|
| `DOCKERHUB_USERNAME` | Docker Hub account with push access to `aectann/emergency-ration-webhook` |
| `DOCKERHUB_TOKEN` | A Docker Hub access token (not the account password) |

If either secret is missing, the workflow fails fast with an `::error::` naming
the missing secret. The full secret contract (token scoping, rotation,
verification) is in
[`specs/011-docker-hub-publishing/contracts/secrets.md`](./specs/011-docker-hub-publishing/contracts/secrets.md).

### Manual dispatch

A maintainer can re-publish without creating a git tag via **Actions → "Publish
to Docker Hub" → Run workflow**, optionally passing an `image_tag` input (e.g.
`nightly`) to publish under that tag. The full design (triggers, tag derivation,
caching) is in [`specs/011-docker-hub-publishing/`](./specs/011-docker-hub-publishing/).

## Multi-Cluster Capacity Equalizer (spec-013)

The equalizer is a separate binary (`capacity-equalizer`) that lives in the same
Cargo workspace. Build and test it with:

```bash
# Build the equalizer binary.
cargo build --bin capacity-equalizer

# Run the equalizer algorithm unit tests (pure function truth-table cases).
cargo test --test algorithm

# Run the equalizer reconcile integration tests (mocked multi-cluster apiserver).
cargo test --test reconcile

# Run the equalizer BDD scenarios.
cargo test --test equalizer_bdd

# Build the equalizer Docker image.
docker build -f Dockerfile.equalizer -t capacity-equalizer .
```

The full design (CRD schema, algorithm, multi-cluster client construction, RBAC)
is in [`specs/013-multi-cluster-capacity-equalizer/`](./specs/013-multi-cluster-capacity-equalizer/).
