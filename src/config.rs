//! Runtime configuration parsed from CLI flags and environment variables.
//!
//! Precedence (highest first): `--flag value` on the command line → environment
//! variable → compiled default. Parsing is dependency-free (`std::env`); there
//! is intentionally no external arg-parsing crate (Constitution Principle V:
//! minimal surface).

use std::env;
use std::path::PathBuf;

/// Runtime configuration for the webhook process.
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTPS port the admission server listens on.
    pub port: u16,
    /// Path to the TLS certificate (PEM).
    pub tls_cert_file: PathBuf,
    /// Path to the TLS private key (PEM).
    pub tls_key_file: PathBuf,
    /// Per-request admission decision timeout (the webhook fails closed on
    /// elapsed time — Constitution Principle I).
    pub decision_timeout_ms: u64,
    /// Maximum age of cached capacity data before it is treated as stale.
    pub capacity_freshness_timeout_secs: u64,
    /// Namespace the webhook and its CRDs live in.
    pub namespace: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8443,
            tls_cert_file: PathBuf::from("/tls/tls.crt"),
            tls_key_file: PathBuf::from("/tls/tls.key"),
            decision_timeout_ms: 100,
            capacity_freshness_timeout_secs: 30,
            namespace: String::from("capacity-admission"),
        }
    }
}

impl Config {
    /// Load configuration from `argv` and the process environment.
    pub fn load() -> Self {
        let args: Vec<String> = env::args().skip(1).collect();
        Self::from_args_and_env(&args, |name| env::var(name).ok())
    }

    /// Pure configuration resolver: CLI flags take precedence over environment
    /// variables, which take precedence over the compiled defaults. Exposed for
    /// deterministic testing without mutating the global environment.
    pub fn from_args_and_env(args: &[String], env_var: impl Fn(&str) -> Option<String>) -> Self {
        let default = Self::default();
        Self {
            port: resolve(args, "--port", "PORT", &env_var).unwrap_or(default.port),
            tls_cert_file: resolve(args, "--tls-cert-file", "TLS_CERT_FILE", &env_var)
                .unwrap_or(default.tls_cert_file),
            tls_key_file: resolve(args, "--tls-key-file", "TLS_KEY_FILE", &env_var)
                .unwrap_or(default.tls_key_file),
            decision_timeout_ms: resolve(
                args,
                "--decision-timeout-ms",
                "DECISION_TIMEOUT_MS",
                &env_var,
            )
            .unwrap_or(default.decision_timeout_ms),
            capacity_freshness_timeout_secs: resolve(
                args,
                "--capacity-freshness-timeout-secs",
                "CAPACITY_FRESHNESS_TIMEOUT_SECS",
                &env_var,
            )
            .unwrap_or(default.capacity_freshness_timeout_secs),
            namespace: resolve(args, "--namespace", "NAMESPACE", &env_var)
                .unwrap_or(default.namespace),
        }
    }
}

/// Resolve a single value from CLI flag → environment → (absent).
///
/// Returns `None` when neither source provides a value or the value cannot be
/// parsed as `T`; the caller then falls back to the default.
fn resolve<T: std::str::FromStr>(
    args: &[String],
    flag: &str,
    env_name: &str,
    env_var: &impl Fn(&str) -> Option<String>,
) -> Option<T> {
    let raw = cli_value(args, flag).or_else(|| env_var(env_name))?;
    raw.parse().ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_when_nothing_provided() {
        let cfg = Config::from_args_and_env(&[], no_env);
        assert_eq!(cfg.port, 8443);
        assert_eq!(cfg.tls_cert_file, PathBuf::from("/tls/tls.crt"));
        assert_eq!(cfg.tls_key_file, PathBuf::from("/tls/tls.key"));
        assert_eq!(cfg.decision_timeout_ms, 100);
        assert_eq!(cfg.capacity_freshness_timeout_secs, 30);
        assert_eq!(cfg.namespace, "capacity-admission");
    }

    #[test]
    fn cli_flag_overrides_default() {
        let args: Vec<String> = vec!["--port".into(), "9443".into()];
        let cfg = Config::from_args_and_env(&args, no_env);
        assert_eq!(cfg.port, 9443);
    }

    #[test]
    fn env_var_overrides_default() {
        let env = |name: &str| match name {
            "NAMESPACE" => Some("custom-ns".into()),
            _ => None,
        };
        let cfg = Config::from_args_and_env(&[], env);
        assert_eq!(cfg.namespace, "custom-ns");
    }

    #[test]
    fn cli_takes_precedence_over_env() {
        let args: Vec<String> = vec!["--port".into(), "7000".into()];
        let env = |name: &str| match name {
            "PORT" => Some("9000".into()),
            _ => None,
        };
        let cfg = Config::from_args_and_env(&args, env);
        assert_eq!(cfg.port, 7000);
    }

    #[test]
    fn unparseable_value_falls_back_to_default() {
        let args: Vec<String> = vec!["--port".into(), "not-a-port".into()];
        let cfg = Config::from_args_and_env(&args, no_env);
        assert_eq!(cfg.port, 8443);
    }
}
