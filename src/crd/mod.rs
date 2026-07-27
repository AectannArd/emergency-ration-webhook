//! Custom resource definitions (CRDs) used as shared state between the three
//! components.

pub mod allocation;
pub mod cluster_capacity;

pub use allocation::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, EnforcementMode,
    ExemptionReason, check_exemption, resolve_enforcement_mode,
};
pub use cluster_capacity::{
    CLUSTER_CAPACITY_NAME, ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
