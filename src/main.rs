//! Binary entry point for the capacity admission webhook.
//!
//! Wires the three components onto one process (Constitution Principle V):
//! the Node Capacity Controller and Allocation Controller (background tasks), and
//! the Admission Webhook (HTTPS server). The webhook reads allocation figures
//! only from the in-process reflector cache — no network on the hot path.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use capacity_admission_webhook::config::Config;
use capacity_admission_webhook::controllers;
use capacity_admission_webhook::crd::Allocation;
use capacity_admission_webhook::webhook::handler::{AppState, router};

use kube::runtime::{reflector, watcher};
use kube::{Api, Client};

/// Initialise structured tracing before any component starts.
///
/// Log level is taken from `RUST_LOG`, defaulting to `info` when unset. Must be
/// called exactly once at startup (it installs the process-global subscriber).
pub fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let config = Config::load();
    info!(
        port = config.port,
        namespace = %config.namespace,
        decision_timeout_ms = config.decision_timeout_ms,
        "starting capacity admission webhook"
    );

    let client = Client::try_default().await?;

    // Allocation reflector: the webhook's hot-path cache. Kept warm by a
    // background task so admission decisions never hit the apiserver.
    let (allocation_store, writer) = reflector::store::<Allocation>();
    let allocation_api = Api::<Allocation>::all(client.clone());
    tokio::spawn(
        reflector::reflector(
            writer,
            watcher::watcher(allocation_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "allocation watch error");
            }
        }),
    );

    // Capacity supply + demand controllers (CRDs are the shared state).
    tokio::spawn(controllers::node_capacity::run(client.clone()));
    tokio::spawn(controllers::allocation::run(client.clone()));

    // HTTPS admission server.
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &config.tls_cert_file,
        &config.tls_key_file,
    )
    .await?;
    let app = router(AppState {
        allocation_store: Arc::new(allocation_store),
    });
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));

    let handle = axum_server::Handle::new();
    tokio::spawn(shutdown_signal(handle.clone()));

    info!(%addr, "admission server listening on HTTPS");
    axum_server::bind_rustls(addr, tls)
        .handle(handle)
        .serve(app.into_make_service())
        .await?;
    info!("admission server stopped");
    Ok(())
}

/// Wait for SIGTERM/SIGINT, then trigger a graceful drain of the HTTPS server.
async fn shutdown_signal(handle: axum_server::Handle<SocketAddr>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received; draining in-flight admission requests");
    handle.graceful_shutdown(Some(Duration::from_secs(10)));
}
