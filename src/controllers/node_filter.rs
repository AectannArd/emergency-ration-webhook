//! Node filter (spec-006) — the pure decision logic that determines which nodes
//! count toward the capacity aggregate.
//!
//! Two independent exclusion layers, evaluated in order:
//! 1. **Default (unschedulable)**: a node with `spec.unschedulable = true` is
//!    never counted (FR-001). This cannot be disabled — it fixes the
//!    phantom-capacity bug where cordoned/control-plane nodes inflated the pool.
//! 2. **Selector**: a node matching the optional `ClusterCapacity.spec.nodeSelector`
//!    is not counted (FR-003). An absent or empty selector matches nothing, so
//!    only unschedulable nodes are excluded (FR-005).
//!
//! A node counted toward capacity must pass *both* layers (FR-004). The module is
//! pure — no I/O, no client, no async — so every branch is unit-testable in
//! isolation (Constitution Principle VIII).

use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

/// Breakdown of how the node filter disposed of every node in a reconciliation
/// pass. Returned by [`sum_node_allocatable`](super::node_capacity::sum_node_allocatable)
/// so the controller can populate the `ClusterCapacity` status observability
/// fields (spec-006 US3).
///
/// `excluded_node_count()` is always `excluded_unschedulable + excluded_by_selector`:
/// a node that is both unschedulable and selector-matched is counted under
/// `excluded_unschedulable` only (unschedulable is checked first), never
/// double-counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionBreakdown {
    /// Nodes summed toward the capacity aggregate.
    pub counted: i32,
    /// Nodes excluded because `spec.unschedulable = true` (layer 1).
    pub excluded_unschedulable: i32,
    /// Nodes excluded because they matched `spec.nodeSelector` (layer 2).
    pub excluded_by_selector: i32,
}

impl ExclusionBreakdown {
    /// Total nodes excluded from the aggregate — written to status as
    /// `excludedNodeCount`.
    pub fn excluded_node_count(&self) -> i32 {
        self.excluded_unschedulable + self.excluded_by_selector
    }
}

/// The core predicate: should `node` count toward the capacity aggregate?
///
/// - `unschedulable` — `node.spec.unschedulable.unwrap_or(false)`.
/// - `labels` — `node.metadata.labels`.
/// - `selector` — the optional `ClusterCapacity.spec.nodeSelector`.
///
/// Returns `false` if the node is unschedulable (FR-001) or matches the selector
/// (FR-003); `true` otherwise. A `None`/empty selector disables layer 2 (FR-005).
pub fn is_node_counted(
    unschedulable: bool,
    // `labels` and `selector` wire the label-exclusion path (spec-006 US2); unused
    // until then so the US1 cordon fix can be delivered and validated in isolation.
    _labels: Option<&BTreeMap<String, String>>,
    _selector: Option<&LabelSelector>,
) -> bool {
    // FR-001: unschedulable nodes are always excluded (the default, cannot disable).
    !unschedulable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unschedulable_node_is_not_counted() {
        // FR-001: a cordoned node (spec.unschedulable = true) is always excluded,
        // regardless of selector or labels.
        assert!(!is_node_counted(true, None, None));
    }

    #[test]
    fn schedulable_node_with_no_selector_is_counted() {
        // FR-002: a schedulable node with no selector configured is counted.
        assert!(is_node_counted(false, None, None));
    }
}

