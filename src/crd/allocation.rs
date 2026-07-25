//! `Allocation` CRD — aggregated cluster demand and the user-configurable budget
//! threshold (in `spec`), status written by the Allocation Controller.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Singleton instance name enforced by convention (one per cluster).
pub const CLUSTER_ALLOCATION_NAME: &str = "cluster-allocation";

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "emergency-ration.dev",
    version = "v1",
    kind = "Allocation",
    status = "AllocationStatus",
    shortname = "alloc"
)]
// Cluster-scoped: the `namespaced` flag is intentionally omitted (its absence
// means `scope: Cluster`), matching data-model.md §2.
/// Spec of the Allocation CRD. `budget_percent` is the only user-configurable
/// field in the system.
pub struct AllocationSpec {
    /// Maximum allowed allocation as a percentage of total allocatable capacity
    /// (0–100). Applied to both CPU and RAM independently.
    #[schemars(range(min = 0, max = 100))]
    pub budget_percent: i32,
}

/// Status of the Allocation CRD, populated by the Allocation Controller.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllocationStatus {
    /// Currently allocated CPU, in milli-CPUs (sum of pod requests).
    pub allocated_cpu_milli: i64,
    /// Currently allocated memory, in bytes (sum of pod requests).
    pub allocated_memory_bytes: i64,
    /// Budget ceiling for CPU in milli-CPUs
    /// (`floor(totalAllocatableCpuMilli * budgetPercent / 100)`).
    pub ceiling_cpu_milli: i64,
    /// Budget ceiling for memory, in bytes.
    pub ceiling_memory_bytes: i64,
    /// Utilisation ratio for CPU (allocated / ceiling), 0.0–1.0+.
    pub utilization_percent_cpu: f64,
    /// Utilisation ratio for memory.
    pub utilization_percent_memory: f64,
    /// Timestamp of the last allocation recomputation (RFC 3339).
    pub last_updated: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::CustomResourceExt;

    #[test]
    fn crd_is_cluster_scoped_with_expected_name() {
        let crd = Allocation::crd();
        assert_eq!(
            crd.metadata.name.as_deref(),
            Some("allocations.emergency-ration.dev")
        );
        assert_eq!(crd.spec.scope, "Cluster");
        assert_eq!(crd.spec.names.kind, "Allocation");
        let short: Vec<&str> = crd
            .spec
            .names
            .short_names
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        assert_eq!(short, vec!["alloc"]);
        let has_status = crd.spec.versions[0]
            .subresources
            .as_ref()
            .map(|s| s.status.is_some())
            .unwrap_or(false);
        assert!(has_status);
    }

    #[test]
    fn budget_percent_has_range_constraints() {
        let crd = Allocation::crd();
        let v = serde_json::to_value(&crd).unwrap();
        let budget = v
            .pointer(
                "/spec/versions/0/schema/openAPIV3Schema/properties/spec/properties/budgetPercent",
            )
            .expect("budgetPercent schema present");
        assert_eq!(budget.get("minimum").and_then(|m| m.as_f64()), Some(0.0));
        assert_eq!(budget.get("maximum").and_then(|m| m.as_f64()), Some(100.0));
    }

    #[test]
    fn status_serialises_camel_case() {
        let status = AllocationStatus {
            allocated_cpu_milli: 250_000,
            allocated_memory_bytes: 386_547_056_640,
            ceiling_cpu_milli: 256_000,
            ceiling_memory_bytes: 412_316_860_416,
            utilization_percent_cpu: 0.9766,
            utilization_percent_memory: 0.9375,
            last_updated: "2026-07-26T14:32:05Z".to_string(),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("allocatedCpuMilli").is_some());
        assert!(json.get("ceilingMemoryBytes").is_some());
        assert!(json.get("utilizationPercentCpu").is_some());
        assert!(json.get("lastUpdated").is_some());
    }
}
