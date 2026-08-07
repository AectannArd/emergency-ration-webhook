# Implementation Plan: Multi-Cluster Capacity Equalizer (spec-013)

**Branch**: `013-multi-cluster-capacity-equalizer` | **Date**: 2026-08-06 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/013-multi-cluster-capacity-equalizer/spec.md`

## Summary

A new controller binary (`capacity-equalizer`), packaged as its own Docker image,
deployed in a single cluster, that manages a fleet of N Kubernetes clusters. It
reads an `EqualizerConfig` CRD specifying per-resource cumulative budget targets
and a list of target clusters (each via a kubeconfig Secret). Every reconcile
cycle, it reads each target cluster's `Allocation.status` (current utilization)
and `ClusterCapacity.status` (total allocatable), computes the equalized
per-resource budgets using the overflow-distribution algorithm, and patches each
target's `Allocation.spec.cpuBudgetPercent` / `memoryBudgetPercent` (the spec-012
override fields).

The equalization algorithm (per resource, independently): clusters over the
target are frozen at their current utilization; the total absolute overflow
(sum of `(utilization − target) × capacity / 100` per over-cluster, in CPU milli
or RAM bytes) is divided equally among the good-state clusters; each good
cluster's budget is lowered by the corresponding percentage points below target.
The fleet average converges to the target. When the over-cluster drops, budgets
recalculate immediately via live WATCH streams (with polling fallback).

The equalizer is a fleet-level optimizer — it adjusts budget *numbers*, never
touches the admission path. If the equalizer is down, each cluster's webhook
continues enforcing its last-known budget independently. The equalizer reuses
the `capacity_admission_webhook` library crate for CRD types (`Allocation`,
`ClusterCapacity`) but is a separate binary with its own CRD, RBAC, and Docker
image.

## Technical Context

**Language/Version**: Rust 1.89 (edition 2024) — same as the existing crate.
The equalizer binary lives in the same Cargo workspace (same `Cargo.toml`),
adding a third `[[bin]]` target.

**Primary Dependencies**: the same crate dependency tree (`kube 4.2.0`,
`k8s-openapi 0.28.0`, `tokio`, `serde`, `tracing`, `prometheus`). No new external
crate is required — kube-rs already handles multi-cluster clients
(`Config::from_custom_kubeconfig`, as proven by `erw-verify/client.rs`). The
equalizer reads kubeconfig bytes from Secrets and constructs per-target
`kube::Client` instances using the same kube-rs API.

**Storage**: N/A — state lives in CRDs: the `EqualizerConfig` CRD (spec = config,
status = observations) and the target clusters' `Allocation` CRDs (spec = budgets
written by the equalizer, status = utilization read by the equalizer). No
external store.

**Testing**: the equalization algorithm is a pure function — fully unit-testable
with the truth-table approach (the worked examples from spec US2 AC1/AC2 become
test cases). Multi-cluster integration tests use `tower-test` mocked apiservers
(one mock per target cluster). BDD via `cucumber-rs` for the equalization
scenarios. E2E via `kind` (two kind clusters for the multi-cluster fixture).

**Target Platform**: Linux container, Kubernetes `Deployment`. Separate Docker
image from the webhook (own `Dockerfile.equalizer` or multi-target build).

**Project Type**: new binary in an existing single-crate project (the library
crate `capacity_admission_webhook` is shared; the binary `capacity-equalizer` is
new, mirroring how `erw-verify` is a second binary).

**Performance Goals**: reconcile cycle latency proportional to N (target cluster
count) — each cycle does N concurrent GETs (parallelized via `tokio::join_all`)
plus N concurrent PATCHes. For a typical fleet of 3–10 clusters, sub-second
reconcile. WATCH streams provide sub-second reactivity between cycles.

**Constraints**: the equalizer MUST NOT modify the target's legacy `budgetPercent`
field (FR-007) — only the per-resource overrides. The equalizer MUST be stateless
(FR-013) — no local cache survives a pod restart. CPU and RAM MUST be equalized
independently (FR-014).

**Scale/Scope**: new feature — new CRD (EqualizerConfig), new binary target
(`src/bin/capacity-equalizer/`), new module (`src/equalizer/`), new deploy
manifests, equalizer-specific RBAC, Dockerfile. Estimated ~40–55 tasks in
`/speckit-tasks`.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Evidence |
|---|-----------|--------|----------|
| I | Fail-Closed by Default (NON-NEGOTIABLE) | ✅ PASS | The equalizer is NOT on the admission critical path. If it is down, each cluster's webhook continues enforcing its last-known budget independently — the fail-closed guarantee is per-cluster, held by the webhook, not by the equalizer. The equalizer adds no new failure mode to the admission path. |
| II | Capacity as a Hard Budget (NON-NEGOTIABLE) | ✅ PASS | The equalizer reinforces the hard-budget principle at the fleet level — it ensures the cumulative budget is enforced across all clusters, not just per-cluster. The budgets it writes are deterministic (the algorithm is pure), not heuristics. |
| III | Explicit Failure Mode Configuration | ✅ PASS | Target-cluster unreachable = skip + report (FR-009). Config error = report + continue (FR-009). Equalizer down = last-known budgets persist (stateless, FR-013). Every failure path is enumerated in the spec (US3, edge cases) and MUST have a corresponding test. |
| IV | Observability Before Optimisation | ✅ PASS | FR-010 (per-cluster status), FR-011 (fleet condition), FR-012 (structured logs). The EqualizerConfig status carries full per-cluster observations + computed budgets + states. |
| V | Separated Concerns, Minimal Surface (NON-NEGOTIABLE) | ✅ PASS (with Complexity Tracking entry) | The equalizer is a **fleet-level optimizer**, architecturally separate from the 3-component per-cluster operator (Node Capacity Controller, Allocation Controller, Admission Webhook). It is a new binary with its own CRD, deployed independently. Coupling it into the webhook binary would conflate a fleet-control-plane component with an admission-critical-path component — different risk profiles, different deployment lifecycles. See Complexity Tracking for the justification. |
| VI | Integration Test Coverage | ✅ PASS | The equalization algorithm is unit-tested (pure function truth table). Multi-cluster integration tests use tower-test mocks. E2E via kind multi-cluster. FR-015 requires erw-verify coverage. |
| VII | Kubernetes Version Support Window (N-2) | ✅ PASS | The EqualizerConfig CRD is `v1`, standard Kubernetes types (Secret, CRD). The kubeconfig-based multi-cluster client construction uses stable kube-rs APIs. No alpha features. CI tests across the N-2 matrix. |
| VIII | Test-First Development (NON-NEGOTIABLE) | ✅ PASS | tasks.md (Phase 2) will order test-before-implementation. The algorithm's pure-function nature makes it ideal for strict TDD. |
| IX | Editor Configuration as Code | ✅ PASS | New files are `.rs`, `.yaml`, `.md`, `.feature` — all covered by `.editorconfig`. |
| X | User-Facing Functionality Documented in README.md | ✅ PASS | The new CRD fields, deployment, kubeconfig Secret setup, and operational behavior MUST be documented in README.md (task in tasks.md). |
| XI | CI-Green Completion Gate | ✅ PASS | CI must test the new binary across the N-2 matrix. The equalizer binary compiles in the same `cargo build`; the E2E suite may gain a multi-cluster kind fixture. |
| XII | Scratch Space for Agent Intercommunication | ✅ PASS | Any transient artifacts go to `.temp/`. |
| XIII | Separation of Usage and Contribution Documentation | ✅ PASS | Equalizer deployment + CRD reference → README.md (operators). Equalizer build/test/dev workflow → CONTRIBUTING.md (contributors). |

**Gate result**: PASS. One Complexity Tracking entry for Principle V (4th component / new binary justified by fleet-vs-admission separation).

## Project Structure

### Documentation (this feature)

```text
specs/013-multi-cluster-capacity-equalizer/
├── spec.md              # /speckit-specify output (committed)
├── checklists/
│   └── requirements.md  # quality checklist (committed)
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   ├── equalizer-config-crd.md  # EqualizerConfig CRD contract
│   └── target-cluster-api.md    # how the equalizer reads/writes target clusters
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT this phase)
```

### Source Code (repository root)

```text
src/
├── equalizer/                       # NEW — the equalization logic
│   ├── mod.rs                       # public re-exports
│   ├── algorithm.rs                 # the pure equalization algorithm (unit-tested)
│   ├── reconcile.rs                 # the reconcile loop (read targets, compute, patch)
│   ├── cluster_client.rs            # build kube::Client per target from kubeconfig Secret
│   └── crd.rs                       # EqualizerConfig CRD definition (kube derive)
├── bin/
│   └── capacity-equalizer/          # NEW — the binary entry point
│       └── main.rs                  # rustls provider install, init tracing, run reconcile loop
├── crd/                             # EXISTING — reused as-is (Allocation, ClusterCapacity)
├── controllers/                     # EXISTING — webhook's controllers (unchanged)
├── webhook/                         # EXISTING — webhook (unchanged)
├── resources/                       # EXISTING — quantity parsing (reused)
├── config.rs                        # EXISTING — webhook config (unchanged)
├── lib.rs                           # EDITED — re-export equalizer module
└── main.rs                          # EXISTING — webhook binary (unchanged)

deploy/
├── equalizer/                       # NEW — equalizer deployment manifests
│   ├── crds.yaml                    # EqualizerConfig CRD
│   ├── rbac.yaml                    # equalizer ServiceAccount + ClusterRole/Binding
│   ├── deployment.yaml              # equalizer Deployment
│   └── equalizer-config.example.yaml # example EqualizerConfig + kubeconfig Secrets
├── crds.yaml                        # EXISTING — webhook CRDs (unchanged)
├── deployment.yaml                  # EXISTING — webhook (unchanged)
├── rbac.yaml                        # EXISTING — webhook (unchanged)
├── webhook-config.yaml              # EXISTING — webhook (unchanged)
└── cert-setup.yaml                  # EXISTING — webhook (unchanged)

Dockerfile                           # EXISTING — webhook image (unchanged)
Dockerfile.equalizer                 # NEW — equalizer image (same pattern, different binary)
Cargo.toml                           # EDITED — add [[bin]] capacity-equalizer + [[test]] entries

tests/
├── equalizer/                       # NEW — equalizer test suite
│   ├── algorithm.rs                 # unit tests for the pure equalization function
│   └── reconcile.rs                 # integration tests (mocked multi-cluster apiserver)
├── bdd/
│   ├── features/equalizer.feature   # NEW — equalization BDD scenarios
│   └── steps/equalizer_steps.rs     # NEW — step definitions
├── integration/                     # EXISTING — webhook tests (unchanged)
└── verify/                          # EXISTING — erw-verify tests (unchanged)

src/bin/erw-verify/scenarios/
└── equalizer.rs                     # NEW — erw-verify multi-cluster equalizer scenario (FR-015)

README.md                            # EDITED — document EqualizerConfig CRD, deployment, kubeconfig setup
CONTRIBUTING.md                      # EDITED — document equalizer build/test workflow
.editorconfig                        # EXISTING — unchanged (no new file type)
```

**Structure Decision**: the equalizer logic lives in `src/equalizer/` (a library
module, unit-testable in isolation), with the binary entry point in
`src/bin/capacity-equalizer/main.rs` (mirrors the `erw-verify` pattern). The
equalizer depends on the existing library crate for CRD types (`Allocation`,
`ClusterCapacity`) and quantity parsing — no duplication. A separate
`Dockerfile.equalizer` builds the equalizer image (same multi-stage pattern as
the webhook Dockerfile, different `--bin` target). Deploy manifests live under
`deploy/equalizer/` to keep them visually separate from the webhook's manifests.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Principle V: 4th component (new binary + CRD) | The equalizer is a fleet-level optimizer operating across cluster boundaries — a fundamentally different concern from the per-cluster admission webhook. It needs its own kubeconfig-based multi-cluster client, its own CRD (EqualizerConfig), and its own deployment lifecycle. | Merging into the webhook binary would couple a fleet-control-plane component (low-risk if down — last-known budgets persist) with an admission-critical-path component (high-risk if down — fail-closed). Different risk profiles + different scaling characteristics + different RBAC scope = separate binaries. This is the same separation rationale as spec-005 (erw-verify as a separate binary to keep rcgen out of the webhook's dependency tree). |

---

## Constitution Check (Post-Design)

*Re-evaluated against the Phase 1 artifacts: `research.md`, `data-model.md`,
`contracts/equalizer-config-crd.md`, `contracts/target-cluster-api.md`,
`quickstart.md`.*

| # | Principle | Status | Evidence from the artifacts |
|---|-----------|--------|-----------------------------|
| I | Fail-Closed (NON-NEGOTIABLE) | ✅ PASS | The equalizer is not on the admission path. `contracts/target-cluster-api.md` §3: it only writes `Allocation.spec.cpuBudgetPercent/memoryBudgetPercent` — the enforcement is done by each target's webhook. `data-model.md` §3 state machine: Unreachable/ConfigError clusters skip patching (last-known budget preserved). If the equalizer is down, each cluster's webhook enforces its last-patched budget independently. |
| II | Capacity as a Hard Budget | ✅ PASS | The algorithm (`data-model.md` §2.2) is deterministic, pure, and unit-testable — no heuristics. Budgets are clamped to [0,100]. The equalizer reinforces the hard budget at fleet level. |
| III | Explicit Failure Modes | ✅ PASS | `data-model.md` §3 state machine + `contracts/target-cluster-api.md` §1.2 error handling: every failure (Secret missing, kubeconfig malformed, API timeout) maps to a ClusterState enum value (Unreachable/ConfigError), recorded in status, reconcile continues. `quickstart.md` V3.1–V3.3 test these paths. |
| IV | Observability | ✅ PASS | `contracts/equalizer-config-crd.md` §3 (full per-cluster status) + `data-model.md` §1.1 status struct (10 fields per cluster observation + fleet condition + timestamp). FR-010/011/012 fully covered by the design. |
| V | Separated Concerns (NON-NEGOTIABLE) | ✅ PASS (justified) | The equalizer is a separate binary with its own CRD, RBAC, and Docker image (`plan.md` Project Structure, `research.md` R8/R9). It reuses the library crate for types only (`research.md` R11). The Complexity Tracking entry above justifies the new component. |
| VI | Integration Test Coverage | ✅ PASS | `quickstart.md`: algorithm unit tests (V1.1, V2.1–V2.5), multi-cluster mocked integration tests (V1.3, V2.6, V3.1–V3.3), BDD (V1.4), E2E (kind multi-cluster). FR-015 erw-verify scenario. |
| VII | K8s N-2 | ✅ PASS | `EqualizerConfig` CRD is `v1` with standard types. Multi-cluster client construction uses stable kube-rs APIs (`Config::from_custom_kubeconfig`, proven in erw-verify). No alpha features. |
| VIII | Test-First (NON-NEGOTIABLE) | ✅ PASS | The algorithm's purity (research R5, data-model §2) makes it ideal for strict TDD — truth-table tests first. `quickstart.md` enumerates the tests. |
| IX | EditorConfig | ✅ PASS | New files: `.rs`, `.yaml`, `.md`, `.feature`, `Dockerfile` — all covered. |
| X | README Documentation | ✅ PASS | Project Structure lists README.md + CONTRIBUTING.md as edited. |
| XI | CI-Green Gate | ✅ PASS | The equalizer binary compiles in the same `cargo build`; CI gains a multi-cluster kind E2E fixture. |
| XII | Scratch Space | ✅ PASS | `.temp/` for transient artifacts. |
| XIII | Usage/Contribution Doc Separation | ✅ PASS | Equalizer CRD + deployment → README; build/test workflow → CONTRIBUTING. |

**Post-design gate result**: PASS. The design is self-consistent across all
artifacts: the algorithm (data-model §2.2), the worked examples (§2.3, 5 cases),
the contract (equalizer-config-crd §2-3, target-cluster-api §1-3), and the
quickstart validation scenarios all agree on field names, budget values (77 not
78 — spec AC2 corrected during the cross-doc consistency pass), ClusterState/FleetCondition
enum values, and the patch semantics (only override fields, never budgetPercent).
