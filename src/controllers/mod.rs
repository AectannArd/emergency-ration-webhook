//! Cluster controllers: Node Capacity Controller and Allocation Controller.

pub mod allocation;
pub mod node_capacity;

pub use allocation::{build_allocation_status, is_non_terminal, sum_pod_allocation};
pub use node_capacity::sum_node_allocatable;

// Test-only helper: a kube::Client backed by a tower_test mock apiserver, used
// to exercise the controllers' singleton-autocreation logic end-to-end.
#[cfg(test)]
mod mock_api;
