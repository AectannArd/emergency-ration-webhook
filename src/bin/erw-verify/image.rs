//! Docker image build + push for the verify pipeline (spec-009, research R2).
//!
//! Shells out to the `docker` CLI via [`std::process::Command`] (Constitution
//! Principle V: no new crates — there is no dependency-free way to build an OCI
//! image; `bollard` is a heavy API client for what is a simple build+push).
//! [`fully_qualified_image`] is a pure, unit-tested resolver; the build/push
//! wrappers are thin Command orchestration that requires a real Docker daemon +
//! registry and is exercised by running the tool, not by unit tests.

use std::process::Command;

/// Resolve the fully-qualified image reference: `{registry}/{image}:{tag}`.
///
/// Pure — no I/O. Mirrors `contracts/env.md` §Fully-Qualified Image, e.g.
/// `cr.yandex/crppbh5k4v76t4ml9u8f/capacity-admission-webhook:latest`.
pub fn fully_qualified_image(registry: &str, image: &str, tag: &str) -> String {
    format!("{registry}/{image}:{tag}")
}

/// Whether the `docker` CLI is on `PATH` (FR-009 pre-flight). Uses `docker
/// --version` so a running daemon is NOT required to pass — only the binary's
/// presence matters. A missing binary or non-zero exit returns `false`.
pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Build the webhook image from the repo-root `Dockerfile`, tagged with the
/// fully-qualified reference (research R2: `docker build -t <ref> .`). The
/// blocking invocation runs on the runtime's blocking-pool so it does not stall
/// the async reactor. Returns `Err` with captured stdout+stderr on non-zero exit.
pub async fn build_image(full_ref: &str) -> Result<(), String> {
    let full_ref = full_ref.to_string();
    tokio::task::spawn_blocking(move || run_docker(&["build", "-t", &full_ref, "."]))
        .await
        .map_err(|e| format!("docker build task failed: {e}"))?
}

/// Push the built image to the configured registry (research R2:
/// `docker push <ref>`). Returns `Err` with captured stdout+stderr on non-zero
/// exit (e.g. an authentication failure — research R2 error handling).
pub async fn push_image(full_ref: &str) -> Result<(), String> {
    let full_ref = full_ref.to_string();
    tokio::task::spawn_blocking(move || run_docker(&["push", &full_ref]))
        .await
        .map_err(|e| format!("docker push task failed: {e}"))?
}

/// Invoke `docker <args>`, capturing output. Returns `Err` with stdout+stderr on
/// a non-zero exit, or if the `docker` binary cannot be spawned at all.
fn run_docker(args: &[&str]) -> Result<(), String> {
    let command = args.join(" ");
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("failed to invoke docker {command}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "docker {command} exited with status {}.\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    ))
}

#[cfg(test)]
mod tests {
    use super::fully_qualified_image;

    #[test]
    fn resolves_registry_image_tag() {
        assert_eq!(
            fully_qualified_image(
                "cr.yandex/crppbh5k4v76t4ml9u8f",
                "capacity-admission-webhook",
                "latest",
            ),
            "cr.yandex/crppbh5k4v76t4ml9u8f/capacity-admission-webhook:latest",
        );
    }

    #[test]
    fn resolves_custom_registry_and_tag() {
        assert_eq!(
            fully_qualified_image("ghcr.io/acme", "webhook", "v1.2.3"),
            "ghcr.io/acme/webhook:v1.2.3",
        );
    }

    #[test]
    fn resolves_empty_tag_segment() {
        // The resolver is a pure format — it does not validate ref grammar.
        assert_eq!(
            fully_qualified_image("registry.example.com", "img", ""),
            "registry.example.com/img:",
        );
    }
}
