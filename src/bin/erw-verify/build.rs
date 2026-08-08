//! Build script for the `erw-verify` binary (spec-015, research R5).
//!
//! Renders the webhook Kustomize bundle at compile time and writes the
//! multi-document YAML to `$OUT_DIR/webhook-manifests.yaml`, which `setup.rs`
//! embeds via `include_str!`. This keeps the Kustomize bundle
//! (`deploy/kustomize/webhook`) as the single manifest source of truth while
//! preserving `erw-verify`'s self-contained-binary property — the rendered
//! manifests are baked into the binary, so there is no runtime `kustomize`
//! dependency.
//!
//! Build-time requirement: `kustomize` (preferred) or `kubectl` (which bundles
//! `kustomize` via `kubectl kustomize`) must be on PATH. GitHub-hosted
//! ubuntu-latest runners ship both; document the requirement for local builds
//! (CONTRIBUTING.md).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Path to the webhook Kustomize bundle, relative to the package root
/// (`CARGO_MANIFEST_DIR`).
const WEBHOOK_KUSTOMIZE_DIR: &str = "deploy/kustomize/webhook";

/// Name of the rendered file written into `$OUT_DIR`.
const RENDERED_FILE: &str = "webhook-manifests.yaml";

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let kustomize_dir = manifest_dir.join(WEBHOOK_KUSTOMIZE_DIR);

    let out_dir =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR for build scripts"));

    // Prefer the standalone `kustomize` binary; fall back to `kubectl kustomize`
    // (kubectl bundles kustomize and is ubiquitous on CI runners) when it is
    // absent. If neither is available (e.g. inside a Docker build of a
    // different binary that doesn't consume these manifests), write an empty
    // placeholder and emit a warning — only `erw-verify` needs the rendered
    // manifests at compile time; the webhook and equalizer binaries ignore them.
    let output = Command::new("kustomize")
        .arg("build")
        .arg(&kustomize_dir)
        .output()
        .or_else(|_| {
            Command::new("kubectl")
                .arg("kustomize")
                .arg(&kustomize_dir)
                .output()
        });

    match output {
        Ok(output) if output.status.success() => {
            fs::write(out_dir.join(RENDERED_FILE), &output.stdout)
                .expect("write rendered webhook manifests to OUT_DIR");
        }
        Ok(output) => {
            panic!(
                "rendering the webhook Kustomize bundle failed:\n{}",
                String::from_utf8_lossy(&output.stderr),
            );
        }
        Err(_) => {
            println!(
                "cargo:warning=neither `kustomize` nor `kubectl` found on PATH; \
                 writing empty webhook-manifests.yaml placeholder. \
                 Only `erw-verify` needs the rendered manifests — \
                 install kustomize/kubectl if building erw-verify."
            );
            fs::write(out_dir.join(RENDERED_FILE), "")
                .expect("write placeholder webhook manifests to OUT_DIR");
        }
    }

    // Re-render whenever a file in the bundle changes.
    if kustomize_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&kustomize_dir)
    {
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in &paths {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
