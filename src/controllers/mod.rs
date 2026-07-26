//! Cluster controllers: Node Capacity Controller and Allocation Controller.

use serde::Serialize;

pub mod allocation;
pub mod node_capacity;

pub use allocation::{build_allocation_status, is_non_terminal, sum_pod_allocation};
pub use node_capacity::sum_node_allocatable;

/// Wrap a status value in the `{"status": ...}` envelope a `/status`-subresource
/// merge patch requires.
///
/// `Api::patch_status(.., &Patch::Merge(status))` sends `status` verbatim as the
/// merge-patch body. On the `/status` subresource the apiserver applies that body
/// to the whole object and then copies **only** the resulting `.status` — so a
/// bare status object (no top-level `status` key) matches nothing and is a silent
/// no-op: the patch returns `200` and nothing is persisted. Both controllers must
/// therefore wrap their status under `"status"`.
pub(crate) fn status_merge_patch<T: Serialize>(status: &T) -> serde_json::Value {
    serde_json::json!({ "status": status })
}

// Test-only helper: a kube::Client backed by a tower_test mock apiserver, used
// to exercise the controllers' singleton-autocreation logic end-to-end.
#[cfg(test)]
mod mock_api;

#[cfg(test)]
mod tests {
    use super::status_merge_patch;

    #[test]
    fn status_merge_patch_envelopes_under_status_key() {
        // Regression guard: a bare status object is a silent no-op on the
        // `/status` subresource; the body must carry a top-level `status` key
        // and nothing else at the top level.
        let inner = serde_json::json!({ "totalAllocatableCpuMilli": 16000, "nodeCount": 1 });
        let envelope = status_merge_patch(&inner);
        assert_eq!(envelope["status"]["totalAllocatableCpuMilli"], 16_000);
        assert_eq!(envelope["status"]["nodeCount"], 1);
        assert_eq!(
            envelope.as_object().unwrap().len(),
            1,
            "only the `status` key sits at the top level of a status merge patch"
        );
    }
}
