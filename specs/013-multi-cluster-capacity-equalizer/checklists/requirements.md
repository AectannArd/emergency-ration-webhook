# Quality Checklist — Multi-Cluster Capacity Equalizer (spec-013)

*Generated from `.specify/templates/` checklist format. Validated against
`specs/013-multi-cluster-capacity-equalizer/spec.md`.*

## Content Quality

- [x] **All sections filled** — User Scenarios (3 stories), Edge Cases (13),
      Requirements (15 FRs), Key Entities (4), Success Criteria (5),
      Assumptions (6). No `[PLACEHOLDER]` tokens remain.
- [x] **User stories are independently testable** — US1 (all-under-target baseline
      loop) is a standalone MVP; US2 (overflow compensation) adds the core
      algorithm; US3 (reachability/status) adds operational resilience. Each
      verifiable without the others.
- [x] **Stories prioritised** — P1 = baseline read→compute→patch loop; P2 = the
      equalization algorithm (reason the feature exists); P3 = production
      resilience (unreachable targets).
- [x] **Acceptance scenarios are Given/When/Then** — every scenario references
      concrete percentages, cluster counts, and field names.

## Requirement Completeness

- [x] **Every FR is testable** — FR-001 (separate binary), FR-002 (CRD),
      FR-003 (target cluster defs), FR-004 (read both CRDs), FR-005 (algorithm),
      FR-006 (patch overrides), FR-007 (don't touch budgetPercent), FR-008
      (hybrid poll+watch), FR-009 (skip unreachable), FR-010 (per-cluster
      status), FR-011 (fleet condition), FR-012 (structured logs), FR-013
      (stateless), FR-014 (independent CPU/RAM), FR-015 (erw-verify scenario).
- [x] **No NEEDS CLARIFICATION markers** — 5 clarifications resolved (C1: N−1
      divisor in absolute units; C2: per-resource independent; C3: separate
      image; C4: hybrid poll+watch; C5: all clusters via kubeconfig Secret
      including home).
- [x] **FRs map to user stories** — FR-001..004, 006..008, 013 → US1; FR-005,
      014 → US2; FR-009..011 → US3; FR-012, 015 cross-cutting. No orphan FRs.
- [x] **Algorithm is precisely specified** — FR-005 gives the 4-step algorithm
      (identify over-clusters, compute absolute overflow, distribute among good,
      freeze-all edge case). The worked examples in US2 AC1/AC2 verify the math.

## Feature Readiness

- [x] **Edge cases enumerated** — all-over, single-cluster, zero-capacity,
      cluster join/remove, kubeconfig update, multiple over-clusters, over→good
      transition, rounding (floor), restart, runtime config update, webhook
      sovereignty, home cluster identity. 13 cases covering the boundary surface.
- [x] **No tech-stack leakage** — spec references CRDs, Allocation/ClusterCapacity
      status, kubeconfig Secrets, kind clusters (product vocabulary). No Rust
      crate names, no kube-rs API names — those belong in the plan.
- [x] **Constitution alignment documented** — Assumptions states the feature
      does NOT amend the constitution and maps to Principle V (new component
      alongside existing architecture, separate binary per separated concerns),
      Principle I (equalizer down = per-cluster webhook unaffected). The
      plan-phase Constitution Check will re-verify this, especially Principle V
      (is a 4th component justified?) and Principle VII (new CRD on N-2 matrix).
- [x] **Scope is bounded** — SC-005 commits to a separate binary + image. The
      blast radius is: new CRD (EqualizerConfig), new binary target
      (capacity-equalizer), new Dockerfile, new deploy manifests, erw-verify
      extension. The existing webhook binary and CRDs are unchanged.
- [x] **Key Entities correct** — EqualizerConfig (new), Allocation (consumed,
      spec-012 override fields written), ClusterCapacity (consumed read-only),
      Kubeconfig Secrets (standard K8s Secret type).

## Cross-Check Against Existing System

- [x] **Spec-012 prerequisite confirmed** — `Allocation.spec.cpuBudgetPercent` /
      `memoryBudgetPercent` exist in the merged code (verified at
      `src/crd/allocation.rs:110-114`). The equalizer writes these fields.
- [x] **Allocation status fields the equalizer reads** —
      `utilizationPercentCpu/Memory`, `totalAllocatableCpuMilli/MemoryBytes`
      (via ClusterCapacity) all exist in the current CRD status structs.
- [x] **Singleton names** — `cluster-allocation` / `cluster-capacity` are the
      existing convention (verified in `src/crd/allocation.rs:9` and
      `src/crd/cluster_capacity.rs:10`). The equalizer reads these by name.
- [x] **Library reuse** — the `capacity_admission_webhook` library crate exposes
      the CRD types; the new binary can depend on them (same pattern as
      `erw-verify`, spec-005).
- [x] **Constitution Principle V compatibility** — the equalizer is a FLEET-level
      optimizer, not a per-cluster admission component. Adding it as a separate
      binary (not merged into the webhook) respects the separated-concerns
      principle. The plan-phase check must justify the 4th component in the
      Complexity Tracking table (or argue it is outside the 3-component per-
      cluster architecture and thus not a violation).
