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

    // Prefer the standalone `kustomize` binary; fall back to `kubectl kustomize`
    // (kubectl bundles kustomize and is ubiquitous on CI runners) when it is
    // absent. A missing PATH entry is `or_else`'d into the fallback; only a
    // missing binary *and* a failed fallback panic.
    let output = Command::new("kustomize")
        .arg("build")
        .arg(&kustomize_dir)
        .output()
        .or_else(|_| {
            Command::new("kubectl")
                .arg("kustomize")
                .arg(&kustomize_dir)
                .output()
        })
        .expect(
            "failed to render the webhook Kustomize bundle: neither `kustomize` nor `kubectl` \
             was found on PATH. Install one of them (kubectl bundles kustomize) and rebuild.",
        );

    assert!(
        output.status.success(),
        "rendering the webhook Kustomize bundle failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let out_dir =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR for build scripts"));
    fs::write(out_dir.join(RENDERED_FILE), &output.stdout)
        .expect("write rendered webhook manifests to OUT_DIR");

    // Re-render whenever a file in the bundle changes. `cargo:rerun-if-changed`
    // on a directory path watches only the directory's own mtime (add/remove of
    // direct children) — it does NOT catch content edits, so list every file in
    // the bundle explicitly. Sort for a deterministic directive ordering.
    let mut paths: Vec<PathBuf> = fs::read_dir(&kustomize_dir)
        .expect("read the webhook Kustomize bundle directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
