//! Scenario result types and run-summary derivation (spec-005, data-model §2-3).
//!
//! These are the internal types modelling the verification-run lifecycle. The
//! report module ([`crate::report`]) consumes a `Vec<ScenarioResult>`; the
//! orchestrator ([`crate::main`]) layers the setup (2) / teardown (3) exit codes
//! on top of the scenario-derived exit code produced here.

use std::time::Duration;

pub mod degradation;
pub mod enforcement;
pub mod equalizer;

/// Outcome of a single verification scenario.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// Scenario identifier (e.g. `"S1"`).
    pub id: String,
    /// Human-readable scenario name (e.g. `"within-budget pod admitted"`).
    pub name: String,
    /// Which user story / scenario group this belongs to.
    pub group: ScenarioGroup,
    /// Pass / Fail / Skip (skipped when a prior setup step failed).
    pub status: ScenarioStatus,
    /// Wall-clock duration of the scenario.
    pub duration: Duration,
    /// On pass: a short confirmation. On fail: expected vs actual + error detail.
    pub detail: String,
}

/// Which scenario group a result belongs to (US1 vs US2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioGroup {
    /// User Story 1: enforcement scenarios (S1-S8).
    Enforcement,
    /// User Story 2: active degradation scenarios (S9-S11).
    Degradation,
    /// spec-013: multi-cluster equalizer scenarios (E1-E5). Opt-in — skipped
    /// when no target cluster kubeconfigs are supplied.
    Equalizer,
}

impl ScenarioGroup {
    /// Lower-case label for the JSON report (`"enforcement"` / `"degradation"`
    /// / `"equalizer"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioGroup::Enforcement => "enforcement",
            ScenarioGroup::Degradation => "degradation",
            ScenarioGroup::Equalizer => "equalizer",
        }
    }
}

/// Pass / Fail / Skip outcome of a single scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioStatus {
    Pass,
    Fail,
    /// A scenario that did not run (e.g. setup failed before it). The MVP slices
    /// scenarios as Pass/Fail (a setup failure aborts before scenarios run), so
    /// `Skip` is constructed by the report unit tests and reserved for the
    /// "report even when setup failed" enhancement. Allowed dead until then.
    #[allow(dead_code)]
    Skip,
}

impl ScenarioStatus {
    /// Lower-case label for the JSON report (`"pass"` / `"fail"` / `"skip"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioStatus::Pass => "pass",
            ScenarioStatus::Fail => "fail",
            ScenarioStatus::Skip => "skip",
        }
    }
}

/// Aggregated outcome of the full verification run.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Exit code derived from scenario outcomes (0 all-pass/all-skip, 1 any fail).
    /// The setup (2) and teardown (3) codes are layered on by the orchestrator
    /// per data-model §3 (most severe wins: setup > scenario > teardown).
    pub exit_code: i32,
}

/// Aggregate scenario results into a [`RunSummary`] (data-model §3).
///
/// Exit code is scenario-derived only: `0` when no scenario failed (all pass OR
/// all skip), `1` when one or more failed. The orchestrator overrides this with
/// `2` (setup error) or `3` (teardown failure) when those conditions apply.
pub fn derive_summary(results: &[ScenarioResult]) -> RunSummary {
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    for r in results {
        match r.status {
            ScenarioStatus::Pass => passed += 1,
            ScenarioStatus::Fail => failed += 1,
            ScenarioStatus::Skip => skipped += 1,
        }
    }
    RunSummary {
        total: results.len(),
        passed,
        failed,
        skipped,
        // 0 when no scenario failed (all pass OR all skip); 1 if any failed.
        // Setup (2) / teardown (3) codes are layered on by the orchestrator.
        exit_code: i32::from(failed > 0),
    }
}
