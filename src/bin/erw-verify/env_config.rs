//! `.env` file loading + parsing for the verify pipeline (spec-009, research R1,
//! data-model §2).
//!
//! Hand-rolled and dependency-free (Constitution Principle V: minimal surface —
//! the `dotenv` crate is a dependency for a trivial parser, and its
//! `dotenv::dotenv()` mutates the process environment, which breaks deterministic
//! testing). The pure parser ([`parse_env_file`]) is unit-tested below; the disk
//! loader ([`load_env_file`]) is a thin `std::fs` wrapper. Neither mutates the
//! process environment — the precedence chain (CLI → `.env` → ambient → default)
//! is applied in [`crate::args`].

use std::collections::BTreeMap;
use std::fs;

/// Read and parse `.env` from the current working directory (the repo root).
///
/// A missing file is **not** an error — it yields an empty map. FR-009 only fails
/// fast when a *required* variable is missing from every source (CLI, `.env`,
/// ambient). A present-but-unreadable `.env` is logged at WARN and treated as an
/// empty map, so a malformed file never crashes the tool.
pub fn load_env_file() -> BTreeMap<String, String> {
    match fs::read_to_string(".env") {
        Ok(contents) => {
            let map = parse_env_file(&contents);
            tracing::debug!(entries = map.len(), ".env loaded from repo root");
            map
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(".env not found; using CLI/ambient configuration only");
            BTreeMap::new()
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to read .env; ignoring (CLI/ambient only)");
            BTreeMap::new()
        }
    }
}

/// Parse the contents of a `.env` file into an ordered key→value map (pure).
///
/// Rules (data-model §2):
/// - Blank lines are ignored.
/// - Lines whose first non-whitespace character is `#` are comments (ignored).
/// - `KEY=VALUE` format; the first `=` splits the line (later `=` are literal).
/// - Leading/trailing whitespace around KEY and VALUE is trimmed.
/// - Values wrapped in matching single (`'…'`) or double (`"…"`) quotes have the
///   quotes stripped. Mismatched or lone quotes are kept literal. Inline comments
///   are NOT supported — a `#` inside a value is taken literally.
/// - A line without `=`, or with an empty key, is malformed and ignored.
/// - Duplicate keys: last wins (standard `.env` semantics).
pub fn parse_env_file(contents: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = unquote(value.trim());
        map.insert(key.to_string(), value);
    }
    map
}

/// Strip one layer of matching surrounding quotes from a value, if present.
/// Mismatched or lone quotes are left untouched.
fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner.to_string();
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.to_string();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::parse_env_file;
    use std::collections::BTreeMap;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // ---- empty / comments / blanks ----

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse_env_file("").is_empty());
    }

    #[test]
    fn blank_lines_and_comments_ignored() {
        let input = "\n\n# a comment\n   \n   # indented comment\n";
        assert!(parse_env_file(input).is_empty());
    }

    #[test]
    fn comment_with_leading_whitespace_treated_as_comment() {
        let m = parse_env_file("   # ERW_REGISTRY=not-parsed\nERW_REGISTRY=real\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("ERW_REGISTRY").unwrap(), "real");
    }

    // ---- basic KEY=VALUE ----

    #[test]
    fn simple_key_value() {
        let m = parse_env_file("ERW_REGISTRY=cr.yandex/abc");
        assert_eq!(m.get("ERW_REGISTRY").unwrap(), "cr.yandex/abc");
    }

    #[test]
    fn whitespace_around_key_and_value_trimmed() {
        let m = parse_env_file("  ERW_IMAGE_NAME  =  capacity-admission-webhook  ");
        assert_eq!(
            m.get("ERW_IMAGE_NAME").unwrap(),
            "capacity-admission-webhook"
        );
    }

    #[test]
    fn value_with_equals_sign_preserved() {
        // The first `=` splits; subsequent `=` are literal.
        let m = parse_env_file("KEY=a=b=c");
        assert_eq!(m.get("KEY").unwrap(), "a=b=c");
    }

    #[test]
    fn empty_value_is_empty_string() {
        let m = parse_env_file("ERW_KUBECONFIG=");
        assert_eq!(m.get("ERW_KUBECONFIG").unwrap(), "");
    }

    // ---- quotes ----

    #[test]
    fn double_quoted_value_strips_quotes_and_keeps_spaces() {
        let m = parse_env_file("ERW_REGISTRY=\"cr.yandex/abc def\"");
        assert_eq!(m.get("ERW_REGISTRY").unwrap(), "cr.yandex/abc def");
    }

    #[test]
    fn single_quoted_value_strips_quotes() {
        let m = parse_env_file("ERW_IMAGE_NAME='capacity-admission-webhook'");
        assert_eq!(
            m.get("ERW_IMAGE_NAME").unwrap(),
            "capacity-admission-webhook"
        );
    }

    #[test]
    fn quotes_with_surrounding_whitespace_trimmed_then_stripped() {
        let m = parse_env_file("ERW_TAG =  \"v1.0\"  ");
        assert_eq!(m.get("ERW_TAG").unwrap(), "v1.0");
    }

    #[test]
    fn unmatched_double_quote_kept_literal() {
        // No trailing matching quote → the leading quote stays.
        let m = parse_env_file("KEY=\"unterminated");
        assert_eq!(m.get("KEY").unwrap(), "\"unterminated");
    }

    #[test]
    fn mismatched_quotes_kept_literal() {
        let m = parse_env_file("KEY=\"value'");
        assert_eq!(m.get("KEY").unwrap(), "\"value'");
    }

    #[test]
    fn quoted_value_with_internal_hash_kept_literal() {
        // Inline comments are NOT supported; the hash is part of the value.
        let m = parse_env_file("KEY=\"value # not a comment\"");
        assert_eq!(m.get("KEY").unwrap(), "value # not a comment");
    }

    // ---- malformed lines ----

    #[test]
    fn line_without_equals_ignored() {
        let m = parse_env_file("NOSEPARATOR\nKEY=val");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("KEY").unwrap(), "val");
    }

    #[test]
    fn empty_key_ignored() {
        // "=value" has no key → ignored.
        let m = parse_env_file("=value\nKEY=val");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("KEY").unwrap(), "val");
    }

    // ---- duplicates / multi-key ----

    #[test]
    fn duplicate_keys_last_wins() {
        let m = parse_env_file("ERW_IMAGE_TAG=v1\nERW_IMAGE_TAG=v2");
        assert_eq!(m.get("ERW_IMAGE_TAG").unwrap(), "v2");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn multiple_keys_all_parsed() {
        let input = "\
ERW_REGISTRY=cr.yandex/crppbh5k4v76t4ml9u8f
ERW_IMAGE_NAME=capacity-admission-webhook
ERW_IMAGE_TAG=latest
";
        let m = parse_env_file(input);
        assert_eq!(
            m,
            map(&[
                ("ERW_REGISTRY", "cr.yandex/crppbh5k4v76t4ml9u8f"),
                ("ERW_IMAGE_NAME", "capacity-admission-webhook"),
                ("ERW_IMAGE_TAG", "latest"),
            ])
        );
    }

    #[test]
    fn realistic_env_file_parses() {
        let input = "\
# ERW Verify configuration
ERW_REGISTRY=cr.yandex/crppbh5k4v76t4ml9u8f
ERW_IMAGE_NAME=capacity-admission-webhook
ERW_IMAGE_TAG=latest
ERW_KUBECONFIG=test.kubeconfig.yaml

# ERW_SKIP_BUILD=1
";
        let m = parse_env_file(input);
        assert_eq!(
            m.get("ERW_REGISTRY").unwrap(),
            "cr.yandex/crppbh5k4v76t4ml9u8f"
        );
        assert_eq!(
            m.get("ERW_IMAGE_NAME").unwrap(),
            "capacity-admission-webhook"
        );
        assert_eq!(m.get("ERW_IMAGE_TAG").unwrap(), "latest");
        assert_eq!(m.get("ERW_KUBECONFIG").unwrap(), "test.kubeconfig.yaml");
        assert!(
            !m.contains_key("ERW_SKIP_BUILD"),
            "commented line is skipped"
        );
        assert_eq!(m.len(), 4);
    }
}
