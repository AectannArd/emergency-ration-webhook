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
use capacity_admission_webhook::crd::{Allocation, ClusterCapacity};
use capacity_admission_webhook::metrics::Metrics;
use capacity_admission_webhook::time_util;
use capacity_admission_webhook::webhook::handler::{
    AppState, metrics_router, refresh_gauges, router,
};

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
    // Install the rustls crypto provider FIRST — before any TLS operation
    // (including kube Client::try_default(), which opens a TLS connection to
    // the apiserver). Without this, rustls 0.23 panics with "Could not
    // automatically determine the process-level CryptoProvider" because
    // axum-server uses tls-rustls-no-provider, so auto-detection fails.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install ring CryptoProvider");

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
    let (allocation_store, alloc_writer) = reflector::store::<Allocation>();
    let allocation_api = Api::<Allocation>::all(client.clone());
    tokio::spawn(
        reflector::reflector(
            alloc_writer,
            watcher::watcher(allocation_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "allocation watch error");
            }
        }),
    );

    // ClusterCapacity reflector: feeds the total-allocatable capacity gauges
    // (SC-003). The admission decision itself uses Allocation status only.
    let (capacity_store, cap_writer) = reflector::store::<ClusterCapacity>();
    let capacity_api = Api::<ClusterCapacity>::all(client.clone());
    tokio::spawn(
        reflector::reflector(
            cap_writer,
            watcher::watcher(capacity_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "ClusterCapacity watch error");
            }
        }),
    );

    // Capacity supply + demand controllers (CRDs are the shared state).
    tokio::spawn(controllers::node_capacity::run(client.clone()));
    tokio::spawn(controllers::allocation::run(client.clone()));

    let metrics = Arc::new(Metrics::new());
    let allocation_store = Arc::new(allocation_store);
    let capacity_store = Arc::new(capacity_store);

    // Keep the capacity gauges + freshness current between admission requests
    // (T029). Per-decision refresh in the handler covers the SC-003 invariant;
    // this refresh keeps metrics live during idle periods.
    {
        let metrics = Arc::clone(&metrics);
        let allocation_store = Arc::clone(&allocation_store);
        let capacity_store = Arc::clone(&capacity_store);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            loop {
                ticker.tick().await;
                refresh_gauges(
                    &metrics,
                    &allocation_store,
                    &capacity_store,
                    time_util::now_unix(),
                );
            }
        });
    }

    // HTTPS admission server + plaintext HTTP metrics/probe server, sharing one
    // shutdown handle.
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &config.tls_cert_file,
        &config.tls_key_file,
    )
    .await?;
    let state = AppState::new(
        allocation_store,
        capacity_store,
        config.decision_timeout_ms,
        config.capacity_freshness_timeout_secs,
        metrics,
    );

    let handle = axum_server::Handle::new();
    tokio::spawn(shutdown_signal(handle.clone()));

    // Plaintext HTTP scrape/probe server (/metrics, /healthz) — Prometheus and
    // kubelet reach these without TLS.
    let metrics_addr = SocketAddr::from(([0, 0, 0, 0], config.metrics_port));
    let metrics_app = metrics_router(state.clone());
    info!(%metrics_addr, "metrics server listening on HTTP");
    tokio::spawn(
        axum_server::bind(metrics_addr)
            .handle(handle.clone())
            .serve(metrics_app.into_make_service()),
    );

    // HTTPS admission server (/validate).
    let app = router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
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
