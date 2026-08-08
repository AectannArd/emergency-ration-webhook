# Contract: erw-verify Migration to Kustomize

## 1. Current state

`src/bin/erw-verify/setup.rs` embeds 4 raw manifest files at compile time:

```rust
const CRDS: &str = include_str!("../../../deploy/crds.yaml");
const RBAC: &str = include_str!("../../../deploy/rbac.yaml");
const DEPLOYMENT: &str = include_str!("../../../deploy/deployment.yaml");
const WEBHOOK_CONFIG: &str = include_str!("../../../deploy/webhook-config.yaml");
```

These files are being deleted. The tool must consume the Kustomize-rendered
output instead.

## 2. Target state: build.rs renders Kustomize at compile time

A `build.rs` script (research R5) at `src/bin/erw-verify/build.rs` (or
`src/bin/erw-verify/capacity-equalizer/build.rs` — TBD by implementation) runs
`kustomize build` during `cargo build` and writes the rendered YAML to
`$OUT_DIR`. The source module `include_str!`s the rendered file.

### 2.1 build.rs

```rust
use std::process::Command;
use std::path::PathBuf;

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let webhook_kustomize = format!("{manifest_dir}/../../../deploy/kustomize/webhook");

    // Try standalone kustomize, fall back to kubectl kustomize.
    let output = Command::new("kustomize")
        .args(["build", &webhook_kustomize])
        .output()
        .or_else(|_| {
            Command::new("kubectl")
                .args(["kustomize", &webhook_kustomize])
                .output()
        })
        .expect("neither kustomize nor kubectl found on PATH");

    assert!(output.status.success(),
        "kustomize build failed: {}",
        String::from_utf8_lossy(&output.stderr));

    let out_dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out_dir}/webhook-manifests.yaml"), &output.stdout)
        .expect("write rendered manifests");

    println!("cargo:rerun-if-changed={webhook_kustomize}");
}
```

### 2.2 setup.rs change

The four `include_str!` constants are replaced by:

```rust
const WEBHOOK_MANIFESTS: &str =
    include_str!(concat!(env!("OUT_DIR"), "/webhook-manifests.yaml"));
```

The `apply_manifests` function splits this multi-document YAML stream (it
already does this via `serde_yaml::Deserializer`) and applies each document via
SSA. The function signature is unchanged.

## 3. Image substitution retarget

### 3.1 Current behavior

`apply_manifests` receives an `image: Option<&str>` and replaces
`ERW_IMAGE_PLACEHOLDER` in the Deployment document with the resolved image
reference from `.env` (spec-009).

### 3.2 New behavior

The Kustomize-rendered Deployment contains `capacity-admission-webhook:latest`
(the default from `kustomization.yaml`'s `images:` directive). The substitution
logic finds the image field and replaces the full reference
(`capacity-admission-webhook:latest`) with the `.env`-resolved image.

The `image` parameter to `apply_manifests` remains `Option<&str>`:
- `Some(ref)` — replace the Kustomize-default image with the resolved reference.
- `None` (`--skip-build` with no registry) — leave the Kustomize default
  (`aectann/emergency-ration-webhook:latest`), which is now a pullable image
  (unlike the old `ERW_IMAGE_PLACEHOLDER`).

### 3.3 env_config.rs

The `.env` parsing (`ERW_IMAGE_NAME`, `ERW_IMAGE_TAG`, etc.) is unchanged — it
still resolves the image reference. Only the substitution TARGET changes (from a
placeholder token to the Kustomize default image reference).

## 4. Equalizer scenarios

The equalizer E1–E5 scenarios run in CI (`equalizer-e2e.yml` bash), NOT in the
`erw-verify` binary. `erw-verify` does not embed equalizer manifests. The CI
workflow migration is covered by contract `release-workflow.md` (R8 research) —
no `erw-verify` code change for the equalizer.

## 5. Build environment requirement

`kustomize` (or `kubectl`) MUST be on PATH at `cargo build` time. This is a
**build-time** requirement only — the compiled `erw-verify` binary is
self-contained (the rendered manifests are embedded in the binary via
`include_str!`).

Documented in CONTRIBUTING.md prerequisites:
> Building `erw-verify` requires `kustomize` (or `kubectl`) on PATH to render
> the webhook manifests at compile time.

## 6. What does NOT change

- The `apply_manifests` function signature and SSA apply mechanism.
- The TLS cert generation (rcgen), caBundle injection, readiness wait, and
  pre-flight checks.
- The scenario runner (enforcement.rs, degradation.rs).
- The `.env`-driven build+push automation (spec-009).
- The teardown logic.
- Exit codes.

## 7. Regression gate

After migration, the full S1–S11 scenario suite MUST pass against a `kind`
cluster with zero behavioral change. The acceptance criterion (SC-005) is: the
webhook deploys, reaches Ready, and enforces the budget identically to
pre-migration. This is verified by running `erw-verify` in CI
(`ci.yml` E2E job) after the migration.
