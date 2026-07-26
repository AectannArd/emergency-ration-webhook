# Quickstart: README Documentation

**Feature**: 002-readme-documentation | **Date**: 2026-07-26

This quickstart is the **validation guide** for the README deliverable. It
defines how to verify the README is accurate and complete — it is NOT the
quick start that goes *inside* the README (that is the deliverable itself).
Per Principle VIII (adapted for documentation), this validation spec is
written BEFORE the README, defining what "done" means.

---

## Prerequisites for Validation

- The repository at `main` with all spec-001 implementation present.
- `grep` / `rg` (or the search_files tool) to cross-check values against source.
- A local Kubernetes cluster (`k3d` or `kind`) for the end-to-end quick start
  verification (optional but recommended).

---

## Validation Scenario 1: Quick Start is Runnable (FR-001, SC-001)

**Goal**: An operator can follow the README's quick start section from clone
to a running webhook.

### Steps

1. Read the README quick start section.
2. Follow it on a fresh `k3d`/`kind` cluster.
3. Apply manifests in the documented order.

### Expected

- `kubectl get pods -n capacity-admission` shows 2/2 replicas Ready.
- `kubectl get validatingwebhookconfiguration` shows
  `capacity-admission.emergency-ration.dev`.
- `kubectl port-forward` to the metrics port; `curl /healthz` returns 200.
- A test pod with small requests is admitted; a pod exceeding the budget is
  rejected with a message citing the violated resource.

### Pass criteria

- [ ] All `kubectl apply` commands in the README succeed against the manifests
      in `deploy/`.
- [ ] The TLS section covers both cert-manager and manual Secret paths.
- [ ] The verification steps produce the documented outcomes.

---

## Validation Scenario 2: Configuration Accuracy (FR-002, FR-003, FR-008, FR-009, SC-002, SC-003)

**Goal**: Every documented flag, default, CRD field, and precedence rule
matches the source.

### Steps

1. **Flags**: For each row in the README configuration table, verify against
   `src/config.rs`:
   - Flag name matches a `resolve(args, "--<flag>", ...)` call.
   - Env-var name matches the second arg to `resolve`.
   - Default matches `impl Default for Config`.
   - Type matches the struct field type.

2. **CRD fields**: For each field in the README CRD tables, verify against
   `src/crd/allocation.rs` and `src/crd/cluster_capacity.rs`:
   - Field name matches the Rust field (converted to camelCase via
     `#[serde(rename_all = "camelCase")]`).
   - Type matches.
   - The `budgetPercent` range constraint (0–100) matches
     `#[schemars(range(min = 0, max = 100))]`.

3. **Precedence**: the README states "CLI flag → environment variable →
   compiled default" and notes that unparseable values fall back to default.

4. **Runtime budget adjustment**: the README documents patching
   `spec.budgetPercent` on the Allocation CRD and states no restart is needed.

### Pass criteria

- [ ] All 7 flags verified (VR-001).
- [ ] All CRD fields verified (VR-002).
- [ ] Precedence rule explicitly stated (VR — data-model.md §2).
- [ ] Runtime budget workflow documented (VR — data-model.md §3).

---

## Validation Scenario 3: Metrics & Endpoints Accuracy (FR-004, FR-005, SC-002, SC-004)

**Goal**: Every documented metric name, type, label set, and endpoint matches
the source.

### Steps

1. **Endpoints**: verify the 3-row endpoint table against `src/main.rs`:
   - `/validate` on HTTPS port 8443.
   - `/metrics` on HTTP port 9090.
   - `/healthz` on HTTP port 9090.

2. **Metrics**: for each row in the README metrics table, verify against
   `src/metrics.rs`:
   - Metric name matches the `Opts::new(...)` / `HistogramOpts::new(...)` arg.
   - Type (counter/histogram/gauge) matches the constructor type.
   - Labels match the label slice passed to the `*Vec::new` call.

3. **Scrape test**: `curl localhost:9090/metrics` after deploying; confirm
   all 7 metric families appear with `# HELP` and `# TYPE` lines.

### Pass criteria

- [ ] 3 endpoints verified (VR-004).
- [ ] 7 metrics verified — name, type, labels (VR-003).
- [ ] Histogram buckets documented (.005 through 1.0).

---

## Validation Scenario 4: Failure Modes & Compatibility (FR-006, FR-007, SC-002)

**Goal**: The fail-closed model and version matrix are accurately documented.

### Steps

1. **Failure modes**: verify the 6-row failure table against
   `src/webhook/handler.rs` and `src/webhook/error.rs`. Each documented
   condition must correspond to an actual error path in the handler.

2. **Kubernetes versions**: verify the documented CI matrix against
   `.github/workflows/ci.yml`.

3. **namespaceSelector**: verify the excluded namespace list (`capacity-admission`,
   `kube-system`, `kube-public`) against `deploy/webhook-config.yaml`.

### Pass criteria

- [ ] All 6 failure paths documented and match source (VR-006).
- [ ] CI version matrix matches workflow file (VR-007).
- [ ] namespaceSelector exclusions match webhook-config.yaml (VR-008).

---

## Validation Scenario 5: Single Entry Point (FR-011, SC-005)

**Goal**: The README covers the essentials without delegating the core
operator workflow.

### Steps

1. Read the README top-to-bottom.
2. Confirm an operator can: install, configure, monitor, and troubleshoot
   using ONLY the README.
3. Confirm deeper material (full architecture, spec artifacts) is linked, not
   required for operation.

### Pass criteria

- [ ] Installation, configuration, operations, and troubleshooting are all
      covered in the README body.
- [ ] Links to `specs/001-capacity-admission-webhook/` exist for deeper
      architecture detail but are not required for the operator workflow.
- [ ] No section says "see source code" for essential information.
