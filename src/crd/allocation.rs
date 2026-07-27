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

    /// Optional list of namespace names whose pods are exempt from capacity
    /// admission (spec-008, FR-001). A pod whose namespace matches any entry is
    /// admitted without a budget check. Absent/empty → no namespace exclusions
    /// (backward-compatible, FR-004). JSON field: `excludedNamespaces`.
    pub excluded_namespaces: Option<Vec<String>>,

    /// Optional list of priority class names whose pods are exempt from capacity
    /// admission (spec-008, FR-002). Matched as a string against
    /// `pod.spec.priorityClassName` (no PriorityClass resource resolution, R3).
    /// Absent/empty → no priority class exclusions (FR-004). JSON field:
    /// `excludedPriorityClasses`.
    pub excluded_priority_classes: Option<Vec<String>>,
}

/// The criterion that triggered an admission exemption, for observability
/// (spec-008, R6). Recorded in structured logs and as the `reason` label on
/// `capacity_admission_exemptions_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExemptionReason {
    /// Pod namespace is in `excludedNamespaces` (FR-001).
    Namespace,
    /// Pod `priorityClassName` is in `excludedPriorityClasses` (FR-002).
    PriorityClass,
    /// Pod is in the webhook's own namespace (FR-007 bootstrap fallback).
    WebhookNamespace,
}

impl ExemptionReason {
    /// Lower-case label value used in the exemption counter's `reason` label and
    /// the structured log's `exemption_reason` field. Matches the contract values
    /// `namespace` / `priority_class` / `webhook_namespace`.
    pub fn as_str(self) -> &'static str {
        match self {
            ExemptionReason::Namespace => "namespace",
            ExemptionReason::PriorityClass => "priority_class",
            ExemptionReason::WebhookNamespace => "webhook_namespace",
        }
    }
}

/// Whether a string appears in an optional exclusion list. `None` and an empty
/// list both match nothing (FR-004).
fn list_contains(list: Option<&Vec<String>>, value: &str) -> bool {
    list.is_some_and(|entries| entries.iter().any(|entry| entry == value))
}

/// Check whether a pod is exempt from capacity admission (spec-008). Returns
/// `Some(reason)` if the pod skips the budget check, `None` if it is subject to
/// it.
///
/// Order (data-model §3.2; first match wins, subsequent checks skipped):
/// 1. Webhook's own namespace (FR-007) — the webhook never self-gates once the
///    Allocation is cached.
/// 2. `excludedNamespaces` (FR-001).
/// 3. `excludedPriorityClasses` (FR-002) — string match only; an absent or
///    empty-string priority class never matches (US2 AC4).
///
/// OR semantics: matching either list exempts the pod (FR-003). Duplicate list
/// entries are harmless (`Vec` containment is idempotent).
pub fn check_exemption(
    pod_namespace: Option<&str>,
    pod_priority_class: Option<&str>,
    spec: &AllocationSpec,
    webhook_namespace: &str,
) -> Option<ExemptionReason> {
    if pod_namespace == Some(webhook_namespace) {
        return Some(ExemptionReason::WebhookNamespace);
    }
    if let Some(ns) = pod_namespace
        && list_contains(spec.excluded_namespaces.as_ref(), ns)
    {
        return Some(ExemptionReason::Namespace);
    }
    if let Some(pc) = pod_priority_class
        && !pc.is_empty()
        && list_contains(spec.excluded_priority_classes.as_ref(), pc)
    {
        // An absent or empty priority class never reaches here (US2 AC4).
        return Some(ExemptionReason::PriorityClass);
    }
    None
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

    // ---- spec-008: excludedNamespaces + excludedPriorityClasses ----

    #[test]
    fn allocation_spec_exclusion_fields_round_trip_camel_case() {
        // T001: the two new fields serialise camelCase and round-trip.
        let spec = AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
            excluded_namespaces: Some(vec!["kube-system".to_string(), "monitoring".to_string()]),
            excluded_priority_classes: Some(vec!["system-node-critical".to_string()]),
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            json.get("excludedNamespaces")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(2),
            "excludedNamespaces serialises as a camelCase array: {json}"
        );
        assert_eq!(
            json.get("excludedNamespaces")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str()),
            Some("kube-system")
        );
        assert_eq!(
            json.get("excludedPriorityClasses")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1),
            "excludedPriorityClasses serialises as a camelCase array: {json}"
        );
        let back: AllocationSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back.excluded_namespaces, spec.excluded_namespaces);
        assert_eq!(
            back.excluded_priority_classes,
            spec.excluded_priority_classes
        );
    }

    #[test]
    fn allocation_spec_absent_exclusion_fields_default_to_none() {
        // T001: a pre-spec-008 Allocation (fields absent) deserialises with both
        // new fields as None — backward-compatible default (FR-004).
        let pre_008 = serde_json::json!({
            "budgetPercent": 80,
            "enforcementMode": "enforce",
        });
        let spec: AllocationSpec = serde_json::from_value(pre_008).unwrap();
        assert_eq!(spec.budget_percent, 80);
        assert_eq!(spec.enforcement_mode, Some(EnforcementMode::Enforce));
        assert!(
            spec.excluded_namespaces.is_none(),
            "absent excludedNamespaces must default to None (FR-004)"
        );
        assert!(
            spec.excluded_priority_classes.is_none(),
            "absent excludedPriorityClasses must default to None (FR-004)"
        );
    }

    #[test]
    fn crd_schema_has_optional_exclusion_fields() {
        // T001: both exclusion fields are optional string arrays (nullable) and
        // neither is in the spec `required` array (FR-004).
        let crd = Allocation::crd();
        let v = serde_json::to_value(&crd).unwrap();
        let spec_props = v
            .pointer("/spec/versions/0/schema/openAPIV3Schema/properties/spec/properties")
            .expect("spec properties present");
        for field in ["excludedNamespaces", "excludedPriorityClasses"] {
            let schema = spec_props
                .get(field)
                .unwrap_or_else(|| panic!("{field} schema present"));
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("array"),
                "{field} is an array-typed field"
            );
            let items = schema
                .get("items")
                .unwrap_or_else(|| panic!("{field} has items"));
            assert_eq!(
                items.get("type").and_then(|t| t.as_str()),
                Some("string"),
                "{field} items are strings"
            );
        }
        // Neither field may be required.
        let required = v
            .pointer("/spec/versions/0/schema/openAPIV3Schema/properties/spec/required")
            .and_then(|r| r.as_array());
        for field in ["excludedNamespaces", "excludedPriorityClasses"] {
            let listed = required.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(field)));
            assert!(!listed, "{field} must be optional, not required (FR-004)");
        }
    }

    // ---- spec-008: ExemptionReason + check_exemption (data-model §3.2 / §8) ----

    /// Build a spec carrying only the two exclusion lists (budget/mode irrelevant
    /// to exemption logic).
    fn spec_with(
        excluded_namespaces: Option<Vec<&str>>,
        excluded_priority_classes: Option<Vec<&str>>,
    ) -> AllocationSpec {
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
            excluded_namespaces: excluded_namespaces
                .map(|v| v.into_iter().map(String::from).collect()),
            excluded_priority_classes: excluded_priority_classes
                .map(|v| v.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn exemption_reason_as_str_matches_metric_and_log_labels() {
        assert_eq!(ExemptionReason::Namespace.as_str(), "namespace");
        assert_eq!(ExemptionReason::PriorityClass.as_str(), "priority_class");
        assert_eq!(
            ExemptionReason::WebhookNamespace.as_str(),
            "webhook_namespace"
        );
    }

    #[test]
    fn check_exemption_webhook_namespace_match() {
        // FR-007: a pod in the webhook's own namespace is exempt even with both
        // exclusion lists empty/absent.
        let spec = spec_with(None, None);
        assert_eq!(
            check_exemption(
                Some("capacity-admission"),
                None,
                &spec,
                "capacity-admission"
            ),
            Some(ExemptionReason::WebhookNamespace),
        );
    }

    #[test]
    fn check_exemption_namespace_list_match() {
        // FR-001: pod namespace in excludedNamespaces -> Namespace.
        let spec = spec_with(Some(vec!["monitoring", "kube-system"]), None);
        assert_eq!(
            check_exemption(Some("monitoring"), None, &spec, "capacity-admission"),
            Some(ExemptionReason::Namespace),
        );
    }

    #[test]
    fn check_exemption_priority_class_match() {
        // FR-002: pod priorityClassName in excludedPriorityClasses -> PriorityClass.
        let spec = spec_with(None, Some(vec!["system-node-critical", "gold"]));
        assert_eq!(
            check_exemption(
                Some("default"),
                Some("system-node-critical"),
                &spec,
                "capacity-admission"
            ),
            Some(ExemptionReason::PriorityClass),
        );
    }

    #[test]
    fn check_exemption_no_match_returns_none() {
        let spec = spec_with(Some(vec!["monitoring"]), Some(vec!["gold"]));
        assert_eq!(
            check_exemption(
                Some("app-team-a"),
                Some("bronze"),
                &spec,
                "capacity-admission"
            ),
            None,
        );
    }

    #[test]
    fn check_exemption_first_match_precedence() {
        // FR-003 + data-model §3.2 order: webhook namespace -> namespaces ->
        // priority classes; first match wins.
        let spec = spec_with(
            Some(vec!["capacity-admission", "kube-system"]),
            Some(vec!["system-node-critical"]),
        );
        // Webhook namespace beats a namespace-list match.
        assert_eq!(
            check_exemption(
                Some("capacity-admission"),
                Some("system-node-critical"),
                &spec,
                "capacity-admission"
            ),
            Some(ExemptionReason::WebhookNamespace),
        );
        // Namespace beats a priority-class match (checked before priority).
        assert_eq!(
            check_exemption(
                Some("kube-system"),
                Some("system-node-critical"),
                &spec,
                "capacity-admission"
            ),
            Some(ExemptionReason::Namespace),
        );
    }

    #[test]
    fn check_exemption_empty_or_absent_lists_return_none() {
        // FR-004: absent and empty-list lists behave identically (no exclusions).
        assert_eq!(
            check_exemption(
                Some("monitoring"),
                Some("gold"),
                &spec_with(None, None),
                "ns"
            ),
            None,
        );
        assert_eq!(
            check_exemption(
                Some("monitoring"),
                Some("gold"),
                &spec_with(Some(vec![]), Some(vec![])),
                "ns"
            ),
            None,
        );
    }

    #[test]
    fn check_exemption_absent_or_empty_priority_class_never_matches() {
        // Edge case: absent (None) and empty-string priority class never match,
        // even if the list happens to name one.
        let spec = spec_with(None, Some(vec!["system-node-critical", ""]));
        assert_eq!(
            check_exemption(Some("default"), None, &spec, "capacity-admission"),
            None,
            "absent priority class must not match (US2 AC4)"
        );
        assert_eq!(
            check_exemption(Some("default"), Some(""), &spec, "capacity-admission"),
            None,
            "empty-string priority class must not match (Edge Case)"
        );
    }

    #[test]
    fn check_exemption_duplicate_entries_match_once() {
        // Edge case: duplicate entries are a harmless set — no error, single match.
        let spec = spec_with(Some(vec!["monitoring", "monitoring"]), None);
        assert_eq!(
            check_exemption(Some("monitoring"), None, &spec, "capacity-admission"),
            Some(ExemptionReason::Namespace),
        );
    }
}
