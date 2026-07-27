# Clarifications — Session 2026-07-25

> Produced by `/speckit-clarify` ahead of `/speckit-specify`. No spec file
> existed yet (clarify precedes specify in this repo's workflow), so answers
> are captured here for the specify phase to encode into `spec.md`.

## Session 2026-07-25

- Q: When calculating consumed capacity, does the webhook count declared pod
  `resources.requests` or live usage via metrics-server?
  → A: **Declared requests** (pod `resources.requests`). Deterministic, consistent with kube-scheduler, no metrics-server dependency.
- Q: Is the capacity percentage ceiling applied cluster-wide, per-node, or per-resource-pool?
  → A: **Cluster-wide** (Option A). One capacity percentage for total cluster allocatable CPU and RAM. Single budget, simplest correct model. "For now" — per-node/pool partitioning is a deferred future concern, not v1.
- Q: Which admission verbs does the webhook need to gate (CREATE / UPDATE / DELETE)?
  → A: Reframed into a full **3-component operator architecture** (see Architecture
     Vision below). Admission verbs fall out of the component split, not decided
     in isolation.

## Architecture Vision (2026-07-25)

Two independent processes drive capacity; the webhook owns one, not both:

1. **Node lifecycle** — drives the top of the budget (available capacity).
   We **watch** it but do **not interrupt** it (not draining a node for
   maintenance carries heavy risk; that's an operator decision, not the
   webhook's).
2. **Pod lifecycle** — drives consumption. The webhook **controls** this.

Between the two processes, the data link is **CRDs** (shared state). Three
components:

### Component 1 — Node Capacity Controller
- Watches nodes.
- Owns a CRD whose **status** holds the cumulative cluster capacity
  (sum of `.status.allocatable` across all nodes).
- Read-only on nodes; never interrupts node lifecycle.

### Component 2 — Allocation Controller
- Watches the Node Capacity CRD (from Component 1) + resources allocated to
  scheduled pods.
- Calculates current allocation percentage (stored in **status**).
- Holds the **target allocation threshold** in its **spec** (the configurable
  capacity ceiling).
- Tracks pod **CREATE + UPDATE + DELETE** to keep allocation accurate.

### Component 3 — Admission Webhook
- Reads Component 2's CRD **spec** (threshold) + **status** (current allocation)
  for the admission decision.
- Validates a new Pod against the remaining budget.
- Tracks pod **CREATE + UPDATE**.

### Deployment Topology
- **Single binary, three roles.** All three components (Node Capacity Controller,
  Allocation Controller, Admission Webhook) run as async tasks within one
  process, deployed as one `Deployment`. CRDs are the internal data contract.
  Horizontal scaling via stateless replicas. Splitting into separate binaries
  is a future concern if a component needs independent scaling.

Data flow:

```
  nodes ──watch──▶ [Node Capacity Controller] ──status──▶ ClusterCapacity CRD
                                                                      │
  pods ──watch───▶ [Allocation Controller] ──reads──▶ ClusterCapacity CRD
                         │ writes status (allocation %) + reads spec (threshold)
                         ▼
                  Allocation CRD ◀──reads── [Admission Webhook]
                                       │ CREATE+UPDATE on pods
                                       ▼
                                 AdmissionReview response
```

## Session 2026-07-27 (spec-004: dry-run mode)

> Produced by `/speckit-clarify` ahead of `/speckit-specify` for the dry-run
> (audit/shadow) enforcement mode feature.

- Q: How should dry-run mode be toggled?
  → A: **Allocation CRD spec field** (`spec.enforcementMode: enforce | dry-run`).
     Runtime-adjustable via `kubectl patch` — no restart required to switch
     modes. Consistent with how `spec.budgetPercent` already works: the webhook
     reads the Allocation spec from its in-process cache, so a spec change takes
     effect on the next admission decision. No CLI flag or env var for this.

- Q: What should the AdmissionResponse look like when dry-run mode admits a pod
  that WOULD have been rejected?
  → A: **`allowed: true` with the would-be rejection reason surfaced via the
     admission `warnings` field** (available since Kubernetes 1.19). The pod is
     cleanly admitted (no modification to `allowed` or `message`), but the
     operator sees the would-be rejection surfaced as a Warning — visible in
     `kubectl` output (`Warning: ...`) and in cluster events. Structured logs
     and metrics also reflect the dry-run decision so dashboards/alerts can
     track what *would* be blocked. This avoids polluting the rejection
     `message` (which is the contract for real rejections) while still
     surfacing the information at the point of action.

### Design consequences (carried into specify)

- The webhook evaluates every admission request normally (budget check,
  capacity freshness, fail-closed paths) — it just flips the final verdict from
  deny to allow when `enforcementMode == dry-run` and the only reason for
  denial is an over-budget condition.
- **Fail-closed paths stay fail-closed even in dry-run mode.** If capacity data
  is missing/stale, the webhook cannot evaluate the request at all — it rejects
  regardless of `enforcementMode`. Dry-run only converts *over-budget* denials
  to admits; it does not convert *error* rejections to admits. This preserves
  Constitution Principle I: the webhook never admits under degraded knowledge,
  even in audit mode.
- The `enforcementMode` field defaults to `enforce`. The auto-created singleton
  (`cluster-allocation`) includes this default.
- Metrics and structured logging must distinguish a dry-run would-deny from a
  real deny and a real allow, so operators can build dashboards that answer
  "what would dry-run block?" without conflating it with enforced denials.

## Session 2026-07-27 (spec-005: on-demand infrastructure verification)

> Produced by `/speckit-clarify` ahead of `/speckit-specify` for the on-demand
> verification feature — an operator-initiated tool that performs setup → test
> → teardown against a real Kubernetes cluster and prints a report.

- Q: What form should the on-demand verification deliverable take in the repo?
  → A: **Dedicated CLI binary** (Option A) — a second binary from the same
     crate (working name `erw-verify`) that orchestrates setup → test → teardown
     → report. Operator runs it directly with a kubeconfig path/flag. Chosen
     over extending the `cargo test -- --ignored` E2E harness (report output is
     constrained by the test runner) and over a shell script (doesn't carry the
     project's Rust discipline — typed errors, structured output, testability).

- Q: How should the tool isolate its setup/test/teardown footprint on a real
  cluster?
  → A: **Assume a clean/dedicated cluster** (Option A). Install into the default
     `capacity-admission` namespace, full teardown on exit. The caller guarantees
     the cluster is throwaway. This matches CI semantics exactly and keeps the
     tool simple — no namespace-prefixed isolation, no detect-existing-install
     logic. The throwaway guarantee removes the risk of clobbering a production
     install. A future `--namespace` override may extend this, but is out of
     scope for v1.

- Q: How comprehensive should the verification scenario matrix be?
  → A: **Exhaustive, including active fail-closed simulation** (Option C). The
     matrix covers: (a) the constitutional verification suite from Option B —
     budget enforcement (admit/deny + edge cases at 0%/100%), runtime budget
     adjustment without restart, dry-run mode (admit-with-warning), capacity
     tracking accuracy (CRD status vs actual node allocatable), metrics/health
     endpoints; PLUS (b) active fail-closed simulation — killing webhook pods,
     deleting CRDs, inducing stale capacity data — to verify fail-closed paths
     fire on real infrastructure. Safe because the throwaway-cluster model
     (above) makes active degradation non-destructive.

- Q: How should the verification report be printed/output?
  → A: **Human-readable terminal report (default) + optional JSON via `--json`
     flag** (Option B). Rich colorized per-scenario PASS/FAIL output by default;
     `--json` emits structured machine-readable output for CI/tooling
     consumption. Chosen over plain text only (hard to parse) and always-write-
     both-to-disk (adds I/O complexity without clear benefit over stdout+flag).

### Design consequences (carried into specify)

- The deliverable is a **new binary target** in `Cargo.toml` (`[[bin]]
  name = "erw-verify"`), sharing the library crate (`capacity_admission_webhook`)
  so it can reuse CRD types, config parsing, and any shared test fixtures.
- The tool consumes a kubeconfig via the standard `KUBECONFIG` env var or a
  `--kubeconfig` flag (precedence: flag > env > default `~/.kube/config`).
- Setup reuses the existing `deploy/` manifests (crds, rbac, deployment,
  webhook-config, cert-setup), applying them against the target cluster via the
  kube-rs client (not `kubectl`, to keep the single-binary, no-external-deps
  property). TLS provisioning follows the manual Secret path (self-signed cert
  generation in-process), not cert-manager — the tool must not assume
  cert-manager is installed.
- Teardown deletes everything the tool applied, in reverse dependency order,
  with a `--keep-on-failure` escape hatch for debugging (default: always tear
  down, even on failure, so the cluster is left clean).
- The exhaustive scenario matrix (including active degradation) is safe only
  because of the throwaway-cluster guarantee. The tool should document this
  requirement prominently (Principle X) and may refuse to run if it detects a
  non-empty cluster — this safety check is a plan-phase decision.
- The report module is pure (no cluster I/O) so it can be unit-tested in
  isolation (Principle VIII).
- This feature does NOT amend the constitution — it verifies the existing 12
  principles hold on real infrastructure. No new principle is needed.

## Session 2026-07-27 (spec-006: schedulable-node-filter)

> Produced by `/speckit-clarify` ahead of `/speckit-specify` for the node
> exclusion feature — excluding non-schedulable nodes from the cluster capacity
> aggregate so the reported budget matches what kube-scheduler can actually
> place workloads on.

- Q: Which nodes should the operator exclude from the capacity pool, and should
  the exclusion be configurable?
  → A: **Exclude unschedulable nodes by default + provide a configurable label
  selector for arbitrary node-subset exclusion.** This is a two-layer design:
  (1) `spec.unschedulable = true` (cordoned nodes) are always excluded — this is
  a correctness fix, not optional; (2) an optional Kubernetes LabelSelector on
  the ClusterCapacity CRD spec lets operators exclude any arbitrary node subset
  by label (e.g. control-plane nodes via `node-role.kubernetes.io/control-plane:
  Exists`). The label selector is additive: a node is counted only if it is
  schedulable AND does not match the selector.

### Design consequences (carried into specify)

- The exclusion is **not** based on taints/tolerations. Taint matching is the
  kube-scheduler's responsibility (Constitution Principle V — separated
  concerns). A tainted-but-schedulable node with no label-selector match is
  counted. Operators who need to exclude such nodes use the label selector or
  cordon.
- The default unschedulable exclusion **cannot be disabled** — counting
  cordoned nodes would reintroduce the original phantom-capacity bug.
- The label selector follows standard Kubernetes LabelSelector semantics
  (matchLabels + matchExpressions) — no custom dialect, maximum familiarity.
- The selector is optional; absent/empty means "unschedulable-only exclusion"
  (backward compatible with existing deployments).
- The selector is read from the ClusterCapacity CRD spec on each reconciliation
  cycle — runtime-configurable via `kubectl patch`, no restart needed
  (consistent with the Allocation CRD threshold pattern).
- The status gains observability fields: excluded node count + reason
  breakdown (unschedulable vs label-matched), per Principle IV.
- Demand side (Allocation Controller) is unaffected — pods on excluded nodes
  still count against the budget (they consume real resources).
- This feature does NOT amend the constitution — it fixes a correctness gap
  (Principle II: capacity as a hard budget requires accurate supply) and adds
  configurability within the existing 3-component architecture (Principle V).

## Session 2026-07-27 (spec-007: multi-selector-exclusion)

> Produced inline ahead of `/speckit-specify` for the multi-selector node
> exclusion feature. No clarify round was needed — the design fork was
> surfaced during the spec-006 PR review discussion.

- Q: The spec-006 single `LabelSelector` field ANDs all requirements and cannot
  express OR across different label keys. How should multi-criteria exclusion
  work?
  → A: **Multiple selectors, ORed together.** Replace the singular
  `spec.nodeSelector: Option<LabelSelector>` with
  `spec.nodeSelectors: Option<Vec<LabelSelector>>` — a list of selectors where
  a node is excluded if it matches ANY one of them. Each selector internally
  ANDs its own matchLabels/matchExpressions (standard K8s semantics); the OR is
  at the list level. This lets operators exclude control-plane nodes by role
  AND experimental nodes by a custom label without applying a shared label.

### Design consequences (carried into specify)

- Since spec-006 was just merged with no production deployments, a clean field
  rename (`nodeSelector` → `nodeSelectors`) is acceptable — no dual-field
  backward compatibility shim is needed.
- The `node_filter.rs` module is extended: `labels_match_selector` is reused
  per-selector; a new `labels_match_any_selector` wrapper ORs the results.
- A node matching multiple selectors is excluded once (no double-count in
  `excludedBySelector`).
- An invalid selector in the list is logged and skipped — the remaining
  selectors still apply. All-invalid → unschedulable-only fallback (same as
  spec-006 FR-010).
- No new dependencies, no new RBAC.
