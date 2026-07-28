//! Unit tests for the verify tool's CLI arg parsing (spec-005, T004; spec-009
//! adds `.env` precedence + build/image/skip-build fields).
//!
//! Drives `VerifyConfig::from_args_and_env` deterministically via an env-closure
//! and an in-memory `.env` map, matching the style of `src/config.rs`'s unit
//! tests. No global env mutation.

// See tests/verify/report.rs for the #[path] + allow(dead_code) rationale.
#[allow(dead_code)]
#[path = "../../src/bin/erw-verify/args.rs"]
mod args;

use std::collections::BTreeMap;
use std::path::PathBuf;

use args::{DEFAULT_IMAGE_NAME, DEFAULT_IMAGE_TAG, DEFAULT_TIMEOUT_SECS, VerifyConfig};

fn no_env(_: &str) -> Option<String> {
    None
}

fn empty_env_file() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn env_file(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Resolve with an empty `.env` map (the pre-spec-009 behaviour).
fn resolve(args: &[String], env: impl Fn(&str) -> Option<String>) -> VerifyConfig {
    VerifyConfig::from_args_and_env(args, &empty_env_file(), env)
}

/// Resolve with a populated `.env` map.
fn resolve_with(
    args: &[String],
    env_file: &BTreeMap<String, String>,
    env: impl Fn(&str) -> Option<String>,
) -> VerifyConfig {
    VerifyConfig::from_args_and_env(args, env_file, env)
}

// ---- defaults ----

#[test]
fn defaults_when_nothing_provided() {
    let cfg = resolve(&[], no_env);
    assert_eq!(cfg.kubeconfig, None);
    assert!(!cfg.json);
    assert!(!cfg.keep_on_failure);
    assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
    // spec-009 build-config defaults.
    assert_eq!(cfg.registry, None);
    assert_eq!(cfg.image_name, DEFAULT_IMAGE_NAME);
    assert_eq!(cfg.image_tag, DEFAULT_IMAGE_TAG);
    assert!(!cfg.skip_build);
}

// ---- --kubeconfig precedence: flag > .env(ERW_KUBECONFIG) > env(KUBECONFIG) > None ----

#[test]
fn kubeconfig_cli_flag_resolves_to_path() {
    let args: Vec<String> = vec!["--kubeconfig".into(), "/etc/kube/config".into()];
    let cfg = resolve(&args, no_env);
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
    let cfg = resolve(&[], env);
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
    let cfg = resolve(&args, env);
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/flag/path"))
    );
}

#[test]
fn kubeconfig_env_file_overrides_ambient() {
    // spec-009 edge case: ERW_KUBECONFIG (.env) beats KUBECONFIG (ambient).
    let env = |name: &str| match name {
        "KUBECONFIG" => Some("/ambient/kubeconfig".into()),
        _ => None,
    };
    let cfg = resolve_with(
        &[],
        &env_file(&[("ERW_KUBECONFIG", "/dotenv/kubeconfig")]),
        env,
    );
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/dotenv/kubeconfig"))
    );
}

#[test]
fn kubeconfig_cli_overrides_env_file() {
    let args: Vec<String> = vec!["--kubeconfig".into(), "/flag/path".into()];
    let cfg = resolve_with(
        &args,
        &env_file(&[("ERW_KUBECONFIG", "/dotenv/kubeconfig")]),
        no_env,
    );
    assert_eq!(
        cfg.kubeconfig.as_deref(),
        Some(std::path::Path::new("/flag/path"))
    );
}

#[test]
fn missing_kubeconfig_resolves_to_none() {
    // Neither flag, .env, nor ambient → None (signals fall-back to Config::infer).
    let cfg = resolve(&[], no_env);
    assert_eq!(cfg.kubeconfig, None);
}

// ---- boolean flags: present-or-absent ----

#[test]
fn json_flag_is_present_or_absent() {
    let with_flag: Vec<String> = vec!["--json".into()];
    assert!(resolve(&with_flag, no_env).json);
    assert!(!resolve(&[], no_env).json);
}

#[test]
fn keep_on_failure_flag_is_present_or_absent() {
    let with_flag: Vec<String> = vec!["--keep-on-failure".into()];
    assert!(resolve(&with_flag, no_env).keep_on_failure);
    assert!(!resolve(&[], no_env).keep_on_failure);
}

// ---- --timeout-secs precedence: flag > .env > env > default ----

#[test]
fn timeout_cli_overrides_default() {
    let args: Vec<String> = vec!["--timeout-secs".into(), "240".into()];
    let cfg = resolve(&args, no_env);
    assert_eq!(cfg.timeout_secs, 240);
}

#[test]
fn timeout_env_overrides_default() {
    let env = |name: &str| match name {
        "VERIFY_TIMEOUT_SECS" => Some("60".into()),
        _ => None,
    };
    let cfg = resolve(&[], env);
    assert_eq!(cfg.timeout_secs, 60);
}

#[test]
fn timeout_cli_overrides_env() {
    let args: Vec<String> = vec!["--timeout-secs".into(), "300".into()];
    let env = |name: &str| match name {
        "VERIFY_TIMEOUT_SECS" => Some("60".into()),
        _ => None,
    };
    let cfg = resolve(&args, env);
    assert_eq!(cfg.timeout_secs, 300);
}

#[test]
fn invalid_timeout_falls_back_to_default() {
    // Non-numeric → default.
    let args: Vec<String> = vec!["--timeout-secs".into(), "not-a-number".into()];
    let cfg = resolve(&args, no_env);
    assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
}

#[test]
fn zero_timeout_falls_back_to_default() {
    // data-model §4: timeout MUST be > 0; zero is invalid → default.
    let args: Vec<String> = vec!["--timeout-secs".into(), "0".into()];
    let cfg = resolve(&args, no_env);
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
    let cfg = resolve(&args, no_env);
    assert!(cfg.json);
    assert_eq!(cfg.kubeconfig, Some(PathBuf::from("/k")));
}

// ===========================================================================
// spec-009 — registry / image-name / image-tag precedence: flag > .env > ambient
// ===========================================================================

#[test]
fn registry_defaults_to_none() {
    assert_eq!(resolve(&[], no_env).registry, None);
}

#[test]
fn registry_cli_flag_resolves() {
    let args: Vec<String> = vec!["--registry".into(), "cr.yandex/abc".into()];
    assert_eq!(
        resolve(&args, no_env).registry.as_deref(),
        Some("cr.yandex/abc")
    );
}

#[test]
fn registry_from_env_file() {
    let cfg = resolve_with(
        &[],
        &env_file(&[("ERW_REGISTRY", "cr.yandex/dotenv")]),
        no_env,
    );
    assert_eq!(cfg.registry.as_deref(), Some("cr.yandex/dotenv"));
}

#[test]
fn registry_env_file_overrides_ambient() {
    let env = |name: &str| match name {
        "ERW_REGISTRY" => Some("cr.yandex/ambient".into()),
        _ => None,
    };
    let cfg = resolve_with(&[], &env_file(&[("ERW_REGISTRY", "cr.yandex/dotenv")]), env);
    assert_eq!(cfg.registry.as_deref(), Some("cr.yandex/dotenv"));
}

#[test]
fn registry_cli_overrides_env_file_and_ambient() {
    let args: Vec<String> = vec!["--registry".into(), "cr.yandex/flag".into()];
    let env = |name: &str| match name {
        "ERW_REGISTRY" => Some("cr.yandex/ambient".into()),
        _ => None,
    };
    let cfg = resolve_with(
        &args,
        &env_file(&[("ERW_REGISTRY", "cr.yandex/dotenv")]),
        env,
    );
    assert_eq!(cfg.registry.as_deref(), Some("cr.yandex/flag"));
}

#[test]
fn registry_ambient_used_when_no_cli_or_env_file() {
    let env = |name: &str| match name {
        "ERW_REGISTRY" => Some("cr.yandex/ambient".into()),
        _ => None,
    };
    let cfg = resolve_with(&[], &empty_env_file(), env);
    assert_eq!(cfg.registry.as_deref(), Some("cr.yandex/ambient"));
}

#[test]
fn empty_registry_value_treated_as_absent() {
    // ERW_REGISTRY= (empty) falls through to None.
    let cfg = resolve_with(&[], &env_file(&[("ERW_REGISTRY", "")]), no_env);
    assert_eq!(cfg.registry, None);
}

// ---- image-name / image-tag (default when absent) ----

#[test]
fn image_name_from_env_file() {
    let cfg = resolve_with(
        &[],
        &env_file(&[("ERW_IMAGE_NAME", "custom-webhook")]),
        no_env,
    );
    assert_eq!(cfg.image_name, "custom-webhook");
}

#[test]
fn image_name_cli_overrides_env_file() {
    let args: Vec<String> = vec!["--image-name".into(), "flag-webhook".into()];
    let cfg = resolve_with(
        &args,
        &env_file(&[("ERW_IMAGE_NAME", "dotenv-webhook")]),
        no_env,
    );
    assert_eq!(cfg.image_name, "flag-webhook");
}

#[test]
fn image_tag_defaults_and_overrides() {
    assert_eq!(resolve(&[], no_env).image_tag, DEFAULT_IMAGE_TAG);
    let args: Vec<String> = vec!["--image-tag".into(), "v9".into()];
    assert_eq!(resolve(&args, no_env).image_tag, "v9");
    let cfg = resolve_with(&[], &env_file(&[("ERW_IMAGE_TAG", "dotenv-v1")]), no_env);
    assert_eq!(cfg.image_tag, "dotenv-v1");
}

// ---- --skip-build (boolean flag + truthy env) ----

#[test]
fn skip_build_defaults_false() {
    assert!(!resolve(&[], no_env).skip_build);
}

#[test]
fn skip_build_cli_flag() {
    let args: Vec<String> = vec!["--skip-build".into()];
    assert!(resolve(&args, no_env).skip_build);
}

#[test]
fn skip_build_from_env_file_truthy() {
    // 1 and true (any case) are truthy; other values are false.
    assert!(resolve_with(&[], &env_file(&[("ERW_SKIP_BUILD", "1")]), no_env).skip_build);
    assert!(resolve_with(&[], &env_file(&[("ERW_SKIP_BUILD", "TRUE")]), no_env).skip_build);
    assert!(!resolve_with(&[], &env_file(&[("ERW_SKIP_BUILD", "0")]), no_env).skip_build);
    assert!(!resolve_with(&[], &env_file(&[("ERW_SKIP_BUILD", "yes")]), no_env).skip_build);
}

#[test]
fn skip_build_cli_overrides_false_env() {
    // CLI flag wins even when .env is explicitly false-ish.
    let args: Vec<String> = vec!["--skip-build".into()];
    let cfg = resolve_with(&args, &env_file(&[("ERW_SKIP_BUILD", "0")]), no_env);
    assert!(cfg.skip_build);
}
