//! `Allocation` CRD — aggregated cluster demand and the user-configurable budget
//! threshold (in `spec`), status written by the Allocation Controller.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Singleton instance name enforced by convention (one per cluster).
pub const CLUSTER_ALLOCATION_NAME: &str = "cluster-allocation";

/// The enforcement mode of the capacity admission webhook (spec-004).
///
/// `Enforce` (the default, fail-closed behaviour) rejects pods that exceed the
/// budget. `DryRun` admits over-budget pods, surfacing the would-be rejection as
/// an admission warning instead; fail-closed paths still reject in both modes
/// (FR-006 / Constitution Principle I). Serialises kebab-case so the JSON values
/// are `"enforce"` and `"dry-run"` — exactly what operators type in `kubectl patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnforcementMode {
    /// Reject pods that exceed the budget (the default, fail-closed behaviour).
    Enforce,
    /// Admit pods that exceed the budget, surfacing the would-be rejection as an
    /// admission warning. Fail-closed paths still reject.
    DryRun,
}

impl EnforcementMode {
    /// Lower-case label value used in structured logs and metrics fields
    /// (`"enforce"` / `"dry_run"`). Note the log form uses a snake-case
    /// `dry_run` to match the `dry_run_deny` verdict label, distinct from the
    /// kebab-case CRD value `"dry-run"`.
    pub fn as_log_str(self) -> &'static str {
        match self {
            EnforcementMode::Enforce => "enforce",
            EnforcementMode::DryRun => "dry_run",
        }
    }
}

/// Resolve the effective enforcement mode, defaulting to `Enforce` for an absent
/// value (FR-003). The field is optional on the CRD spec, so a pre-feature
/// Allocation instance (no `enforcementMode`) is treated as `enforce`.
pub fn resolve_enforcement_mode(mode: Option<EnforcementMode>) -> EnforcementMode {
    mode.unwrap_or(EnforcementMode::Enforce)
}

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

    /// Enforcement mode: `enforce` (default) or `dry-run` (spec-004). When
    /// absent, the webhook treats the singleton as `enforce` (FR-003) via
    /// [`resolve_enforcement_mode`]. The Allocation Controller seeds
    /// `Some(EnforcementMode::Enforce)` on auto-creation (FR-010) and never
    /// touches the field afterwards — enforcement is a webhook concern.
    pub enforcement_mode: Option<EnforcementMode>,
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

    // ---- spec-004: EnforcementMode enum + resolution helper ----

    #[test]
    fn enforcement_mode_serialises_kebab_case() {
        // T001: serialises as "enforce" and "dry-run" (kebab-case) and round-trips.
        assert_eq!(
            serde_json::to_string(&EnforcementMode::Enforce).unwrap(),
            r#""enforce""#
        );
        assert_eq!(
            serde_json::to_string(&EnforcementMode::DryRun).unwrap(),
            r#""dry-run""#
        );
        let enforce: EnforcementMode = serde_json::from_str(r#""enforce""#).unwrap();
        let dry_run: EnforcementMode = serde_json::from_str(r#""dry-run""#).unwrap();
        assert_eq!(enforce, EnforcementMode::Enforce);
        assert_eq!(dry_run, EnforcementMode::DryRun);
    }

    #[test]
    fn resolve_enforcement_mode_defaults_to_enforce_for_none() {
        // T002: None -> Enforce (FR-003); Some(DryRun) -> DryRun.
        assert_eq!(
            resolve_enforcement_mode(None),
            EnforcementMode::Enforce,
            "absent enforcement mode must resolve to Enforce (FR-003)"
        );
        assert_eq!(
            resolve_enforcement_mode(Some(EnforcementMode::DryRun)),
            EnforcementMode::DryRun
        );
        assert_eq!(
            resolve_enforcement_mode(Some(EnforcementMode::Enforce)),
            EnforcementMode::Enforce
        );
    }

    #[test]
    fn crd_schema_has_optional_enforcement_mode_enum() {
        // T003: enforcementMode is an optional string enum field (not in required).
        let crd = Allocation::crd();
        let v = serde_json::to_value(&crd).unwrap();
        let enforcement = v
            .pointer(
                "/spec/versions/0/schema/openAPIV3Schema/properties/spec/properties/enforcementMode",
            )
            .expect("enforcementMode schema present");
        assert_eq!(
            enforcement.get("type").and_then(|t| t.as_str()),
            Some("string"),
            "enforcementMode is a string-typed field"
        );
        let enum_values = enforcement
            .get("enum")
            .and_then(|e| e.as_array())
            .expect("enforcementMode is an enum");
        let values: Vec<String> = enum_values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(values.contains(&"enforce".to_string()), "{values:?}");
        assert!(values.contains(&"dry-run".to_string()), "{values:?}");
        // Optional: enforcementMode must NOT appear in the spec `required` array.
        let required = v
            .pointer("/spec/versions/0/schema/openAPIV3Schema/properties/spec/required")
            .and_then(|r| r.as_array());
        let lists_enforcement =
            required.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("enforcementMode")));
        assert!(
            !lists_enforcement,
            "enforcementMode must be optional, not required (FR-003)"
        );
    }
}
