//! CLI configuration for the verify tool, parsed from flags + `.env` + environment
//! (spec-005 → spec-009, research R14/R5, data-model §1-2).
//!
//! Mirrors the hand-rolled, dependency-free style of `src/config.rs` (Constitution
//! Principle V: minimal surface — a handful of flags do not justify a parsing
//! crate). Precedence (highest first, FR-004): `--flag value` → `.env` file →
//! ambient environment variable → compiled default. The `.env` map is loaded by
//! [`crate::env_config`] and threaded in here as an immutable reference, so the
//! resolver stays a pure, deterministically-testable function.

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

/// Default setup-readiness timeout in seconds (matches CI's `kubectl wait --timeout=120s`).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Default image name within the registry (data-model §1).
pub const DEFAULT_IMAGE_NAME: &str = "capacity-admission-webhook";
/// Default image tag (data-model §1).
pub const DEFAULT_IMAGE_TAG: &str = "latest";
/// Truthy values for boolean `.env`/env vars (`ERW_SKIP_BUILD=1`/`true`).
const TRUTHY: [&str; 2] = ["1", "true"];

/// Configuration for the verify tool, resolved from CLI flags + `.env` + env.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Path to kubeconfig (`--kubeconfig` → `ERW_KUBECONFIG` → `KUBECONFIG` →
    /// `Config::infer`). Relative paths resolve against the repo root (cwd).
    pub kubeconfig: Option<PathBuf>,
    /// Emit JSON instead of human-readable report.
    pub json: bool,
    /// Skip teardown if a scenario fails (for debugging).
    pub keep_on_failure: bool,
    /// Timeout for setup readiness waits (seconds).
    pub timeout_secs: u64,
    /// Registry endpoint, e.g. `cr.yandex/<id>` (required unless `skip_build`).
    pub registry: Option<String>,
    /// Image name within the registry (default `capacity-admission-webhook`).
    pub image_name: String,
    /// Image tag (default `latest`).
    pub image_tag: String,
    /// Skip the Docker build+push phase and reuse an already-pushed image.
    pub skip_build: bool,
}

impl VerifyConfig {
    /// Load configuration from `argv`, a pre-loaded `.env` map, and the process
    /// environment, in that precedence order. The `.env` map is loaded by the
    /// caller ([`crate::main`], via [`crate::env_config`]) so this resolver stays
    /// a pure function of its arguments.
    pub fn load(env_file: &BTreeMap<String, String>) -> Self {
        let args: Vec<String> = env::args().skip(1).collect();
        Self::from_args_and_env(&args, env_file, |name| env::var(name).ok())
    }

    /// Pure configuration resolver: CLI flag → `.env` file → ambient env → default.
    /// Exposed for deterministic testing without touching disk or the global env.
    pub fn from_args_and_env(
        args: &[String],
        env_file: &BTreeMap<String, String>,
        env_var: impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            kubeconfig: resolve_kubeconfig(args, env_file, &env_var),
            json: flag_present(args, "--json"),
            keep_on_failure: flag_present(args, "--keep-on-failure"),
            timeout_secs: resolve_timeout(
                args,
                "--timeout-secs",
                "VERIFY_TIMEOUT_SECS",
                env_file,
                &env_var,
            ),
            registry: resolve_raw(args, "--registry", "ERW_REGISTRY", env_file, &env_var),
            image_name: resolve_raw(args, "--image-name", "ERW_IMAGE_NAME", env_file, &env_var)
                .unwrap_or_else(|| DEFAULT_IMAGE_NAME.to_string()),
            image_tag: resolve_raw(args, "--image-tag", "ERW_IMAGE_TAG", env_file, &env_var)
                .unwrap_or_else(|| DEFAULT_IMAGE_TAG.to_string()),
            skip_build: resolve_bool(args, "--skip-build", "ERW_SKIP_BUILD", env_file, &env_var),
        }
    }
}

/// Resolve the kubeconfig path. The `.env` key (`ERW_KUBECONFIG`) differs from the
/// ambient key (`KUBECONFIG`); the rest of the precedence chain is standard
/// (FR-004 + spec edge case: flag → `.env` → ambient → `Config::infer`).
fn resolve_kubeconfig(
    args: &[String],
    env_file: &BTreeMap<String, String>,
    env_var: &impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    cli_value(args, "--kubeconfig")
        .or_else(|| env_file.get("ERW_KUBECONFIG").cloned())
        .or_else(|| env_var("KUBECONFIG"))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Resolve a raw string value honouring precedence: CLI → `.env` → ambient. The
/// same key name is consulted in the `.env` map and the ambient environment.
/// Empty values are treated as absent so a bare `KEY=` falls through.
fn resolve_raw(
    args: &[String],
    flag: &str,
    key: &str,
    env_file: &BTreeMap<String, String>,
    env_var: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    cli_value(args, flag)
        .or_else(|| env_file.get(key).cloned())
        .or_else(|| env_var(key))
        .filter(|s| !s.is_empty())
}

/// Resolve the timeout (seconds) from CLI flag → `.env` → environment → default.
///
/// A value that fails to parse as `u64`, or is `0` (data-model §4: must be > 0),
/// falls back to the compiled default.
fn resolve_timeout(
    args: &[String],
    flag: &str,
    key: &str,
    env_file: &BTreeMap<String, String>,
    env_var: &impl Fn(&str) -> Option<String>,
) -> u64 {
    let raw = resolve_raw(args, flag, key, env_file, &env_var);
    match raw.and_then(|s| s.parse::<u64>().ok()) {
        Some(secs) if secs > 0 => secs,
        _ => DEFAULT_TIMEOUT_SECS,
    }
}

/// Resolve a boolean flag: a present CLI flag wins outright; otherwise the `.env`
/// → ambient value is truthy only when it is `1` or `true` (case-insensitive).
fn resolve_bool(
    args: &[String],
    flag: &str,
    key: &str,
    env_file: &BTreeMap<String, String>,
    env_var: &impl Fn(&str) -> Option<String>,
) -> bool {
    if flag_present(args, flag) {
        return true;
    }
    let Some(raw) = env_file.get(key).cloned().or_else(|| env_var(key)) else {
        return false;
    };
    let lower = raw.trim().to_ascii_lowercase();
    TRUTHY.contains(&lower.as_str())
}

/// Whether a boolean `--flag` token is present on the command line.
fn flag_present(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Read the value following a `--flag` token on the command line, if present.
fn cli_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
    }
    None
}
