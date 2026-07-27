# Phase 1: Data Model — On-Demand Infrastructure Verification

> Produced by `/speckit-plan`. The verify tool introduces no new CRDs or
> persistent Kubernetes objects — its data model is the internal Rust types that
> model the verification run lifecycle and the report.

## 1. Verification Run State Machine

A verification run has four phases. The tool transitions through them linearly,
with teardown always reached (unless `--keep-on-failure` is set and a scenario
fails).

```text
                  ┌─────────┐
                  │ Connect │  construct kube::Client from kubeconfig
                  └────┬────┘
                       │
                       ▼
               ┌───────────────┐
        ┌──────│ Pre-Flight    │  cluster-cleanness check (R16)
        │      │ Check         │  ── refuses if default ns has pods ──▶ ERROR (exit 2)
        │      └───────┬───────┘
        │              │ (clean)
        │              ▼
        │      ┌───────────────┐
        │      │ Setup         │  apply manifests (R2), generate TLS (R3-R4),
        │      │               │  wait readiness (R5)
        │      └───────┬───────┘
        │              │
        │     ┌────────┴────────┐
        │     ▼                 ▼
        │  ┌────────┐     ┌───────────┐
        │  │Setup OK│     │Setup FAIL │──▶ skip to Teardown (exit 2)
        │  └───┬────┘     └───────────┘
        │      │
        │      ▼
        │ ┌────────────────────────────────────────────┐
        │ │ Scenarios                                    │
        │ │  Phase A: enforcement (US1)                  │
        │ │    S1  admit small pod                       │
        │ │    S2  deny over-budget pod                  │
        │ │    S3  budgetPercent 0 (circuit-breaker)     │
        │ │    S4  budgetPercent 100 (physical limit)    │
        │ │    S5  runtime budget adjust (no restart)    │
        │ │    S6  dry-run mode (admit + warning)        │
        │ │    S7  capacity tracking accuracy            │
        │ │    S8  metrics + health endpoints            │
        │ │  Phase B: degradation (US2)                  │
        │ │    S9  kill pods → unreachable reject        │
        │ │    S10 delete CRD instances → missing reject │
        │ │    S11 stale capacity → freshness reject     │
        │ │  (each degradation scenario restores health) │
        │ └───────────────────────┬────────────────────┘
        │                         │
        │            ┌────────────┴────────────┐
        │            ▼                         ▼
        │     ┌────────────┐           ┌───────────────┐
        │     │ all passed │           │ some failed   │
        │     └──────┬─────┘           └───────┬───────┘
        │            │                         │
        │            │    ┌────────────────────┘
        │            │    │ (if --keep-on-failure: skip teardown)
        │            ▼    ▼
        │      ┌───────────────┐
        └─────▶│ Teardown      │  delete in reverse order (R12)
               │               │  ── partial failure ──▶ exit 3
               └───────┬───────┘
                       │
                       ▼
               ┌───────────────┐
               │ Report        │  human-readable (default) or JSON (--json)
               └───────┬───────┘
                       │
                       ▼
                  exit code
```

## 2. Core Types

### ScenarioResult

The fundamental unit produced by each scenario. The report module consumes a
`Vec<ScenarioResult>`.

```rust
/// Outcome of a single verification scenario.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// Human-readable scenario name (e.g. "S2: over-budget pod denied").
    pub name: String,
    /// Which user story / scenario group this belongs to.
    pub group: ScenarioGroup,
    /// Pass / Fail / Skip (skipped if a prior setup step failed).
    pub status: ScenarioStatus,
    /// Wall-clock duration of the scenario.
    pub duration: Duration,
    /// On failure: expected vs actual outcome + error detail from the cluster.
    /// On pass: a short confirmation (e.g. "pod admitted", "pod denied with 403").
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioGroup {
    /// User Story 1: enforcement scenarios.
    Enforcement,
    /// User Story 2: active degradation scenarios.
    Degradation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioStatus {
    Pass,
    Fail,
    Skip,
}
```

### RunSummary

The aggregate result, computed from the `Vec<ScenarioResult>`:

```rust
/// Aggregated outcome of the full verification run.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Exit code derived from the outcomes (see §3).
    pub exit_code: i32,
}
```

### VerifyConfig

The tool's CLI-resolved configuration:

```rust
/// Configuration for the verify tool, resolved from CLI flags.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Path to kubeconfig (flag > KUBECONFIG env > ~/.kube/config).
    pub kubeconfig: Option<PathBuf>,
    /// Emit JSON instead of human-readable report.
    pub json: bool,
    /// Skip teardown if a scenario fails (for debugging).
    pub keep_on_failure: bool,
    /// Timeout for setup readiness waits (seconds).
    pub timeout_secs: u64,
}
```

## 3. Exit-Code Derivation

```text
  exit 0  →  all scenarios passed AND teardown succeeded
  exit 1  →  one or more scenarios failed (teardown still attempted)
  exit 2  →  setup error (cluster unreachable, pre-flight check failed,
             manifests failed to apply, readiness timeout)
  exit 3  →  teardown partial failure (scenarios may have passed, but the
             cluster was not fully cleaned up)
```

When multiple error conditions apply, the most severe wins (setup error >
scenario failure > teardown failure).

## 4. Validation Rules

- `timeout_secs` MUST be > 0 (default 120).
- `kubeconfig` path, if provided, MUST be a readable file (validated at client
  construction — a clear error is reported if not).
- The scenario list is fixed (not configurable in v1); a future iteration may
  add `--only <scenario-name>` filtering.

## 5. No Persistent State

The verify tool creates NO new CRDs, no new Kubernetes object types, and no
on-disk artifacts. Everything it creates in the cluster is deleted at teardown.
Its internal state (the `Vec<ScenarioResult>`) lives only in process memory and
is emitted as the report on stdout. This is consistent with the throwaway-
cluster model and Principle V (minimal surface).
