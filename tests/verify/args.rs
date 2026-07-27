//! Unit tests for the verify tool's CLI arg parsing (spec-005, T004).
//!
//! Drives `VerifyConfig::from_args_and_env` deterministically via an env-closure,
//! matching the style of `src/config.rs`'s unit tests. No global env mutation.

// See tests/verify/report.rs for the #[path] + allow(dead_code) rationale.
#[allow(dead_code)]
#[path = "../../src/bin/erw-verify/args.rs"]
mod args;

use std::path::PathBuf;

use args::{DEFAULT_TIMEOUT_SECS, VerifyConfig};

fn no_env(_: &str) -> Option<String> {
    None
}

// ---- defaults ----

#[test]
fn defaults_when_nothing_provided() {
    let cfg = VerifyConfig::from_args_and_env(&[], no_env);
    assert_eq!(cfg.kubeconfig, None);
    assert!(!cfg.json);
    assert!(!cfg.keep_on_failure);
    assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
}

// ---- --kubeconfig precedence: flag > env > None ----

#[test]
fn kubeconfig_cli_flag_resolves_to_path() {
    let args: Vec<String> = vec!["--kubeconfig".into(), "/etc/kube/config".into()];
    let cfg = VerifyConfig::from_args_and_env(&args, no_env);
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/etc/kube/config"))
    );
}

#[test]
fn kubeconfig_env_resolves_to_path() {
    let env = |name: &str| match name {
        "KUBECONFIG" => Some("/from/env/kubeconfig".into()),
        _ => None,
    };
    let cfg = VerifyConfig::from_args_and_env(&[], env);
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/from/env/kubeconfig"))
    );
}

#[test]
fn kubeconfig_cli_overrides_env() {
    let args: Vec<String> = vec!["--kubeconfig".into(), "/flag/path".into()];
    let env = |name: &str| match name {
        "KUBECONFIG" => Some("/env/path".into()),
        _ => None,
    };
    let cfg = VerifyConfig::from_args_and_env(&args, env);
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/flag/path"))
    );
}

#[test]
fn missing_kubeconfig_resolves_to_none() {
    // Neither flag nor env → None (signals fall-back to Config::infer at runtime).
    let cfg = VerifyConfig::from_args_and_env(&[], no_env);
    assert_eq!(cfg.kubeconfig, None);
}

// ---- boolean flags: present-or-absent ----

#[test]
fn json_flag_is_present_or_absent() {
    let with_flag: Vec<String> = vec!["--json".into()];
    assert!(VerifyConfig::from_args_and_env(&with_flag, no_env).json);
    assert!(!VerifyConfig::from_args_and_env(&[], no_env).json);
}

#[test]
fn keep_on_failure_flag_is_present_or_absent() {
    let with_flag: Vec<String> = vec!["--keep-on-failure".into()];
    assert!(VerifyConfig::from_args_and_env(&with_flag, no_env).keep_on_failure);
    assert!(!VerifyConfig::from_args_and_env(&[], no_env).keep_on_failure);
}

// ---- --timeout-secs precedence: flag > env > default ----

#[test]
fn timeout_cli_overrides_default() {
    let args: Vec<String> = vec!["--timeout-secs".into(), "240".into()];
    let cfg = VerifyConfig::from_args_and_env(&args, no_env);
    assert_eq!(cfg.timeout_secs, 240);
}

#[test]
fn timeout_env_overrides_default() {
    let env = |name: &str| match name {
        "VERIFY_TIMEOUT_SECS" => Some("60".into()),
        _ => None,
    };
    let cfg = VerifyConfig::from_args_and_env(&[], env);
    assert_eq!(cfg.timeout_secs, 60);
}

#[test]
fn timeout_cli_overrides_env() {
    let args: Vec<String> = vec!["--timeout-secs".into(), "300".into()];
    let env = |name: &str| match name {
        "VERIFY_TIMEOUT_SECS" => Some("60".into()),
        _ => None,
    };
    let cfg = VerifyConfig::from_args_and_env(&args, env);
    assert_eq!(cfg.timeout_secs, 300);
}

#[test]
fn invalid_timeout_falls_back_to_default() {
    // Non-numeric → default.
    let args: Vec<String> = vec!["--timeout-secs".into(), "not-a-number".into()];
    let cfg = VerifyConfig::from_args_and_env(&args, no_env);
    assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
}

#[test]
fn zero_timeout_falls_back_to_default() {
    // data-model §4: timeout MUST be > 0; zero is invalid → default.
    let args: Vec<String> = vec!["--timeout-secs".into(), "0".into()];
    let cfg = VerifyConfig::from_args_and_env(&args, no_env);
    assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
}

#[test]
fn flags_mixed_with_unrelated_args() {
    // Unrecognised positional/flags must not break parsing of known flags.
    let args: Vec<String> = vec![
        "extra".into(),
        "--json".into(),
        "--kubeconfig".into(),
        "/k".into(),
    ];
    let cfg = VerifyConfig::from_args_and_env(&args, no_env);
    assert!(cfg.json);
    assert_eq!(cfg.kubeconfig, Some(PathBuf::from("/k")));
}
