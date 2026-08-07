//! Binary entry point for the multi-cluster capacity equalizer (spec-013).
//!
//! Phase 2 (T022): installs the rustls CryptoProvider as the first line (before
//! any `Client`/`Config` construction — kube-rs touches rustls even for
//! plain-HTTP kubeconfigs) and initialises structured tracing. Phase 3 (T026)
//! fills in the reconcile runtime loop.

use tracing::info;
use tracing_subscriber::EnvFilter;

/// Initialise structured tracing before any component starts.
///
/// Log level is taken from `RUST_LOG`, defaulting to `info` when unset. Must be
/// called exactly once at startup (it installs the process-global subscriber).
fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install the rustls CryptoProvider FIRST — before any TLS operation,
    // including kube `Client::try_default()` / `Config::from_custom_kubeconfig`
    // (research R1, CI failure catalog Layer 2). Mirrors the webhook binary.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install ring CryptoProvider");

    init_tracing();
    info!("starting multi-cluster capacity equalizer (spec-013)");

    // Phase 3 (T026) fills in the reconcile runtime loop: read the
    // `EqualizerConfig` singleton, reconcile() every 10s, patch its status.
    todo!("reconcile runtime loop (Phase 3, T026)");
}
