# Validation Results — Phases 4–6 (T024–T044)

Run against the quickstart scenarios (`specs/001-capacity-admission-webhook/quickstart.md`)
on 2026-07-26. Each integration / BDD scenario passes; deviations from the
quickstart's literal commands are listed below.

## Quality gate (T044)

| Check | Command | Result |
|-------|---------|--------|
| Format | `cargo fmt --check` | clean |
| Clippy | `cargo clippy --all-targets -- -D warnings` | clean |
| Unit + integration + BDD | `cargo test` | **all green** (see breakdown) |
| `#[ignore]` excluded by default | `cargo test` reports `performance … 1 ignored` (not run) | confirmed |

Test breakdown (default `cargo test`):

- Unit (`src/lib.rs`): **96 passed**
- `budget_enforcement`: 6 · `capacity_awareness`: 4 · `fail_safe`: 8 (integration)
- BDD: budget **6 scenarios / 33 steps**, capacity **4 scenarios / 33 steps**,
  fail-safe **5 scenarios / 29 steps**
- `performance`: 1 ignored (benchmark)

## Scenario results

### Scenario 1 — Budget Enforcement (US1)

`cargo test --test budget_enforcement` and `cargo test --test budget_bdd`: all
spec US1 acceptance scenarios (1–5) pass — admit under ceiling, deny over with
exact figures (SC-002), inclusive ceiling, zero-request admit, update-as-delta.

### Scenario 2 — Capacity Awareness (US2)

`cargo test --test capacity_awareness` and `cargo test --test capacity_awareness_bdd`:
every decision is observable — structured log entries carry every Logging
Contract field, denial messages carry actionable figures, and the metrics surface
exposes verdict counters + capacity gauges (SC-003). The E2E metrics scrape
(`kubectl port-forward … 9090:metrics` → `curl http://localhost:9090/metrics`) is
exercised by the CI E2E job against the k8s matrix.

### Scenario 3 — Fail-Safe Operation (US3)

`cargo test --test fail_safe` and `cargo test --test fail_safe_bdd`: every
failure path returns `allowed: false` — stale data, missing allocation, missing
ClusterCapacity, malformed AdmissionReview, unparseable quantity, decision
timeout, unknown error (SC-004).

### Scenario 4 — Performance (SC-005/SC-006)

`cargo test --test performance -- --ignored --nocapture`: over 10 000 iterations,
**p50 ≈ 0.11 ms, p99 ≈ 0.18 ms** — well under the 100 ms / 50 ms targets (the hot
path is an in-memory read + budget arithmetic). Footprint requests (< 256 Mi /
< 500m, SC-006) are declared in `deploy/deployment.yaml`.

## Deviations from the quickstart commands

1. **Test target names.** The quickstart uses `cargo test --test integration …`
   and `cargo test --test bdd …`, assuming one binary per kind. This repo ships
   one target per story: run `cargo test --test budget_enforcement`,
   `--test capacity_awareness`, `--test fail_safe`, and their `*_bdd`
   counterparts (`cargo test --test budget_bdd`, etc.).
2. **Performance command.** The benchmark is `#[ignore]`d so it stays out of the
   default gate (T044); run it with
   `cargo test --test performance -- --ignored --nocapture`.
3. **Metrics endpoint.** `/metrics` is served on a dedicated plaintext HTTP port
   (`--metrics-port`, default **9090**) in addition to the HTTPS webhook port
   (8443), so Prometheus can scrape and kubelet can probe `/healthz` without TLS.
   This matches the quickstart's `9090:metrics` port-forward and `curl http://`
   scrape.
