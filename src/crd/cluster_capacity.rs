//! `ClusterCapacity` CRD — aggregated cluster supply (sum of node
//! `.status.allocatable`), written by the Node Capacity Controller.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
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
/// Spec of the ClusterCapacity CRD. Supply-side: the controller sums node
/// `.status.allocatable` into the status. The single optional field,
/// `node_selectors`, is the user-configurable node-exclusion list (spec-007).
pub struct ClusterCapacitySpec {
    /// Optional list of label selectors for excluding nodes from the capacity
    /// aggregate (spec-007). A node matching ANY selector is excluded. Each
    /// selector internally ANDs its matchLabels/matchExpressions (standard K8s
    /// semantics); the list-level result is OR. When absent or empty, only
    /// unschedulable nodes (`spec.unschedulable = true`) are excluded (FR-005).
    pub node_selectors: Option<Vec<LabelSelector>>,
}

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
    // --- spec-006: exclusion observability ---
    /// Total nodes excluded from the aggregate
    /// (`excluded_by_unschedulable + excluded_by_selector`).
    pub excluded_node_count: i32,
    /// Nodes excluded because `spec.unschedulable = true`.
    pub excluded_by_unschedulable: i32,
    /// Nodes excluded because they matched `spec.nodeSelectors`.
    pub excluded_by_selector: i32,
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
        let cc = ClusterCapacity::new(
            CLUSTER_CAPACITY_NAME,
            ClusterCapacitySpec {
                node_selectors: None,
            },
        );
        assert_eq!(cc.metadata.name.as_deref(), Some(CLUSTER_CAPACITY_NAME));
    }

    #[test]
    fn status_serialises_camel_case() {
        let status = ClusterCapacityStatus {
            total_allocatable_cpu_milli: 320_000,
            total_allocatable_memory_bytes: 515_396_075_520,
            node_count: 12,
            last_updated: "2026-07-26T14:32:01Z".to_string(),
            excluded_node_count: 0,
            excluded_by_unschedulable: 0,
            excluded_by_selector: 0,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("totalAllocatableCpuMilli").is_some());
        assert!(json.get("totalAllocatableMemoryBytes").is_some());
        assert!(json.get("nodeCount").is_some());
        assert!(json.get("lastUpdated").is_some());
    }

    // ---- spec-007: node_selectors (array) + exclusion observability fields ----

    #[test]
    fn node_selectors_field_serialises_camel_case_and_round_trips() {
        use super::LabelSelector;
        use std::collections::BTreeMap;

        // A spec carrying a list of one selector that excludes control-plane nodes.
        let mut match_labels = BTreeMap::new();
        match_labels.insert(
            "node-role.kubernetes.io/control-plane".to_string(),
            String::new(),
        );
        let spec = ClusterCapacitySpec {
            node_selectors: Some(vec![LabelSelector {
                match_labels: Some(match_labels),
                match_expressions: None,
            }]),
        };
        let json = serde_json::to_value(&spec).unwrap();
        let node_selectors = json
            .get("nodeSelectors")
            .expect("field must serialise as camelCase `nodeSelectors`: {json}");
        assert!(
            node_selectors.is_array(),
            "nodeSelectors must serialise as an array: {json}"
        );
        assert_eq!(
            node_selectors.as_array().unwrap().len(),
            1,
            "one selector in the list"
        );
        // Round-trips back through serde.
        let back: ClusterCapacitySpec = serde_json::from_value(json).unwrap();
        assert!(
            back.node_selectors.is_some(),
            "nodeSelectors round-trips through serde"
        );
    }

    #[test]
    fn node_selectors_defaults_to_none_when_absent() {
        // An existing instance without nodeSelectors deserialises to None
        // (the field is optional, FR-005).
        let spec: ClusterCapacitySpec = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(spec.node_selectors.is_none());
    }

    #[test]
    fn status_exclusion_fields_serialise_camel_case_and_round_trip() {
        let status = ClusterCapacityStatus {
            total_allocatable_cpu_milli: 320_000,
            total_allocatable_memory_bytes: 515_396_075_520,
            node_count: 10,
            last_updated: "2026-07-26T14:32:01Z".to_string(),
            excluded_node_count: 2,
            excluded_by_unschedulable: 1,
            excluded_by_selector: 1,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json.get("excludedNodeCount").and_then(|v| v.as_i64()),
            Some(2)
        );
        assert_eq!(
            json.get("excludedByUnschedulable").and_then(|v| v.as_i64()),
            Some(1)
        );
        assert_eq!(
            json.get("excludedBySelector").and_then(|v| v.as_i64()),
            Some(1)
        );
        let back: ClusterCapacityStatus = serde_json::from_value(json).unwrap();
        assert_eq!(back.excluded_node_count, 2);
        assert_eq!(back.excluded_by_unschedulable, 1);
        assert_eq!(back.excluded_by_selector, 1);
    }

    #[test]
    fn crd_schema_includes_node_selectors_under_spec() {
        let crd = ClusterCapacity::crd();
        let v = serde_json::to_value(&crd).unwrap();
        let node_selectors = v
            .pointer(
                "/spec/versions/0/schema/openAPIV3Schema/properties/spec/properties/nodeSelectors",
            )
            .expect("nodeSelectors schema present under spec");
        assert_eq!(
            node_selectors.get("type").and_then(|t| t.as_str()),
            Some("array"),
            "nodeSelectors is an array-typed field"
        );
        // nodeSelectors must NOT be in the spec `required` array (optional, FR-005).
        let required =
            v.pointer("/spec/versions/0/schema/openAPIV3Schema/properties/spec/required");
        let lists_selector = required.is_some_and(|arr| {
            arr.as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("nodeSelectors")))
        });
        assert!(
            !lists_selector,
            "nodeSelectors must be optional, not required (FR-005)"
        );
    }
}
