//! The pure equalization algorithm (spec-013, data-model.md §2).
//!
//! [`equalize`] takes the observed fleet state for ONE resource dimension (CPU or
//! RAM) plus a target budget percentage and returns the computed per-cluster
//! budget. It is the most critical component in the feature: pure (no I/O, no
//! async, no panics on any valid input), fully unit-testable via the 5-case truth
//! table in data-model.md §2.3, and reused identically — twice, once per resource
//! — by the reconcile loop (FR-014).

/// Input: one cluster's observed state for ONE resource dimension.
///
/// `total_allocatable` is the cluster's capacity in the dimension's natural units
/// (CPU milli or RAM bytes), read from that cluster's `ClusterCapacity` status.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterResourceObservation {
    pub name: String,
    pub utilization_percent: f64,
    pub total_allocatable: i64, // CPU milli or RAM bytes
}

/// The state of a cluster in the equalization for one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// At or below target; receiving a computed (possibly reduced) budget.
    Good,
    /// Over target; frozen at current utilization.
    Over,
}

/// Output: the computed budget for one cluster + one resource.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedBudget {
    pub name: String,
    pub budget_percent: i32,
    pub state: BudgetState,
}

/// The pure equalization algorithm for a SINGLE resource dimension (CPU or RAM).
///
/// Steps (data-model.md §2.2, FR-005, research R5):
/// 1. Partition: clusters with `utilization_percent > target` are "over"; the
///    rest are "good".
/// 2. Freeze over-clusters at `floor(utilization_percent)`.
/// 3. Compute total absolute overflow = Σ (over: (util% − target) × allocatable
///    / 100), using i128 intermediates so cluster-scale figures cannot overflow.
/// 4. If `good_count == 0`: return all over-clusters frozen (no compensation).
/// 5. Per good-cluster: `budget = target − floor(overflow_abs / good_count /
///    good_cluster_allocatable × 100)`, clamped to [0, 100]. Each good cluster's
///    reduction is computed in absolute units then converted back to percentage
///    points using THAT cluster's own capacity (so a small cluster absorbs the
///    same absolute amount with a larger percentage drop). A zero-capacity good
///    cluster takes no reduction (it has no capacity to absorb overflow).
///
/// The `floor` on good-cluster reductions is conservative (slightly more
/// restrictive), matching the spec's rounding edge case (research R6).
pub fn equalize(
    observations: &[ClusterResourceObservation],
    target_budget_percent: i32,
) -> Vec<ComputedBudget> {
    let target = target_budget_percent as f64;

    let (over, good): (Vec<_>, Vec<_>) = observations
        .iter()
        .cloned()
        .partition(|o| o.utilization_percent > target);

    // Freeze over-clusters at their current utilization (floored + clamped).
    let mut results: Vec<ComputedBudget> = over
        .iter()
        .map(|o| ComputedBudget {
            name: o.name.clone(),
            budget_percent: o.utilization_percent.floor().clamp(0.0, 100.0) as i32,
            state: BudgetState::Over,
        })
        .collect();

    // No good clusters to absorb overflow (US2 AC3 / "all over" edge case).
    if good.is_empty() {
        return results;
    }

    // Total absolute overflow from over-clusters, in the resource's natural units.
    // i128 so the product (percentage × petabyte-scale RAM bytes) cannot overflow.
    let overflow_abs: i128 = over
        .iter()
        .map(|o| {
            let pct_overflow = (o.utilization_percent - target).max(0.0);
            (pct_overflow * o.total_allocatable as f64 / 100.0) as i128
        })
        .sum();

    let good_count = good.len() as i128;

    // Distribute the overflow equally among good clusters (absolute units), then
    // convert each good cluster's share back to percentage points using ITS OWN
    // capacity. Integer division floors (conservative).
    for g in &good {
        let overflow_share_abs = overflow_abs / good_count; // floor
        let pct_reduction = if g.total_allocatable > 0 {
            (overflow_share_abs * 100 / g.total_allocatable as i128) as i32
        } else {
            // Zero-capacity cluster: no capacity to absorb overflow → no reduction.
            0
        };
        let budget = (target_budget_percent - pct_reduction).clamp(0, 100);
        results.push(ComputedBudget {
            name: g.name.clone(),
            budget_percent: budget,
            state: BudgetState::Good,
        });
    }

    results
}
