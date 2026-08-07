//! Binary entry point for the multi-cluster capacity equalizer (spec-013).
//!
//! Wires the reconcile runtime loop: install the rustls CryptoProvider → init
//! tracing → construct the home-cluster client → poll the `EqualizerConfig`
//! singleton every `EQUALIZER_RECONCILE_INTERVAL_SECS` (default 10s) →
//! [`reconcile`] → write the resulting status back. The equalizer is NOT on the
//! admission path (Constitution Principle I does not apply): it only tunes
//! per-cluster budget overrides.
//!
//! Per contract §4.2 the equalizer does NOT auto-create the `EqualizerConfig`
//! singleton — the operator must create it; the binary logs a warning and idles
//! while it is absent.

use std::time::Duration;

use capacity_admission_webhook::equalizer::crd::{EqualizerConfig, FLEET_EQUALIZER_NAME};
use capacity_admission_webhook::equalizer::reconcile::reconcile;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Default reconcile interval (data-model §3, contract §4.1).
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 10;

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
    // including `Client::try_default()` / `Config::from_custom_kubeconfig`
    // (research R1, CI failure catalog Layer 2). Mirrors the webhook binary.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install ring CryptoProvider");

    init_tracing();
    let interval_secs = reconcile_interval_secs();
    info!(
        interval_secs,
        "starting multi-cluster capacity equalizer (spec-013)"
    );

    let home_client = Client::try_default().await?;
    let eq_api = Api::<EqualizerConfig>::all(home_client.clone());

    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        match eq_api.get(FLEET_EQUALIZER_NAME).await {
            Ok(eq_config) => {
                let status = reconcile(&home_client, &eq_config).await;
                patch_status(&eq_api, &status).await;
                info!(
                    condition = ?status.condition,
                    clusters = status.clusters.len(),
                    "reconciled fleet"
                );
            }
            Err(err) if is_not_found(&err) => {
                // Contract §4.2: the operator must create the singleton; do NOT
                // auto-create. Log + idle until it appears.
                warn!(
                    name = FLEET_EQUALIZER_NAME,
                    "EqualizerConfig singleton absent; idling (operator must create it)"
                );
            }
            Err(err) => {
                warn!(%err, "failed to read EqualizerConfig; retrying next cycle");
            }
        }
    }
}

/// The reconcile interval, from `EQUALIZER_RECONCILE_INTERVAL_SECS` (default 10s),
/// clamped to a positive value.
fn reconcile_interval_secs() -> u64 {
    std::env::var("EQUALIZER_RECONCILE_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&secs| secs > 0)
        .unwrap_or(DEFAULT_RECONCILE_INTERVAL_SECS)
}

/// Whether a kube error is a 404 NotFound — i.e. the singleton is absent.
fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 404)
}

/// Merge-patch the `EqualizerConfig.status` subresource. The body is wrapped under
/// a top-level `status` key (a bare status object is a silent no-op on the
/// `/status` subresource). Failures are logged, not fatal — the next cycle retries.
async fn patch_status(
    api: &Api<EqualizerConfig>,
    status: &capacity_admission_webhook::equalizer::crd::EqualizerConfigStatus,
) {
    let body = serde_json::json!({ "status": status });
    if let Err(err) = api
        .patch_status(
            FLEET_EQUALIZER_NAME,
            &PatchParams::default(),
            &Patch::Merge(&body),
        )
        .await
    {
        warn!(%err, "failed to patch EqualizerConfig status");
    }
}
