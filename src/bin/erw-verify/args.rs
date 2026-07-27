//! CLI configuration for the verify tool, parsed from flags + environment
//! (spec-005, research R14, data-model §2).
//!
//! Mirrors the hand-rolled, dependency-free style of `src/config.rs` (Constitution
//! Principle V: minimal surface — 4 flags do not justify a parsing crate).
//! Precedence (highest first): `--flag value` → environment variable → default.

use std::env;
use std::path::PathBuf;

/// Default setup-readiness timeout in seconds (matches CI's `kubectl wait --timeout=120s`).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Configuration for the verify tool, resolved from CLI flags.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Path to kubeconfig (flag > `KUBECONFIG` env > `None` → `Config::infer`).
    pub kubeconfig: Option<PathBuf>,
    /// Emit JSON instead of human-readable report.
    pub json: bool,
    /// Skip teardown if a scenario fails (for debugging).
    pub keep_on_failure: bool,
    /// Timeout for setup readiness waits (seconds).
    pub timeout_secs: u64,
}

impl VerifyConfig {
    /// Load configuration from `argv` and the process environment.
    pub fn load() -> Self {
        let args: Vec<String> = env::args().skip(1).collect();
        Self::from_args_and_env(&args, |name| env::var(name).ok())
    }

    /// Pure configuration resolver: CLI flag → environment → default. Exposed for
    /// deterministic testing without mutating the global environment.
    pub fn from_args_and_env(args: &[String], env_var: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            kubeconfig: resolve_path(args, "--kubeconfig", "KUBECONFIG", &env_var),
            json: flag_present(args, "--json"),
            keep_on_failure: flag_present(args, "--keep-on-failure"),
            timeout_secs: resolve_timeout(args, "--timeout-secs", "VERIFY_TIMEOUT_SECS", &env_var),
        }
    }
}

/// Resolve a path-typed value from CLI flag → environment → `None`.
///
/// Returns `None` when neither source provides a value; the caller (client
/// construction) then falls back to `Config::infer`.
fn resolve_path(
    args: &[String],
    flag: &str,
    env_name: &str,
    env_var: &impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    cli_value(args, flag)
        .or_else(|| env_var(env_name))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Resolve the timeout (seconds) from CLI flag → environment → default.
///
/// A value that fails to parse as `u64`, or is `0` (data-model §4: must be > 0),
/// falls back to the compiled default.
fn resolve_timeout(
    args: &[String],
    flag: &str,
    env_name: &str,
    env_var: &impl Fn(&str) -> Option<String>,
) -> u64 {
    let raw = cli_value(args, flag).or_else(|| env_var(env_name));
    match raw.and_then(|s| s.parse::<u64>().ok()) {
        Some(secs) if secs > 0 => secs,
        _ => DEFAULT_TIMEOUT_SECS,
    }
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
