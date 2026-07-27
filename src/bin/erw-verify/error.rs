//! Error type for the verify tool (spec-005).
//!
//! Every verify operation can fail with a boxed, `Send + Sync` error — a kube
//! error, an rcgen/serde_yaml/serde_json error, or a context string. The
//! orchestrator maps the *phase* that failed (setup vs teardown) to the exit
//! code (data-model §3); it does not classify by error variant, so a single
//! opaque error type suffices.

/// Result alias shared by all verify modules.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Build a boxed error from a context string.
pub(crate) fn err(msg: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    let s: String = msg.into();
    s.into()
}
