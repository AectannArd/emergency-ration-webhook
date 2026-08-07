//! Budget calculation — the pure core of the admission decision (T017).
//!
//! Implements the algorithm in `data-model.md` §4. Both resources (CPU in
//! milli-CPUs, memory in bytes) are checked independently; the ceiling is
//! *inclusive* (`projected == ceiling` admits, `projected == ceiling + 1` denies).
//! This module has no I/O and no Kubernetes coupling, so the budget arithmetic is
//! exhaustively unit-testable in isolation.

use crate::webhook::error::{BudgetViolation, ResourceType};

/// A (CPU-milli, memory-bytes) pair — the unit all budget figures move in.
pub type Figures = (i64, i64);

/// Outcome of a budget check: either the pod fits, or it violates one or both
/// resource ceilings (each violation carries the figures for its message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionVerdict {
    /// `allocated + request <= ceiling` for both resources.
    Admit,
    /// At least one resource exceeds its ceiling.
    Deny(Vec<BudgetViolation>),
}

impl AdmissionVerdict {
    /// `true` when the pod fits within the budget.
    pub fn is_admit(&self) -> bool {
        matches!(self, AdmissionVerdict::Admit)
    }
}

/// Compute the budget ceiling for a single resource (spec-012):
/// `floor(total * budget_percent / 100)` with 128-bit intermediates, saturating
/// to i64. Same arithmetic as [`ceiling`], extracted per-resource. Clamp is
/// defensive — the CRD schema already bounds the budget to 0–100, but this is the
/// trust boundary for the figure actually used in decisions.
pub fn ceiling_single(total: i64, budget_percent: i32) -> i64 {
    let budget = budget_percent.clamp(0, 100) as i128;
    let product = total as i128 * budget;
    // floor(total * budget / 100); saturate to i64 defensively so a future caller
    // can never produce a wrapping ceiling.
    ((product / 100).min(i64::MAX as i128)) as i64
}

/// Per-resource ceiling pair (spec-012). Each figure gets its own budget percent,
/// mirroring the independent CPU/RAM resolution in [`resolve_effective_budgets`].
pub fn ceiling_per_resource(total: Figures, budgets: (i32, i32)) -> Figures {
    (
        ceiling_single(total.0, budgets.0),
        ceiling_single(total.1, budgets.1),
    )
}

/// Compute the budget ceiling from cluster supply and a single budget percentage:
/// `floor(total_allocatable * budget_percent / 100)` per resource, applying the
/// SAME budget to both figures.
///
/// Uses 128-bit intermediates so large clusters (memory in the exabyte range)
/// cannot overflow before the floor division. Now a thin delegation to
/// [`ceiling_per_resource`] with the budget repeated for both resources — the
/// arithmetic is byte-identical to the pre-spec-012 body (proven by the
/// `ceiling_per_resource_matches_legacy_ceiling_when_budgets_equal` test, FR-005).
pub fn ceiling(total_allocatable: Figures, budget_percent: i32) -> Figures {
    ceiling_per_resource(total_allocatable, (budget_percent, budget_percent))
}

/// Pure budget check (data-model.md §4). Returns `Admit` iff
/// `allocated + request <= ceiling` for **both** resources (inclusive ceiling);
/// otherwise `Deny` with one `BudgetViolation` per exceeded resource.
pub fn check_budget(
    allocated: Figures,
    pod_request: Figures,
    ceiling: Figures,
) -> AdmissionVerdict {
    // Saturating add so a maliciously huge request cannot panic on overflow; an
    // overflowed projection is guaranteed > ceiling, so it denies correctly.
    let projected_cpu = allocated.0.saturating_add(pod_request.0);
    let projected_mem = allocated.1.saturating_add(pod_request.1);

    let mut violations = Vec::new();
    if projected_cpu > ceiling.0 {
        violations.push(BudgetViolation {
            resource: ResourceType::Cpu,
            allocated: allocated.0,
            requested: pod_request.0,
            projected: projected_cpu,
            ceiling: ceiling.0,
        });
    }
    if projected_mem > ceiling.1 {
        violations.push(BudgetViolation {
            resource: ResourceType::Memory,
            allocated: allocated.1,
            requested: pod_request.1,
            projected: projected_mem,
            ceiling: ceiling.1,
        });
    }

    if violations.is_empty() {
        AdmissionVerdict::Admit
    } else {
        AdmissionVerdict::Deny(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(resource: ResourceType, a: i64, r: i64, p: i64, c: i64) -> BudgetViolation {
        BudgetViolation {
            resource,
            allocated: a,
            requested: r,
            projected: p,
            ceiling: c,
        }
    }

    // ---- check_budget: admit cases ----

    #[test]
    fn admits_when_both_resources_under_ceiling() {
        let verdict = check_budget(
            (70_000, 110 * 1024),
            (5_000, 40 * 1024),
            (80_000, 160 * 1024),
        );
        assert_eq!(verdict, AdmissionVerdict::Admit);
    }

    #[test]
    fn admits_when_projected_equals_ceiling_exactly() {
        // Inclusive ceiling: allocated + request == ceiling → admit.
        let verdict = check_budget((75_000, 0), (5_000, 0), (80_000, 0));
        assert_eq!(verdict, AdmissionVerdict::Admit);
    }

    #[test]
    fn admits_zero_request() {
        let verdict = check_budget((80_000, 160 * 1024), (0, 0), (80_000, 160 * 1024));
        assert_eq!(verdict, AdmissionVerdict::Admit);
    }

    // ---- check_budget: deny cases ----

    #[test]
    fn denies_when_cpu_one_over_ceiling() {
        let verdict = check_budget((75_000, 0), (6_000, 0), (80_000, 0));
        assert_eq!(
            verdict,
            AdmissionVerdict::Deny(vec![violation(
                ResourceType::Cpu,
                75_000,
                6_000,
                81_000,
                80_000
            )])
        );
    }

    #[test]
    fn denies_when_memory_over_ceiling() {
        let verdict = check_budget((0, 150 * 1024), (0, 20 * 1024), (0, 160 * 1024));
        assert_eq!(
            verdict,
            AdmissionVerdict::Deny(vec![violation(
                ResourceType::Memory,
                150 * 1024,
                20 * 1024,
                170 * 1024,
                160 * 1024
            )])
        );
    }

    #[test]
    fn deny_reports_both_resources_independently() {
        // Both over: CPU 85>80, memory 165Gi>160Gi. Both reported (CPU first).
        let verdict = check_budget(
            (70_000, 110 * 1024),
            (15_000, 55 * 1024),
            (80_000, 160 * 1024),
        );
        let AdmissionVerdict::Deny(violations) = verdict else {
            panic!("expected deny");
        };
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].resource, ResourceType::Cpu);
        assert_eq!(violations[1].resource, ResourceType::Memory);
        assert_eq!(violations[0].projected, 85_000);
        assert_eq!(violations[1].projected, 165 * 1024);
    }

    #[test]
    fn deny_cpu_only_reports_cpu_not_memory() {
        // CPU over, memory exactly at ceiling (inclusive) → only CPU reported.
        let verdict = check_budget(
            (75_000, 150 * 1024),
            (6_000, 10 * 1024),
            (80_000, 160 * 1024),
        );
        let AdmissionVerdict::Deny(violations) = verdict else {
            panic!("expected deny");
        };
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].resource, ResourceType::Cpu);
    }

    #[test]
    fn denies_any_positive_request_when_ceiling_is_zero() {
        // budget 0 → ceiling 0 → any >0 request denied.
        let verdict = check_budget((0, 0), (1, 1), (0, 0));
        assert!(matches!(verdict, AdmissionVerdict::Deny(_)));
    }

    // ---- ceiling computation ----

    #[test]
    fn ceiling_floors_total_times_budget_over_100() {
        // 320000m CPU, 80% → 256000m. Memory 480Gi * 80% = 384Gi.
        assert_eq!(ceiling((320_000, 480 * 1024), 80), (256_000, 384 * 1024));
    }

    #[test]
    fn ceiling_uses_floor_not_round() {
        // 999 * 33 / 100 = 329.67 → floor 329.
        assert_eq!(ceiling((999, 999), 33), (329, 329));
    }

    #[test]
    fn ceiling_zero_budget_is_zero() {
        assert_eq!(ceiling((1_000_000, 1_000_000), 0), (0, 0));
    }

    #[test]
    fn ceiling_full_budget_equals_total() {
        assert_eq!(ceiling((123_456, 999_999), 100), (123_456, 999_999));
    }

    #[test]
    fn ceiling_zero_total_is_zero() {
        // Zero nodes → zero capacity → zero ceiling regardless of budget.
        assert_eq!(ceiling((0, 0), 80), (0, 0));
    }

    #[test]
    fn ceiling_handles_large_memory_without_overflow() {
        // ~9 EiB memory must not overflow i64 during the multiply.
        let big = i64::MAX / 2;
        let (cpu, mem) = ceiling((big, big), 50);
        assert_eq!(cpu, big / 2);
        assert_eq!(mem, big / 2);
    }

    // ---- spec-012: per-resource ceiling helper (data-model.md §3.1) ----

    #[test]
    fn ceiling_per_resource_applies_each_figure_its_own_budget() {
        // T003: each figure uses its own budget percent (independent resolution).
        const GIB: i64 = 1024 * 1024 * 1024;
        let supply = (100_000, 200 * GIB);
        // CPU 90% of 100_000 = 90_000; memory 60% of 200 GiB = floor(200GIB*60/100).
        let (cpu, mem) = ceiling_per_resource(supply, (90, 60));
        assert_eq!(cpu, 90_000, "CPU figure uses the CPU budget");
        assert_eq!(
            mem,
            (200 * GIB) * 60 / 100,
            "memory figure uses the memory budget"
        );
        // Sanity: the two figures now differ (asymmetric budgets).
        assert_ne!(cpu, mem);
    }

    #[test]
    fn ceiling_per_resource_matches_legacy_ceiling_when_budgets_equal() {
        // T004: backward-compat equivalence — ceiling_per_resource((t,t),(p,p))
        // == ceiling((t,t),p) for several (t, p). FR-005 / research R3.
        let cases: &[(i64, i32)] = &[
            (320_000, 80),
            (999, 33),
            (123_456, 100),
            (0, 50),
            (1_000_000, 0),
            (i64::MAX / 2, 50),
        ];
        for &(t, p) in cases {
            assert_eq!(
                ceiling_per_resource((t, t), (p, p)),
                ceiling((t, t), p),
                "ceiling_per_resource(({t},{t}),({p},{p})) == ceiling(({t},{t}),{p})"
            );
        }
    }
}
