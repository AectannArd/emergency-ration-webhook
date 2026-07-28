# Research: ERW Verify Image Build Automation

## R1: `.env` parser approach

**Decision**: hand-roll a minimal `.env` parser (no `dotenv` crate).

**Rationale**: Constitution Principle V mandates minimal surface. The `.env`
format is simple: `KEY=VALUE` lines, `#` comments, optional single/double
quotes around values. This is ~40 lines of pure Rust, fully unit-testable.
The `dotenv` crate adds a dependency for trivial parsing and also mutates the
process environment (side-effecting — harder to test).

**Alternative rejected**: `dotenv` crate — adds a dependency for a ~40-line
parser, and its `dotenv::dotenv()` mutates the global environment, which breaks
deterministic testing.

## R2: Docker build+push via `std::process::Command`

**Decision**: use `std::process::Command` to shell out to `docker build` and
`docker push`.

**Rationale**: Docker operations are external side-effects (building images,
pushing to registries). There is no Rust crate that builds Docker images
without shelling out to `docker` (buildkit-rs is experimental and adds a heavy
dependency). `std::process::Command` is std-only, captures stdout/stderr, and
returns exit codes — exactly what we need.

**Command shapes**:
- Build: `docker build -t <registry>/<image>:<tag> .` (from repo root)
- Push: `docker push <registry>/<image>:<tag>`

**Error handling**: non-zero exit code → `Err(message)` with stdout+stderr.
Missing `docker` binary → `Err("docker not found on PATH")`.

**Alternative rejected**: `bollard` crate (Docker API client) — adds a major
dependency for a simple build+push. Overkill.

## R3: Image placeholder in deployment.yaml

**Decision**: `deploy/deployment.yaml` uses a placeholder string
(`ERW_IMAGE_PLACEHOLDER`) that the tool replaces at apply time.

**Rationale**: keeps the committed manifest portable (no hardcoded registry
path in the repo). The tool already parses YAML documents into JSON values for
SSA apply; it can walk the parsed structure, find the `image:` field, and
substitute the placeholder with the resolved fully-qualified reference before
applying.

**Implementation**: after `parse_docs(DEPLOYMENT)` but before `apply_doc()`,
walk each doc's JSON tree. If `doc.kind == "Deployment"`, set
`doc.spec.template.spec.containers[0].image` to the resolved reference. This is
a targeted JSON mutation on the parsed value, not a text substitution on the
raw YAML.

**Alternative rejected**: text substitution on raw YAML (fragile — would match
`image:` anywhere in the file). Runtime JSON mutation is precise.

## R4: `.env` variable names

**Decision**: use clear, prefixed variable names:

| Variable | Purpose | Required |
|----------|---------|----------|
| `ERW_REGISTRY` | Registry endpoint (e.g. `cr.yandex/crppbh5k4v76t4ml9u8f`) | Yes (unless `--skip-build`) |
| `ERW_IMAGE_NAME` | Image name within the registry (e.g. `capacity-admission-webhook`) | Yes (unless `--skip-build`) |
| `ERW_IMAGE_TAG` | Image tag (default: `latest`) | No |
| `ERW_KUBECONFIG` | Path to kubeconfig file | No (falls back to `--kubeconfig`, `KUBECONFIG`, `Config::infer`) |
| `ERW_SKIP_BUILD` | Set to `1`/`true` to skip build+push | No |

**Rationale**: the `ERW_` prefix avoids collision with other tools. Names are
descriptive and self-documenting.

## R5: Precedence chain

**Decision**: CLI flag → `.env` file → ambient environment variable → default.

The `.env` file is loaded into an in-memory map. Resolution checks CLI args
first, then the `.env` map, then `std::env::var()`, then compiled defaults.

**Rationale**: CLI flags are the most explicit override. `.env` provides the
operational default. Ambient environment is a fallback for CI. Defaults handle
the last-mile.

## R6: `.env.example` content

**Decision**: committed `.env.example` with placeholder values and inline
comments documenting every variable.

```env
# ERW Verify — Real Infrastructure Test Configuration
# Copy this file to `.env` and fill in your values.

# Container registry endpoint (e.g. cr.yandex/crppbh5k4v76t4ml9u8f).
# Required unless --skip-build is set.
ERW_REGISTRY=

# Image name within the registry.
ERW_IMAGE_NAME=capacity-admission-webhook

# Image tag (default: latest).
ERW_IMAGE_TAG=latest

# Path to kubeconfig for the target cluster (relative to repo root or absolute).
ERW_KUBECONFIG=

# Skip Docker build+push (reuse already-pushed image). Set to 1 or true.
# ERW_SKIP_BUILD=1
```
