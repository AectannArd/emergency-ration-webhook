//! Admission error types and the fail-closed mapping to `AdmissionResponse`.
//!
//! Constitution Principle I (NON-NEGOTIABLE): every error path returns
//! `allowed: false`. `From<AdmissionError> for AdmissionResponse` enforces that
//! invariant at the type boundary. See `contracts/admission-webhook.md`
//! §Error Path Matrix for the per-variant `code`/`message` mapping.

use kube::core::Status;
use kube::core::admission::AdmissionResponse;

/// Which capacity resource a figure refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Cpu,
    Memory,
}

impl ResourceType {
    /// Human label used in rejection messages.
    pub fn label(self) -> &'static str {
        match self {
            ResourceType::Cpu => "CPU",
            ResourceType::Memory => "memory",
        }
    }
}

/// One resource that a pod's projected allocation would push over the ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetViolation {
    pub resource: ResourceType,
    pub allocated: i64,
    pub requested: i64,
    pub projected: i64,
    pub ceiling: i64,
}

impl BudgetViolation {
    /// Format the per-resource rejection line per the Error Path Matrix / T028.
    pub fn message_line(&self) -> String {
        match self.resource {
            ResourceType::Cpu => format!(
                "CPU budget exceeded: allocated {}m, requested {}m, projected {}m, ceiling {}m",
                self.allocated, self.requested, self.projected, self.ceiling
            ),
            ResourceType::Memory => format!(
                "memory budget exceeded: allocated {} bytes, requested {} bytes, projected {} bytes, ceiling {} bytes",
                self.allocated, self.requested, self.projected, self.ceiling
            ),
        }
    }
}

/// Which cached capacity state was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingCapacityData {
    Allocation,
    ClusterCapacity,
}

impl MissingCapacityData {
    fn detail(self) -> &'static str {
        match self {
            MissingCapacityData::Allocation => "allocation state not initialised",
            MissingCapacityData::ClusterCapacity => "cluster capacity state not initialised",
        }
    }
}

/// Every reason the webhook rejects an admission request.
///
/// Each variant carries exactly the data needed to render its contract message.
#[derive(Debug, Clone)]
pub enum AdmissionError {
    /// Pod's projected allocation exceeds the budget for one or more resources.
    OverBudget { violations: Vec<BudgetViolation> },
    /// Cached capacity data is older than the freshness threshold.
    CapacityDataStale { age_secs: u64, threshold_secs: u64 },
    /// A required capacity CRD is not yet populated.
    CapacityDataMissing { which: MissingCapacityData },
    /// The AdmissionReview body could not be deserialised.
    DeserialisationFailure { detail: String },
    /// A resource quantity string in the pod spec could not be parsed.
    QuantityParseFailure { field: String, value: String },
    /// The admission decision exceeded the per-request timeout.
    Timeout { timeout_ms: u64 },
    /// A panic was caught inside the admission decision.
    InternalError,
    /// Any error not matching a known variant (Principle III catch-all).
    Unknown { detail: String },
}

impl AdmissionError {
    /// HTTP status code for this rejection (see Error Path Matrix).
    pub fn status_code(&self) -> u16 {
        match self {
            AdmissionError::OverBudget { .. } => 403,
            AdmissionError::DeserialisationFailure { .. }
            | AdmissionError::QuantityParseFailure { .. } => 400,
            AdmissionError::CapacityDataStale { .. }
            | AdmissionError::CapacityDataMissing { .. }
            | AdmissionError::Timeout { .. }
            | AdmissionError::InternalError
            | AdmissionError::Unknown { .. } => 500,
        }
    }

    /// Machine-readable reason slug for the AdmissionResponse.
    pub fn reason(&self) -> &'static str {
        // NOTE: takes `&self` (not `self`) so callers can read several fields
        // (reason/status_code/message) from one owned value without moving it.
        match self {
            AdmissionError::OverBudget { .. } => "OverBudget",
            AdmissionError::CapacityDataStale { .. } => "CapacityDataStale",
            AdmissionError::CapacityDataMissing { .. } => "CapacityDataMissing",
            AdmissionError::DeserialisationFailure { .. } => "DeserialisationFailure",
            AdmissionError::QuantityParseFailure { .. } => "QuantityParseFailure",
            AdmissionError::Timeout { .. } => "Timeout",
            AdmissionError::InternalError => "InternalError",
            AdmissionError::Unknown { .. } => "Unknown",
        }
    }

    /// Human-readable rejection message (see Error Path Matrix).
    pub fn message(&self) -> String {
        match self {
            AdmissionError::OverBudget { violations } => violations
                .iter()
                .map(BudgetViolation::message_line)
                .collect::<Vec<_>>()
                .join("\n"),
            AdmissionError::CapacityDataStale {
                age_secs,
                threshold_secs,
            } => format!(
                "capacity data unavailable: last refresh {age_secs}s ago exceeds {threshold_secs}s threshold"
            ),
            AdmissionError::CapacityDataMissing { which } => {
                format!("capacity data unavailable: {}", which.detail())
            }
            AdmissionError::DeserialisationFailure { detail } => {
                format!("admission request malformed: {detail}")
            }
            AdmissionError::QuantityParseFailure { field, value } => {
                format!("cannot parse resource quantity in pod spec: {field}={value}")
            }
            AdmissionError::Timeout { timeout_ms } => {
                format!("admission decision timed out after {timeout_ms}ms")
            }
            AdmissionError::InternalError => {
                "internal error: panic in admission handler".to_string()
            }
            AdmissionError::Unknown { detail } => format!("internal error: {detail}"),
        }
    }

    /// Build the fail-closed response for this error, echoing the request `uid`.
    pub fn into_response(self, uid: impl Into<String>) -> AdmissionResponse {
        let mut response: AdmissionResponse = self.into();
        response.uid = uid.into();
        response
    }
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for AdmissionError {}

impl From<AdmissionError> for AdmissionResponse {
    /// Principle I: an error always becomes `allowed: false` with the matching
    /// status code and message. The request `uid` is left empty here; callers
    /// set it (see [`AdmissionError::into_response`]).
    fn from(error: AdmissionError) -> Self {
        let reason = error.reason();
        let code = error.status_code();
        let message = error.message();
        let mut response = AdmissionResponse::invalid(reason);
        response.allowed = false;
        response.result = Status::failure(&message, reason).with_code(code);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(resource: ResourceType) -> BudgetViolation {
        BudgetViolation {
            resource,
            allocated: 70_000,
            requested: 15_000,
            projected: 85_000,
            ceiling: 80_000,
        }
    }

    #[test]
    fn every_variant_denies() {
        let cases: Vec<AdmissionError> = vec![
            AdmissionError::OverBudget {
                violations: vec![violation(ResourceType::Cpu)],
            },
            AdmissionError::CapacityDataStale {
                age_secs: 45,
                threshold_secs: 30,
            },
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation,
            },
            AdmissionError::DeserialisationFailure {
                detail: "unexpected token".into(),
            },
            AdmissionError::QuantityParseFailure {
                field: "containers[0].resources.requests.cpu".into(),
                value: "abc".into(),
            },
            AdmissionError::Timeout { timeout_ms: 100 },
            AdmissionError::InternalError,
            AdmissionError::Unknown {
                detail: "something broke".into(),
            },
        ];
        for error in cases {
            let response = error.into_response("uid-123");
            assert!(!response.allowed, "variant must be denied");
            assert_eq!(response.uid, "uid-123");
            assert_eq!(
                response.result.status,
                Some(kube::core::response::StatusSummary::Failure)
            );
        }
    }

    #[test]
    fn over_budget_has_403_and_figures() {
        let error = AdmissionError::OverBudget {
            violations: vec![
                violation(ResourceType::Cpu),
                violation(ResourceType::Memory),
            ],
        };
        let response: AdmissionResponse = error.into();
        assert!(!response.allowed);
        assert_eq!(response.result.code, 403);
        let message = &response.result.message;
        assert!(message.contains("CPU budget exceeded: allocated 70000m, requested 15000m, projected 85000m, ceiling 80000m"));
        assert!(message.contains("memory budget exceeded: allocated 70000 bytes"));
        // Both resources reported, newline-separated.
        assert_eq!(message.matches('\n').count(), 1);
    }

    #[test]
    fn deserialisation_failure_is_400() {
        let response: AdmissionResponse = AdmissionError::DeserialisationFailure {
            detail: "eof".into(),
        }
        .into();
        assert_eq!(response.result.code, 400);
        assert!(
            response
                .result
                .message
                .starts_with("admission request malformed: eof")
        );
    }

    #[test]
    fn quantity_parse_failure_is_400() {
        let response: AdmissionResponse = AdmissionError::QuantityParseFailure {
            field: "requests.cpu".into(),
            value: "xx".into(),
        }
        .into();
        assert_eq!(response.result.code, 400);
        assert!(response.result.message.contains("requests.cpu=xx"));
    }

    #[test]
    fn stale_data_is_500_with_threshold() {
        let response: AdmissionResponse = AdmissionError::CapacityDataStale {
            age_secs: 45,
            threshold_secs: 30,
        }
        .into();
        assert_eq!(response.result.code, 500);
        assert!(
            response
                .result
                .message
                .contains("last refresh 45s ago exceeds 30s threshold")
        );
    }

    #[test]
    fn timeout_is_500() {
        let response: AdmissionResponse = AdmissionError::Timeout { timeout_ms: 100 }.into();
        assert_eq!(response.result.code, 500);
        assert!(response.result.message.contains("timed out after 100ms"));
    }

    #[test]
    fn internal_error_message_matches_contract() {
        let response: AdmissionResponse = AdmissionError::InternalError.into();
        assert_eq!(
            response.result.message,
            "internal error: panic in admission handler"
        );
    }
}
