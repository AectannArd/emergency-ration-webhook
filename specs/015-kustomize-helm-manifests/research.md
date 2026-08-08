# Research — Kustomize + Helm Manifest Bundles (spec-015)

## R1 — Kustomize bundle structure: single base per component

**Decision**: Each component gets its own self-contained Kustomize bundle at
`deploy/kustomize/<component>/` with a flat `kustomization.yaml` listing all
resources. No separate `base/` + `overlays/` split for the in-repo bundle.

**Rationale**: The current manifests are a single deploy target per component —
there are no environment-specific variants (staging/prod) today. A base+overlays
split adds complexity with no consumer. The `images:` directive in
`kustomization.yaml` handles the image override (the only parameterized field),
and operators who need overlays can build their own pointing at this bundle as a
remote base. The user's clarification ("they should be located in
deploy/kustomize/") confirms a flat per-component structure.

**Alternatives considered**:
- `base/` + `overlays/{dev,prod}/` — rejected: no existing need for environment
  variants; YAGNI per Constitution Principle V.
- A single combined `deploy/kustomize/` for both components — rejected: the
  user specified per-component nesting (`webhook/` + `equalizer/`), and the two
  components deploy independently.

## R2 — Image override mechanism in Kustomize

**Decision**: The `kustomization.yaml` uses the `images:` directive with a
placeholder name (`capacity-admission-webhook:placeholder` for the webhook,
`capacity-equalizer:placeholder` for the equalizer). Override via:

```sh
cd deploy/kustomize/webhook
kustomize edit set image capacity-admission-webhook:placeholder=aectann/emergency-ration-webhook:v1.0.0
kustomize build | kubectl apply -f -
```

**Rationale**: The `images:` directive is the canonical Kustomize image override
mechanism. It replaces the `ERW_IMAGE_PLACEHOLDER` sed pattern currently used by
CI and `erw-verify`. The placeholder name in the base `deployment.yaml` is a
real (but non-pullable) image reference, so `kustomize build` without an overlay
produces valid YAML that simply won't pull — the override is mandatory before
apply. This mirrors how every public Kustomize bundle works (e.g.
`kustomize edit set image` in upstream examples).

**Alternatives considered**:
- Keeping `ERW_IMAGE_PLACEHOLDER` and using a `sed` or `patch` step in
  kustomization — rejected: defeats the purpose of Kustomize's native image
  override, and leaves a non-standard sed dependency.
- No image in the base (empty string) — rejected: produces invalid Deployment
  YAML (`image: ""` fails K8s validation), so Kustomize can't even render it.

## R3 — Helm chart structure: templates mirror Kustomize resources

**Decision**: Each chart's `templates/` directory contains one file per resource
kind (or small logical group), using Go templates with `.Values` references. The
chart is NOT generated from the Kustomize output — the templates are
hand-written YAML with Go template conditionals/loops, validated for parity
against the Kustomize-rendered output (US1 AC4, SC-004).

**Rationale**: Generating Helm charts FROM Kustomize (e.g. `kustomize build |
helmify`) is fragile, produces unreadable templates, and creates a build-time
dependency between the two formats. The user asked for "verified manifest sets
templated by 2 instruments" — two independent, human-authored bundles that are
cross-validated. A parity test (render both, compare) ensures they stay in sync.

**Alternatives considered**:
- `kustomize build | helmify` auto-generation — rejected: unreadable templates,
  fragile, lossy (helmify doesn't handle all K8s resource features).
- Helm chart as the single source, Kustomize generated from `helm template` —
  rejected: Kustomize is the designated source of truth (clarification), and
  generating Kustomize from Helm is an uncommon, awkward pattern.

## R4 — Helm values schema: image, namespace, budget

**Decision**: The webhook chart `values.yaml` exposes:

```yaml
image:
  repository: aectann/emergency-ration-webhook
  tag: latest
  pullPolicy: Always

namespace: capacity-admission

budget:
  defaultPercent: 80  # applied to the auto-created Allocation singleton

# Advanced: resource overrides, replicas, ports (with current defaults)
replicas: 2
```

The equalizer chart `values.yaml` exposes:

```yaml
image:
  repository: aectann/emergency-ration-equalizer
  tag: latest
  pullPolicy: Always

namespace: capacity-equalizer

reconcile:
  intervalSeconds: 10
```

**Rationale**: The image reference is the primary value an operator overrides.
Namespace is exposed for multi-environment installs. The budget default (80%)
matches the controller's auto-created Allocation singleton default. The
equalizer chart does NOT template the EqualizerConfig singleton or its kubeconfig
Secrets — those are operator-specific runtime configuration applied separately
(the chart deploys the controller, not the fleet config; same as how the raw
`deploy/equalizer/equalizer-config.example.yaml` was a separate file from
`deployment.yaml`).

**Alternatives considered**:
- Templating EqualizerConfig in the chart — rejected: the config is
  operator-specific (cluster names, kubeconfig Secret refs); templating it with
  placeholder values creates a chart that can't install cleanly without
  overriding every target. Ship the example as a commented-out template or a
  separate file in the chart dir, not a default-applied resource.

## R5 — erw-verify migration: embed rendered Kustomize output at compile time

**Decision**: `erw-verify`'s `setup.rs` is reworked to embed the
**Kustomize-rendered** manifests at compile time via a `build.rs` script that
runs `kustomize build` and writes the output to `OUT_DIR`, then `include_str!`s
the rendered file. This preserves the current compile-time embedding (no runtime
`kustomize` binary dependency) while making the source the Kustomize bundle.

**Build script approach**:

```rust
// build.rs
use std::process::Command;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let kustomize_dir = manifest_dir.join("../../deploy/kustomize/webhook");
    let output = Command::new("kustomize")
        .args(["build", kustomize_dir.to_str().unwrap()])
        .output()
        .expect("kustomize build failed — is kustomize on PATH?");
    if !output.status.success() {
        panic!("kustomize build failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("webhook-manifests.yaml"), &output.stdout)
        .expect("write rendered manifests");
    println!("cargo:rerun-if-changed={}", kustomize_dir.display());
}
```

In `setup.rs`:
```rust
const WEBHOOK_MANIFESTS: &str = include_str!(concat!(env!("OUT_DIR"), "/webhook-manifests.yaml"));
```

**Image override**: `erw-verify` already parses the Deployment YAML, finds the
image field, and replaces it before applying (spec-009 `apply_manifests`
function). Under the new approach, the image in the Kustomize base is
`capacity-admission-webhook:placeholder` (R2); `erw-verify` replaces this with
the resolved `.env`-driven image reference at apply time — same logic, different
token to find-and-replace.

**Rationale**: This is the only approach that (a) makes the Kustomize bundle the
source of truth, (b) preserves `erw-verify`'s no-runtime-dependency property
(the binary is self-contained after compile), and (c) keeps the existing
`apply_manifests` → parse → image-replace → SSA apply flow. A `build.rs`
dependency on `kustomize` is a build-time tool requirement (like `cargo`), not a
runtime one.

**Alternatives considered**:
- Runtime `kustomize build` via `std::process::Command` — rejected: adds a
  runtime binary dependency; `erw-verify` is meant to be a self-contained binary
  run against any cluster, not requiring the Kustomize binary on the operator's
  machine.
- Embedding the raw Kustomize source files and re-implementing Kustomize
  rendering in Rust — rejected: enormous, fragile, pointless.
- Keeping a separate copy of the manifests for erw-verify only — rejected by the
  user (raw files must be deleted).
- Using a Rust Kustomize library (`kustomize-api` crate) — research found no
  mature, maintained pure-Rust Kustomize renderer. The `kustomize` binary is the
  only stable implementation.

**Build-environment requirement**: `kustomize` must be on PATH at `cargo build`
time for `erw-verify`. This is documented in CONTRIBUTING.md. CI already has
`kubectl` (which bundles `kustomize` via `kubectl kustomize`); alternatively the
`install-kustomize` GitHub Action or a direct download step is added. The
`build.rs` can fall back to `kubectl kustomize` if `kustomize` is absent.

## R6 — Helm chart distribution: GitHub Release attachments

**Decision**: On tag push, the `publish.yml` workflow runs a new `charts` job
(after `quality`, parallel to or after `publish`) that:

1. Sets the chart `version` in each `Chart.yaml` to match the git tag (strip
   leading `v`).
2. Runs `helm lint` on each chart (gate — failure blocks the release).
3. Runs `helm package` to produce versioned `.tgz` files.
4. Attaches the `.tgz` files to the GitHub Release using
   `softprops/action-gh-release`.

**Rationale**: The user chose "GitHub Release attachment only — no registry."
This is the simplest distribution: no OCI registry setup, no GitHub Pages chart
repo to maintain. The `.tgz` is a standard Helm package that `helm install
./chart.tgz` consumes. The GitHub Release is auto-created by GitHub on tag push;
the workflow just attaches files.

**Alternatives considered**:
- OCI registry (GHCR) — rejected by user.
- GitHub Pages chart repo (`chart-releaser-action`) — rejected by user.
- Git-based only (no `.tgz`) — rejected: the user wants release artifacts, not
  just repo paths.

## R7 — Chart version derivation from git tag

**Decision**: The chart `version` field is set at package time by the release
workflow using `docker/metadata-action` (already in the workflow) or a simple
shell step:

```sh
VERSION="${GITHUB_REF_NAME#v}"  # vX.Y.Z → X.Y.Z
sed -i "s/^version:.*/version: ${VERSION}/" deploy/charts/webhook/Chart.yaml
sed -i "s/^version:.*/version: ${VERSION}/" deploy/charts/equalizer/Chart.yaml
```

The in-repo `Chart.yaml` keeps `version: 0.0.0-dev` (or `0.1.0-dev`) as a
placeholder; the release workflow stamps the real version. `appVersion` is set to
the same value (or kept in sync manually — it's informational).

**Rationale**: Chart version MUST match the git tag (FR-013). Stamping at package
time avoids maintaining the version in two places (Chart.yaml + git tag) during
development. The `0.0.0-dev` placeholder makes `helm lint` pass during
development without a real version.

**Alternatives considered**:
- Manually bumping Chart.yaml version on every release — rejected: error-prone,
  easy to forget, creates a commit-chase cycle.
- Using `appVersion` only and leaving `version` static — rejected: Helm requires
  `version` to be semver and unique per package; a static version means every
  release overwrites the same chart package.

## R8 — CI workflow migration: kustomize build replaces sed

**Decision**: In `ci.yml` and `equalizer-e2e.yml`, the current pattern:

```sh
sed -e 's|image: ERW_IMAGE_PLACEHOLDER|image: capacity-admission-webhook:e2e|' deploy/deployment.yaml | kubectl apply -f -
```

is replaced by:

```sh
cd deploy/kustomize/webhook
kustomize edit set image capacity-admission-webhook:placeholder=capacity-admission-webhook:e2e
kustomize build | kubectl apply -f -
cd -
```

Or equivalently using `kubectl kustomize` (no separate binary):

```sh
kubectl kustomize deploy/kustomize/webhook | sed 's|capacity-admission-webhook:placeholder|capacity-admission-webhook:e2e|' | kubectl apply -f -
```

The second form (kubectl kustomize + sed on the rendered output) is preferred for
CI because it avoids `kustomize edit` (which modifies files on disk, requiring a
git checkout reset) and works with just `kubectl` (already installed). The sed
targets the rendered output's image string, not a placeholder token in the source.

**Rationale**: CI runners have `kubectl` but may not have `kustomize` as a
standalone binary. `kubectl kustomize` is equivalent for rendering. The image
override via sed on rendered output is pragmatic for CI (ephemeral, no need to
preserve kustomization.yaml state).

**Alternatives considered**:
- `kustomize edit set image` + `kustomize build` — works but dirties the
  checkout; requires `git checkout deploy/kustomize/` after.
- A Kustomize overlay for CI (`deploy/kustomize/webhook/ci/`) — overkill for a
  single image swap.

## R9 — Kustomize parity verification

**Decision**: A CI step (in `ci.yml`) renders both the Kustomize webhook bundle
and the Helm chart, then compares the resource sets for structural parity. This
validates FR-008/US2 AC4 and US1 AC4 (Kustomize ↔ Helm equivalence).

Implementation: a shell or Python script that:
1. `kustomize build deploy/kustomize/webhook > /tmp/k-out.yaml`
2. `helm template deploy/charts/webhook > /tmp/h-out.yaml`
3. Parse both, group by `kind:metadata.name`, compare critical fields
   (apiVersion, kind, name, namespace, failurePolicy, sideEffects, RBAC verbs,
   container ports, probes).
4. Fail if any critical field differs.

**Rationale**: Two independently-authored manifest sets (Kustomize + Helm) will
drift unless continuously cross-validated. A parity test in CI catches drift at
PR time, not at operator-install time. The comparison need not be byte-identical
(Helm adds labels, changes formatting) but must be field-equivalent on the
contract-critical fields.

**Alternatives considered**:
- Manual review only — rejected: doesn't scale, easy to miss.
- Generating Helm from Kustomize — rejected (R3).

## R10 — equalizer-config.example.yaml migration

**Decision**: The `deploy/equalizer/equalizer-config.example.yaml` (containing
example kubeconfig Secrets + EqualizerConfig) is moved into the Helm chart as a
commented-out template or a `ci/` subdirectory of the chart (not applied by
default). In the Kustomize bundle, it lives alongside the base as an
`example-config.yaml` that is NOT listed in `kustomization.yaml`'s `resources:`
(it's documentation, not a deployable resource). The target-cluster RBAC comment
block from `deploy/equalizer/rbac.yaml` is preserved inside the Kustomize
`rbac.yaml` and the Helm `rbac.yaml` template comments.

**Rationale**: The example config is operator reference material, not part of the
default deployment (it contains `BASE64_KUBECONFIG_CLUSTER_A` placeholders). It
must not be applied by `kustomize build` or `helm install` by default.

## R11 — .editorconfig coverage for Helm templates

**Decision**: The `.editorconfig` already covers `*.yaml` (indent, line endings,
final newline). Helm templates (`*.tpl`, `*.yaml` under `templates/`) are YAML
with embedded Go template directives. The existing `*.yaml` section applies.
Verify `[*.{yaml,yml}]` indent is 2 spaces (Helm convention). No new section
needed unless `*.tpl` requires different handling — `*.tpl` files use the same
indent as YAML templates, so the existing YAML section covers them by extension
if we add `[*.{yaml,yml,tpl}]`.

**Rationale**: Constitution Principle IX requires every file type to have an
.editorconfig section. Helm template files are YAML-based; adding `.tpl` to the
existing YAML section is the minimal change.

## R12 — Constitution amendment: Principle XVI (Manifest Bundle Release Artifacts)

**Decision**: A new Principle XVI is added (MINOR bump, 2.9.0 → 2.10.0):

> Every containerised artifact in `ARTIFACTS.md` MUST ship two versioned manifest
> bundles — a Kustomize bundle and a Helm chart — as release artifacts. ...

**Rationale**: The user explicitly asked to "pay discreet attention to
alterations that must be made to the release procedure." The user's established
preference (Constitution Principle XV origin) is for systemic governance fixes
(documentation-as-prevention) over ad-hoc patches. Adding a principle that
mandates manifest bundles alongside Docker images prevents a future binary from
shipping with a Docker image but no Kustomize/Helm bundle — the same class of gap
Principle XV closed for publish workflows.

**Why expand via a new principle (XVI) rather than rewriting XV**: Principle XV
is specifically about Docker build+publish. Manifest bundling is a distinct
concern (templating + distribution format). A new principle keeps each
principle's scope clean and follows the project's convention of one-concern-per-
principle. The amendment is MINOR (new principle, no existing principle
redefined).

**Alternatives considered**:
- Expanding Principle XV to cover both Docker + manifests — rejected: conflates
  two concerns; Principle V (separated concerns) applies to the constitution
  itself.
- No principle, just ARTIFACTS.md expansion — rejected: doesn't prevent future
  gaps (the lesson from Principle XIV→XV).
