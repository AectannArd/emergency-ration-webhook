//! On-demand infrastructure verification tool — entry point (spec-005, T024;
//! spec-009 adds the `.env`-driven build+push prefix).
//!
//! Wires the full run lifecycle (data-model §1 state machine):
//! install rustls provider → load `.env` + resolve config → config pre-flight →
//! (optionally build + push image) → build client → pre-flight check →
//! setup (TLS + manifests + caBundle + readiness) → enforcement scenarios →
//! teardown → render report → exit code.
//!
//! Exit codes (data-model §3, most severe wins: setup > scenario > teardown):
//!   0  all scenarios passed AND teardown succeeded
//!   1  one or more scenarios failed
//!   2  setup error (cluster unreachable / not empty / manifests / readiness /
//!      build or push failure — spec-009)
//!   3  teardown partial failure (scenarios may have passed)
//!   4  configuration error (.env missing a required variable, docker not on
//!      PATH) — spec-009

mod args;
mod client;
mod env_config;
mod error;
mod image;
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

    // Load .env from the repo root (cwd) BEFORE resolving config (FR-001/FR-004).
    let env_file = env_config::load_env_file();
    let config = VerifyConfig::load(&env_file);
    tracing::info!(
        skip_build = config.skip_build,
        registry = ?config.registry,
        image = ?config.image_name,
        tag = ?config.image_tag,
        "configuration resolved",
    );

    // Config pre-flight (spec-009, FR-009): validate build config + resolve the
    // fully-qualified image. A configuration error (missing ERW_REGISTRY, or
    // docker absent when building) exits 4 before any network action.
    let resolved_image = match preflight(&config) {
        Ok(image) => image,
        Err(message) => {
            eprintln!("ERROR: {message}");
            return ExitCode::from(4);
        }
    };

    match run(&config, resolved_image.as_deref()).await {
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
            // Pre-report failure (client / pre-flight / setup / build+push):
            // error to stderr, exit 2, no report (contracts/cli.md §Error Output
            // — JSON is never emitted before the report phase).
            eprintln!("ERROR: {message}");
            ExitCode::from(2)
        }
    }
}

/// Validate build configuration and resolve the fully-qualified image reference
/// (spec-009, FR-009). Returns:
/// - `Ok(Some(ref))` — build+push will run, or `--skip-build` reuses this ref.
/// - `Ok(None)` — `--skip-build` with no registry: the manifest placeholder is
///   left as-is (operator opt-out; US3 acceptance scenario 2).
/// - `Err(msg)` — configuration error → exit 4: a required variable is missing
///   or `docker` is not on `PATH`.
fn preflight(config: &VerifyConfig) -> Result<Option<String>, String> {
    let image_ref = config
        .registry
        .as_ref()
        .map(|r| image::fully_qualified_image(r, &config.image_name, &config.image_tag));
    if config.skip_build {
        return Ok(image_ref);
    }
    if config.registry.is_none() {
        return Err("Missing required configuration: ERW_REGISTRY. \
             Copy .env.example to .env and fill in your values."
            .into());
    }
    if !image::docker_available() {
        return Err("docker not found on PATH. Install Docker or use --skip-build.".into());
    }
    Ok(image_ref)
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
async fn run(config: &VerifyConfig, image: Option<&str>) -> Result<ReportData, String> {
    let run_start = Instant::now();
    let started = time_util::now_rfc3339();

    // Build + push phase (spec-009, FR-005/FR-006/FR-011): runs BEFORE any
    // cluster connection so a build/push failure aborts before cluster resources
    // are touched (exit 2 — a setup-class error per data-model §3).
    if !config.skip_build {
        let image_ref = image.ok_or_else(|| {
            "internal error: build phase started without a resolved image reference".to_string()
        })?;
        tracing::info!(image = %image_ref, "building webhook image");
        image::build_image(image_ref).await?;
        tracing::info!(image = %image_ref, "pushing webhook image to registry");
        image::push_image(image_ref).await?;
        tracing::info!(image = %image_ref, "image built and pushed");
    }

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
    if let Err(e) = run_setup(&client, config.timeout_secs, image).await {
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
    tracing::info!("degradation scenarios complete; running equalizer scenarios");
    // spec-013 (FR-015): cross-cluster equalizer verification. Opt-in — skipped
    // (not failed) when no ERW_EQUALIZER_TARGET_KUBECONFIG_* are set, so the
    // standard single-cluster run is unaffected.
    let eq_config = scenarios::equalizer::EqualizerRunConfig {
        home_kubeconfig: config.kubeconfig.clone(),
        registry: config.registry.clone(),
        image_tag: config.image_tag.clone(),
        skip_build: config.skip_build,
    };
    results.extend(scenarios::equalizer::run(&eq_config, &client).await);
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
/// → readiness). Any failure aborts setup. `image`, when `Some`, is substituted
/// into the applied Deployment (spec-009, FR-007).
async fn run_setup(
    client: &kube::Client,
    timeout_secs: u64,
    image: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // TLS Secret first (it ensures the namespace), so the Deployment pods mount
    // it as soon as they start.
    let cert_pem = setup::create_tls_secret(client).await?;
    setup::apply_manifests(client, image).await?;
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
