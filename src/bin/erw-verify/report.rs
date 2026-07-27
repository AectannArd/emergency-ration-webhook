//! Pure report rendering for the verify tool (spec-005, research R13).
//!
//! Takes a `Vec<ScenarioResult>` + run context and renders either a coloured
//! human-readable report ([`render_human`]) or a JSON object ([`render_json`]).
//! No I/O — fully unit-testable (Constitution Principle VIII). The orchestrator
//! is the only caller that prints the returned string.

use std::time::Duration;

use super::scenarios::{RunSummary, ScenarioResult, ScenarioStatus};

// ---- ANSI colour codes (contracts/cli.md: green ✓ / red ✗ / grey ○) ----
const GREEN: &str = "\u{1b}[32m";
const RED: &str = "\u{1b}[31m";
const GREY: &str = "\u{1b}[90m";
const RESET: &str = "\u{1b}[0m";

/// Width to which the "id  name" column is padded so per-scenario durations line up.
const NAME_COL_WIDTH: usize = 48;

/// Render the human-readable report (contracts/cli.md §Human-Readable).
///
/// `started_rfc3339` is the run start timestamp; `duration` is the total
/// wall-clock run duration. Pure — performs no I/O.
pub fn render_human(
    results: &[ScenarioResult],
    summary: &RunSummary,
    cluster_url: &str,
    started_rfc3339: &str,
    duration: Duration,
) -> String {
    let mut out = String::new();

    // 1. Run header.
    out.push_str("emergency-ration-webhook — on-demand verification\n");
    out.push_str(&format!("Cluster: {cluster_url}\n"));
    out.push_str(&format!("Started: {started_rfc3339}\n"));
    out.push('\n');

    // 2. Per-scenario blocks: coloured marker + id/name + duration + detail.
    for r in results {
        let label = format!("{}  {}", r.id, r.name);
        out.push_str(&format!(
            "{} {:<NAME_COL_WIDTH$}[{}]\n",
            marker_for(r.status),
            label,
            format_duration(r.duration),
        ));
        for line in r.detail.lines() {
            out.push_str(&format!("  {line}\n"));
        }
        out.push('\n');
    }

    // 3. Summary block.
    out.push_str(&separator());
    out.push_str(&format!(
        " Results: {} passed, {} failed, {} skipped ({} total)\n",
        summary.passed, summary.failed, summary.skipped, summary.total
    ));
    out.push_str(&format!(" Duration: {}\n", format_duration(duration)));
    out.push_str(&format!(" Exit code: {}\n", summary.exit_code));
    out.push_str(&separator());

    out
}

/// Coloured status marker: green ✓ (pass), red ✗ (fail), grey ○ (skip).
fn marker_for(status: ScenarioStatus) -> String {
    match status {
        ScenarioStatus::Pass => format!("{GREEN}✓{RESET}"),
        ScenarioStatus::Fail => format!("{RED}✗{RESET}"),
        ScenarioStatus::Skip => format!("{GREY}○{RESET}"),
    }
}

/// A horizontal rule for the summary block.
fn separator() -> String {
    let mut s = "─".repeat(NAME_COL_WIDTH + 2);
    s.push('\n');
    s
}

/// Format a duration compactly: `<60s` → `"{x:.1}s"`, else `"{m}m {s}s"`.
fn format_duration(d: Duration) -> String {
    if d.as_secs() >= 60 {
        format!("{}m {}s", d.as_secs() / 60, d.as_secs() % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Render the machine-readable JSON report (contracts/cli.md §JSON).
///
/// Emits a single pretty-printed JSON object with the exact schema from the CLI
/// contract. Pure — performs no I/O.
pub fn render_json(
    results: &[ScenarioResult],
    summary: &RunSummary,
    cluster_url: &str,
    started_rfc3339: &str,
    duration: Duration,
) -> String {
    let scenarios: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "name": r.name,
                "group": r.group.as_str(),
                "status": r.status.as_str(),
                "duration_secs": r.duration.as_secs_f64(),
                "detail": r.detail,
            })
        })
        .collect();

    let root = serde_json::json!({
        "cluster": cluster_url,
        "started": started_rfc3339,
        "duration_secs": duration.as_secs_f64(),
        "scenarios": scenarios,
        "summary": {
            "total": summary.total,
            "passed": summary.passed,
            "failed": summary.failed,
            "skipped": summary.skipped,
        },
        "exit_code": summary.exit_code,
    });

    serde_json::to_string_pretty(&root).expect("report JSON is always serialisable")
}
