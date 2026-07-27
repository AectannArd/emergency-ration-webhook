//! On-demand infrastructure verification tool — entry point (spec-005, T024).
//!
//! Wires the full run lifecycle (data-model §1 state machine):
//! install rustls provider → parse args → build client → pre-flight check →
//! setup (TLS + manifests + caBundle + readiness) → enforcement scenarios →
//! teardown → render report → exit code.
//!
//! Exit codes (data-model §3, most severe wins: setup > scenario > teardown):
//!   0  all scenarios passed AND teardown succeeded
//!   1  one or more scenarios failed
//!   2  setup error (cluster unreachable / not empty / manifests / readiness)
//!   3  teardown partial failure (scenarios may have passed)

mod args;
mod client;
mod error;
mod report;
mod scenarios;
mod setup;
mod teardown;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use tracing_subscriber::EnvFilter;

use capacity_admission_webhook::time_util;

use crate::args::VerifyConfig;
use crate::report::{render_human, render_json};
use crate::scenarios::{ScenarioStatus, derive_summary};

/// Initialised structured tracing before any component starts (mirrors the
/// webhook binary). Level from `RUST_LOG`, defaulting to `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

#[tokio::main]
async fn main() -> ExitCode {
    // research R17: install the rustls CryptoProvider as the FIRST operation,
    // before any TLS connection (including build_client → Config::infer).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install ring CryptoProvider");

    init_tracing();

    let config = VerifyConfig::load();

    match run(&config).await {
        Ok(report_data) => {
            let summary = derive_summary(&report_data.results);
            let final_code: u8 = if summary.failed > 0 {
                1
            } else if report_data.teardown_failed {
                3
            } else {
                0
            };

            // The report's "Exit code" reflects the final process exit (which
            // may be 3 on a teardown failure the scenario summary cannot see).
            let mut summary = summary;
            summary.exit_code = final_code as i32;

            let report = if config.json {
                render_json(
                    &report_data.results,
                    &summary,
                    &report_data.cluster_url,
                    &report_data.started,
                    report_data.duration,
                )
            } else {
                render_human(
                    &report_data.results,
                    &summary,
                    &report_data.cluster_url,
                    &report_data.started,
                    report_data.duration,
                )
            };

            if config.json {
                println!("{report}");
            } else {
                print!("{report}");
            }
            ExitCode::from(final_code)
        }
        Err(message) => {
            // Pre-report failure (client / pre-flight / setup): error to stderr,
            // exit 2, no report (contracts/cli.md §Error Output — JSON is never
            // emitted before the report phase).
            eprintln!("ERROR: {message}");
            ExitCode::from(2)
        }
    }
}

/// The data needed to render the report once scenarios have run.
struct ReportData {
    results: Vec<scenarios::ScenarioResult>,
    teardown_failed: bool,
    cluster_url: String,
    started: String,
    duration: Duration,
}

/// Drive the full run. Returns `Err` for any pre-report failure (→ exit 2, no
/// report); `Ok` once scenarios have executed (→ report + 0/1/3).
async fn run(config: &VerifyConfig) -> Result<ReportData, String> {
    let run_start = Instant::now();
    let started = time_util::now_rfc3339();

    let (client, cluster_url) = client::build_client(config.kubeconfig.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(%cluster_url, "connected to cluster");

    setup::check_cluster_clean(&client)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("pre-flight check passed: default namespace is empty");

    // Setup. On failure, clean up the partial install (unless the operator asked
    // to keep it), then surface the error as a setup failure (exit 2, no report).
    if let Err(e) = run_setup(&client, config.timeout_secs).await {
        let message = e.to_string();
        tracing::warn!(error = %message, "setup failed");
        if !config.keep_on_failure {
            teardown_quietly(&client).await;
        }
        return Err(message);
    }
    tracing::info!("setup complete; running verification scenarios");

    let mut results = scenarios::enforcement::run(&client).await;
    tracing::info!("enforcement scenarios complete; running degradation scenarios");
    results.extend(scenarios::degradation::run(&client).await);
    let any_failed = results.iter().any(|r| r.status == ScenarioStatus::Fail);

    // Teardown unless the operator asked to keep the install for debugging AND a
    // scenario failed. A teardown failure does not mask a scenario failure
    // (scenario > teardown), but surfaces as exit 3 when scenarios all passed.
    let mut teardown_failed = false;
    let should_teardown = !(config.keep_on_failure && any_failed);
    if should_teardown && let Err(te) = teardown::teardown(&client).await {
        teardown_failed = true;
        eprintln!("ERROR: teardown: {te}");
    }

    Ok(ReportData {
        results,
        teardown_failed,
        cluster_url,
        started,
        duration: run_start.elapsed(),
    })
}

/// Apply the webhook stack and wait for it to be ready (TLS → manifests → caBundle
/// → readiness). Any failure aborts setup.
async fn run_setup(
    client: &kube::Client,
    timeout_secs: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // TLS Secret first (it ensures the namespace), so the Deployment pods mount
    // it as soon as they start.
    let cert_pem = setup::create_tls_secret(client).await?;
    setup::apply_manifests(client).await?;
    setup::inject_ca_bundle(client, &cert_pem).await?;
    setup::wait_for_readiness(client, Duration::from_secs(timeout_secs)).await?;
    Ok(())
}

/// Best-effort teardown used after a setup failure: log any error and continue.
async fn teardown_quietly(client: &kube::Client) {
    if let Err(te) = teardown::teardown(client).await {
        tracing::warn!(error = %te, "teardown after setup failure also failed");
    }
}
