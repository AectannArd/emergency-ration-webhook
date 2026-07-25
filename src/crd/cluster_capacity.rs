//! `ClusterCapacity` CRD — aggregated cluster supply (sum of node
//! `.status.allocatable`), written by the Node Capacity Controller.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Singleton instance name enforced by convention (one per cluster).
pub const CLUSTER_CAPACITY_NAME: &str = "cluster-capacity";

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "emergency-ration.dev",
    version = "v1",
    kind = "ClusterCapacity",
    status = "ClusterCapacityStatus",
    shortname = "cc"
)]
// Cluster-scoped: the `namespaced` flag is intentionally omitted (its absence
// means `scope: Cluster`), matching data-model.md §1.
/// Spec of the ClusterCapacity CRD. Supply-side and controller-written, so it
/// carries no user-configurable fields.
pub struct ClusterCapacitySpec {}

/// Status of the ClusterCapacity CRD, populated by the Node Capacity Controller.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCapacityStatus {
    /// Total allocatable CPU across all nodes, in milli-CPUs.
    pub total_allocatable_cpu_milli: i64,
    /// Total allocatable memory across all nodes, in bytes.
    pub total_allocatable_memory_bytes: i64,
    /// Number of nodes counted.
    pub node_count: i32,
    /// Timestamp of the last capacity recomputation (RFC 3339).
    pub last_updated: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::CustomResourceExt;

    #[test]
    fn crd_is_cluster_scoped_with_expected_name() {
        let crd = ClusterCapacity::crd();
        assert_eq!(
            crd.metadata.name.as_deref(),
            Some("clustercapacities.emergency-ration.dev")
        );
        assert_eq!(crd.spec.scope, "Cluster");
        assert_eq!(crd.spec.names.kind, "ClusterCapacity");
        let short: Vec<&str> = crd
            .spec
            .names
            .short_names
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        assert_eq!(short, vec!["cc"]);
        // Status subresource is declared.
        let has_status = crd.spec.versions[0]
            .subresources
            .as_ref()
            .map(|s| s.status.is_some())
            .unwrap_or(false);
        assert!(has_status);
    }

    #[test]
    fn singleton_constructs_with_empty_spec() {
        let cc = ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {});
        assert_eq!(cc.metadata.name.as_deref(), Some(CLUSTER_CAPACITY_NAME));
    }

    #[test]
    fn status_serialises_camel_case() {
        let status = ClusterCapacityStatus {
            total_allocatable_cpu_milli: 320_000,
            total_allocatable_memory_bytes: 515_396_075_520,
            node_count: 12,
            last_updated: "2026-07-26T14:32:01Z".to_string(),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("totalAllocatableCpuMilli").is_some());
        assert!(json.get("totalAllocatableMemoryBytes").is_some());
        assert!(json.get("nodeCount").is_some());
        assert!(json.get("lastUpdated").is_some());
    }
}
