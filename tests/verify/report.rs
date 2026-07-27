//! Unit tests for the verify tool's pure report + summary modules (spec-005).
//!
//! `#[path]`-includes the binary's source files under `src/bin/erw-verify/` so the
//! pure logic stays out of the library crate (Constitution Principle V) while
//! remaining unit-testable without a cluster (Principle VIII).

// The verify modules live under the binary (Constitution Principle V keeps
// verify logic out of the library crate). `#[path]` includes their source here so
// the pure logic is unit-testable without a cluster (Principle VIII). The test
// exercises only the pure surface, so allow dead code on the included modules —
// the binary build still gets full dead-code checking on every item.
#[allow(dead_code)]
#[path = "../../src/bin/erw-verify/report.rs"]
mod report;
#[allow(dead_code)]
#[path = "../../src/bin/erw-verify/scenarios/mod.rs"]
mod scenarios;

use std::time::Duration;

use report::{render_human, render_json};
use scenarios::{ScenarioGroup, ScenarioResult, ScenarioStatus, derive_summary};

/// Helper: a result with the given id/status.
fn result(id: &str, status: ScenarioStatus) -> ScenarioResult {
    ScenarioResult {
        id: id.into(),
        name: "scenario".into(),
        group: ScenarioGroup::Enforcement,
        status,
        duration: Duration::from_millis(100),
        detail: "detail".into(),
    }
}

// ===========================================================================
// T007 — RunSummary / exit-code derivation (data-model §3)
// ===========================================================================

#[test]
fn all_pass_yields_exit_zero() {
    let results = vec![
        result("S1", ScenarioStatus::Pass),
        result("S2", ScenarioStatus::Pass),
        result("S3", ScenarioStatus::Pass),
    ];
    let summary = derive_summary(&results);
    assert_eq!(summary.total, 3);
    assert_eq!(summary.passed, 3);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.exit_code, 0);
}

#[test]
fn one_fail_yields_exit_one() {
    let results = vec![
        result("S1", ScenarioStatus::Pass),
        result("S2", ScenarioStatus::Fail),
        result("S3", ScenarioStatus::Pass),
    ];
    let summary = derive_summary(&results);
    assert_eq!(summary.total, 3);
    assert_eq!(summary.passed, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.exit_code, 1, "any failure must produce exit code 1");
}

#[test]
fn all_skip_yields_exit_zero() {
    let results = vec![
        result("S1", ScenarioStatus::Skip),
        result("S2", ScenarioStatus::Skip),
    ];
    let summary = derive_summary(&results);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.skipped, 2);
    assert_eq!(summary.exit_code, 0, "skips are not failures");
}

#[test]
fn empty_results_is_success() {
    let summary = derive_summary(&[]);
    assert_eq!(summary.total, 0);
    assert_eq!(summary.exit_code, 0);
}

// ===========================================================================
// T008 — human-readable report rendering (contracts/cli.md §Human-Readable)
// ===========================================================================

/// Build a richer result (multi-line detail) for the render tests.
fn detailed_result(id: &str, name: &str, status: ScenarioStatus, detail: &str) -> ScenarioResult {
    ScenarioResult {
        id: id.into(),
        name: name.into(),
        group: ScenarioGroup::Enforcement,
        status,
        duration: Duration::from_millis(1200),
        detail: detail.into(),
    }
}

#[test]
fn render_human_has_header_scenario_blocks_and_summary() {
    let results = vec![
        detailed_result(
            "S1",
            "within-budget pod admitted",
            ScenarioStatus::Pass,
            "pod default/erw-smoke-ok created",
        ),
        detailed_result(
            "S2",
            "over-budget pod denied",
            ScenarioStatus::Fail,
            "expected: pod rejected with HTTP 403\nactual: pod was admitted",
        ),
        detailed_result(
            "S3",
            "skipped scenario",
            ScenarioStatus::Skip,
            "setup failed",
        ),
    ];
    let summary = derive_summary(&results);
    let out = render_human(
        &results,
        &summary,
        "https://10.0.0.1:6443",
        "2026-07-27T14:32:05Z",
        Duration::from_secs(272),
    );

    // Header.
    assert!(out.contains("emergency-ration-webhook — on-demand verification"));
    assert!(out.contains("Cluster: https://10.0.0.1:6443"));
    assert!(out.contains("Started: 2026-07-27T14:32:05Z"));

    // Per-scenario markers + ids + names (ANSI colour codes surround the marker,
    // so assert the bare glyph + id + name separately).
    assert!(out.contains('✓'), "pass marker present");
    assert!(out.contains('✗'), "fail marker present");
    assert!(out.contains('○'), "skip marker present");
    assert!(out.contains("S1") && out.contains("within-budget pod admitted"));
    assert!(out.contains("S2") && out.contains("over-budget pod denied"));
    assert!(out.contains("S3"));

    // Detail line is present and indented.
    assert!(out.contains("pod default/erw-smoke-ok created"));
    assert!(
        out.contains("  expected: pod rejected with HTTP 403"),
        "multi-line detail is indented two spaces"
    );

    // Summary counts + exit code.
    assert!(out.contains("Results: 1 passed, 1 failed, 1 skipped (3 total)"));
    assert!(out.contains("Exit code: 1"));
}

#[test]
fn render_human_all_pass_has_zero_exit_code() {
    let results = vec![detailed_result("S1", "ok", ScenarioStatus::Pass, "fine")];
    let summary = derive_summary(&results);
    let out = render_human(
        &results,
        &summary,
        "https://cluster",
        "2026-07-27T14:32:05Z",
        Duration::from_secs(5),
    );
    assert!(out.contains("Results: 1 passed, 0 failed, 0 skipped (1 total)"));
    assert!(out.contains("Exit code: 0"));
}

// ===========================================================================
// T029 — JSON report rendering (contracts/cli.md §JSON)
// ===========================================================================

#[test]
fn render_json_matches_contract_schema() {
    let results = vec![
        detailed_result(
            "S1",
            "within-budget pod admitted",
            ScenarioStatus::Pass,
            "pod default/erw-smoke-ok created",
        ),
        detailed_result(
            "S2",
            "over-budget pod denied",
            ScenarioStatus::Fail,
            "expected: pod rejected with HTTP 403",
        ),
    ];
    let summary = derive_summary(&results);
    let json_str = render_json(
        &results,
        &summary,
        "https://10.0.0.1:6443",
        "2026-07-27T14:32:05Z",
        Duration::from_millis(272_400),
    );

    let v: serde_json::Value = serde_json::from_str(&json_str).expect("output is valid JSON");

    assert_eq!(v["cluster"], "https://10.0.0.1:6443");
    assert_eq!(v["started"], "2026-07-27T14:32:05Z");
    let run_secs = v["duration_secs"]
        .as_f64()
        .expect("duration_secs is a number");
    assert!((run_secs - 272.4).abs() < 1e-6, "run duration {run_secs}");

    let scenarios = v["scenarios"].as_array().expect("scenarios is an array");
    assert_eq!(scenarios.len(), 2);
    assert_eq!(scenarios[0]["id"], "S1");
    assert_eq!(scenarios[0]["name"], "within-budget pod admitted");
    assert_eq!(scenarios[0]["group"], "enforcement");
    assert_eq!(scenarios[0]["status"], "pass");
    let s0_dur = scenarios[0]["duration_secs"].as_f64().unwrap();
    assert!((s0_dur - 1.2).abs() < 1e-6, "scenario duration {s0_dur}");
    assert_eq!(scenarios[0]["detail"], "pod default/erw-smoke-ok created");

    assert_eq!(scenarios[1]["status"], "fail");
    assert_eq!(scenarios[1]["group"], "enforcement");

    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["passed"], 1);
    assert_eq!(v["summary"]["failed"], 1);
    assert_eq!(v["summary"]["skipped"], 0);
    assert_eq!(v["exit_code"], 1);
}

#[test]
fn render_json_has_no_stray_top_level_keys() {
    // The contract schema defines exactly 6 top-level keys.
    let results = vec![detailed_result("S1", "ok", ScenarioStatus::Pass, "d")];
    let summary = derive_summary(&results);
    let json_str = render_json(
        &results,
        &summary,
        "https://c",
        "2026-01-01T00:00:00Z",
        Duration::from_secs(1),
    );
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let keys: std::collections::HashSet<&str> =
        v.as_object().unwrap().keys().map(String::as_str).collect();
    let expected: std::collections::HashSet<&str> = [
        "cluster",
        "started",
        "duration_secs",
        "scenarios",
        "summary",
        "exit_code",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "no stray top-level keys; got {keys:?}");
}
