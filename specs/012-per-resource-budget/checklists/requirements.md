# Quality Checklist — Per-Resource Budget Tracking (spec-012)

*Generated from `.specify/templates/` checklist format. Validated against
`specs/012-per-resource-budget/spec.md`.*

## Content Quality

- [x] **All sections filled** — User Scenarios, Edge Cases, Requirements, Key
      Entities, Success Criteria, Assumptions all populated. No `[PLACEHOLDER]`
      tokens remain.
- [x] **User stories are independently testable** — US1 (asymmetric budgets)
      is a standalone MVP slice; US2 (backward compat) certifies no regression;
      US3 (observability) completes the debuggability surface. Each can be
      verified without the others.
- [x] **Stories prioritised** — P1 = the core capability (independent budgets),
      P2 = the safety net (backward compat, mandatory before merge), P3 =
      observability (Constitution Principle IV, the natural completion).
- [x] **Acceptance scenarios are Given/When/Then** — every scenario follows the
      Gherkin shape and references concrete field names and values.

## Requirement Completeness

- [x] **Every FR is testable** — FR-001 (CRD fields), FR-002 (resolution),
      FR-003 (ceiling computation), FR-004 (enforcement), FR-005 (backward
      compat), FR-006 (budgetPercent stays required), FR-007 (controller
      non-modification), FR-008 (auto-creation default), FR-009 (status
      exposure), FR-010 (log exposure), FR-011 (per-resource violation
      reporting), FR-012 (erw-verify scenario) — each has a verifiable outcome.
- [x] **No NEEDS CLARIFICATION markers** — the clarify phase (inline, in the
      interrupted session that pivoted to this spec) resolved the only fork
      (separate limits: confirmed). Every FR is concrete.
- [x] **FRs map to user stories** — FR-001..004, 006..008, 011 → US1; FR-005,
      006, 008 → US2; FR-009, 010 → US3; FR-012 → US1 verification. No orphan
      FRs, no uncovered story acceptance criteria.
- [x] **Backward compatibility is explicit** — FR-005 (byte-identical ceilings)
      and FR-006 (budgetPercent remains required) are the hard guarantees. US2
      exists specifically to certify them.

## Feature Readiness

- [x] **Edge cases enumerated** — both overrides absent, both present, one
      present, override equals budgetPercent, 0% override, 100% override, single
      resource overridden (CPU-only then memory-only symmetric), all-three
      consistent, negative (schema-rejected), exemption interaction, dry-run
      interaction. The boundary surface is covered.
- [x] **No tech-stack leakage** — spec references CRD fields, ceilings, the
      webhook, the Allocation Controller, and `erw-verify` (product vocabulary).
      No Rust crate names, no `kube-rs`, no struct definitions — those belong in
      the plan.
- [x] **Constitution alignment documented** — Assumptions states the feature
      does NOT amend the constitution and maps to Principles II, V, I/III, IV,
      VI, VIII. The plan-phase Constitution Check will re-verify this.
- [x] **Scope is bounded** — SC-004 commits to a single backward-compatible
      change, no new components/CLI/RBAC. The blast radius is confined to the
      Allocation CRD types + controller ceiling computation + log/status fields.
- [x] **Key Entities correct** — the only entity is the extended Allocation CRD;
      no new CRD, no new singleton, apiVersion stays `v1`.

## Cross-Check Against Existing System

- [x] **Existing `check_budget` is already per-resource** — verified against
      `src/webhook/admission.rs`: it reports CPU and memory violations
      independently. FR-004 documents that this feature requires no enforcement-
      path change; the coupling removed is only at budget resolution.
- [x] **Existing `ceiling()` is reusable** — the 128-bit-overflow-guarded
      arithmetic applies unchanged per resource; no new overflow protection
      needed (Assumptions).
- [x] **Existing Allocation status schema is additive** — `ceiling_cpu_milli` /
      `ceiling_memory_bytes` already exist; FR-009 adds
      `effectiveCpuBudgetPercent` / `effectiveMemoryBudgetPercent` as new
      computed fields alongside them.
- [x] **Auto-creation path (spec-003) is respected** — FR-007/008 require the
      controller to seed overrides as absent and never infer them, matching the
      existing `budget_percent = DEFAULT_BUDGET_PERCENT` seeding pattern.
