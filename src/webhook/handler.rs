//! Admission webhook HTTP handler and the network-free decision core (T020).
//!
//! Layering (innermost out):
//! - [`decide`] — the pure decision: extract pod requests (FR-005), resolve the
//!   UPDATE delta (FR-004), run [`check_budget`], and return an admit response or
//!   an [`AdmissionError`]. No I/O, exhaustively unit-testable.
//! - [`evaluate`] — reads the cached `Allocation` singleton from the reflector
//!   [`Store`] and delegates to [`decide`]. Missing cache → fail-closed
//!   (`CapacityDataMissing`, Principle I).
//! - [`handle`] / [`validate`] — the axum `POST /validate` path: deserialise the
//!   `AdmissionReview` body, run [`evaluate`], and map any error to a fail-closed
//!   response. [`healthz`] is the readiness probe.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use k8s_openapi::api::core::v1::Pod;
use kube::ResourceExt;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview, Operation};
use kube::runtime::reflector::Store;
use std::sync::Arc;

use crate::crd::{Allocation, AllocationStatus, CLUSTER_ALLOCATION_NAME};
use crate::resources::quantity::{self, QuantityParseError};
use crate::webhook::admission::{AdmissionVerdict, check_budget};
use crate::webhook::error::{AdmissionError, MissingCapacityData};

/// Shared state injected into every request handler. The webhook's hot path reads
/// allocation figures purely from the in-process reflector cache — no network.
#[derive(Clone)]
pub struct AppState {
    pub allocation_store: Arc<Store<Allocation>>,
}

/// Build the axum router for the webhook endpoints.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/validate", post(validate))
        .route("/healthz", get(healthz))
        .with_state(state)
}

/// Readiness/liveness probe. Always 200 once the process is up.
pub async fn healthz() -> impl IntoResponse {
    "ok"
}

/// `POST /validate` — entry point hit by the kube-apiserver.
pub async fn validate(State(state): State<AppState>, body: Bytes) -> Json<serde_json::Value> {
    let response = handle(body, &state.allocation_store).await;
    // Wrap the AdmissionResponse in an AdmissionReview envelope for the apiserver.
    let review = response.into_review();
    Json(serde_json::to_value(&review).expect("admission response is always serialisable"))
}

/// Deserialise the AdmissionReview body, evaluate it against the cached
/// allocation, and return the verdict response. Pure (no `self`/`State`), so it
/// is exercised directly by integration tests.
pub async fn handle(body: Bytes, store: &Store<Allocation>) -> AdmissionResponse {
    let review: AdmissionReview<Pod> = match serde_json::from_slice(&body) {
        Ok(review) => review,
        Err(err) => {
            return AdmissionError::DeserialisationFailure {
                detail: err.to_string(),
            }
            .into_response("");
        }
    };

    let request: AdmissionRequest<Pod> = match review.try_into() {
        Ok(request) => request,
        Err(_) => {
            return AdmissionError::DeserialisationFailure {
                detail: "request field missing".to_string(),
            }
            .into_response("");
        }
    };

    match evaluate(&request, store) {
        Ok(response) => response,
        Err(err) => err.into_response(&request.uid),
    }
}

/// Read the cached `cluster-allocation` status and decide. Fail-closed when the
/// allocation state is not yet populated (Principle I).
pub fn evaluate(
    request: &AdmissionRequest<Pod>,
    store: &Store<Allocation>,
) -> Result<AdmissionResponse, AdmissionError> {
    let status = store
        .find(|allocation| allocation.name_any() == CLUSTER_ALLOCATION_NAME)
        .and_then(|allocation| allocation.status.clone());
    let status = status.ok_or(AdmissionError::CapacityDataMissing {
        which: MissingCapacityData::Allocation,
    })?;
    decide(request, &status)
}

/// Pure decision core (data-model.md §3–4). Extracts the pod's effective request,
/// resolves an UPDATE as a delta against the old object, and checks the budget.
pub fn decide(
    request: &AdmissionRequest<Pod>,
    status: &AllocationStatus,
) -> Result<AdmissionResponse, AdmissionError> {
    let new_request = pod_request(request.object.as_ref())?;

    // FR-004: on UPDATE, evaluate the *delta* (new − old) so the already-running
    // pod is not double-counted against the budget. CREATE evaluates the full
    // request (old is absent → delta == new).
    let effective = match request.operation {
        Operation::Update => {
            let old_request = pod_request(request.old_object.as_ref())?;
            (new_request.0 - old_request.0, new_request.1 - old_request.1)
        }
        _ => new_request,
    };

    let allocated = (status.allocated_cpu_milli, status.allocated_memory_bytes);
    let ceilings = (status.ceiling_cpu_milli, status.ceiling_memory_bytes);

    match check_budget(allocated, effective, ceilings) {
        AdmissionVerdict::Admit => Ok(AdmissionResponse::from(request)),
        AdmissionVerdict::Deny(violations) => Err(AdmissionError::OverBudget { violations }),
    }
}

/// Extract the (cpu_milli, memory_bytes) request of a pod, applying the
/// Kubernetes defaulting convention (T009). A missing object contributes 0.
fn pod_request(pod: Option<&Pod>) -> Result<(i64, i64), AdmissionError> {
    let Some(spec) = pod.and_then(|p| p.spec.as_ref()) else {
        return Ok((0, 0));
    };
    quantity::extract_pod_requests(spec).map_err(|QuantityParseError::Invalid { input, .. }| {
        AdmissionError::QuantityParseFailure {
            field: "resources.requests".to_string(),
            value: input,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{Allocation, AllocationSpec, AllocationStatus};
    use k8s_openapi::api::core::v1::{Container, PodSpec, ResourceRequirements};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    /// Build a pod with a single container requesting `cpu` / `memory`.
    fn pod(cpu: &str, memory: &str) -> Pod {
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
        requests.insert("memory".to_string(), Quantity(memory.to_string()));
        let spec = PodSpec {
            containers: vec![Container {
                resources: Some(ResourceRequirements {
                    requests: Some(requests),
                    limits: None,
                    claims: None,
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        Pod {
            spec: Some(spec),
            ..Default::default()
        }
    }

    /// Build an `AdmissionRequest<Pod>` by round-tripping a real AdmissionReview
    /// JSON document — the same path the apiserver body takes.
    fn request(obj: &Pod, op: Operation, old: Option<&Pod>) -> AdmissionRequest<Pod> {
        let object = serde_json::to_value(obj).unwrap();
        let old_object = match old {
            Some(o) => serde_json::to_value(o).unwrap(),
            None => serde_json::Value::Null,
        };
        let op_str = match op {
            Operation::Create => "CREATE",
            Operation::Update => "UPDATE",
            Operation::Delete => "DELETE",
            Operation::Connect => "CONNECT",
        };
        let review = serde_json::json!({
            "kind": "AdmissionReview",
            "apiVersion": "admission.k8s.io/v1",
            "request": {
                "uid": "uid-1",
                "name": "p",
                "namespace": "default",
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "resource": {"group": "", "version": "v1", "resource": "pods"},
                "operation": op_str,
                "userInfo": {"username": "test"},
                "object": object,
                "oldObject": old_object,
                "dryRun": false,
            }
        });
        let parsed: AdmissionReview<Pod> = serde_json::from_value(review).unwrap();
        parsed.try_into().unwrap()
    }

    /// Allocation status: allocated 70 CPU / 110Gi, ceiling 80 CPU / 160Gi.
    fn status() -> AllocationStatus {
        AllocationStatus {
            allocated_cpu_milli: 70_000,
            allocated_memory_bytes: 110 * 1024,
            ceiling_cpu_milli: 80_000,
            ceiling_memory_bytes: 160 * 1024,
            utilization_percent_cpu: 0.0,
            utilization_percent_memory: 0.0,
            last_updated: "2026-07-26T00:00:00Z".to_string(),
        }
    }

    fn allocation_with(status: AllocationStatus) -> Allocation {
        let mut a = Allocation::new(
            CLUSTER_ALLOCATION_NAME,
            AllocationSpec { budget_percent: 80 },
        );
        a.status = Some(status);
        a
    }

    // ---- decide: CREATE admit/deny (spec scenarios 1, 2, 4) ----

    #[test]
    fn create_under_ceiling_is_admitted() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let resp = decide(&req, &status()).unwrap();
        assert!(resp.allowed);
        assert_eq!(resp.uid, "uid-1");
    }

    #[test]
    fn create_over_ceiling_is_denied_with_figures() {
        // 70 CPU + 15 CPU = 85 > 80 ceiling → CPU over budget.
        let req = request(&pod("15", "1"), Operation::Create, None);
        let err = decide(&req, &status()).unwrap_err();
        let AdmissionError::OverBudget { violations } = err else {
            panic!("expected OverBudget, got {err:?}");
        };
        let cpu = &violations[0];
        assert_eq!(cpu.allocated, 70_000);
        assert_eq!(cpu.requested, 15_000);
        assert_eq!(cpu.projected, 85_000);
        assert_eq!(cpu.ceiling, 80_000);
    }

    #[test]
    fn create_zero_request_is_admitted() {
        let bare = Pod {
            spec: Some(PodSpec {
                containers: vec![k8s_openapi::api::core::v1::Container::default()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let req = request(&bare, Operation::Create, None);
        assert!(decide(&req, &status()).unwrap().allowed);
    }

    // ---- decide: UPDATE evaluated as delta (spec scenario 5) ----

    #[test]
    fn update_evaluates_delta_not_full_request() {
        // Existing pod at 10 CPU; update to 20 CPU → delta +10 (70→80, at ceiling → admit).
        let old = pod("10", "1");
        let new = pod("20", "1");
        let req = request(&new, Operation::Update, Some(&old));
        let resp = decide(&req, &status()).unwrap();
        assert!(
            resp.allowed,
            "delta of +10 lands exactly at the 80 ceiling (inclusive)"
        );
    }

    #[test]
    fn update_delta_over_ceiling_is_denied() {
        // Existing pod at 10 CPU; update to 30 CPU → delta +20 (70→90 > 80) → deny.
        let old = pod("10", "1");
        let new = pod("30", "1");
        let req = request(&new, Operation::Update, Some(&old));
        assert!(decide(&req, &status()).is_err());
    }

    // ---- evaluate: missing allocation is fail-closed ----

    #[test]
    fn evaluate_denies_when_allocation_missing() {
        let req = request(&pod("1", "1"), Operation::Create, None);
        // A store with no allocation present.
        let (store, _writer) = kube::runtime::reflector::store::<Allocation>();
        let err = evaluate(&req, &store).unwrap_err();
        assert!(matches!(
            err,
            AdmissionError::CapacityDataMissing {
                which: MissingCapacityData::Allocation
            }
        ));
    }

    #[test]
    fn evaluate_admits_when_allocation_present() {
        let req = request(&pod("5", "1Ki"), Operation::Create, None);
        let (store, mut writer) = kube::runtime::reflector::store::<Allocation>();
        writer.apply_watcher_event(&kube::runtime::watcher::Event::Apply(allocation_with(
            status(),
        )));
        assert!(evaluate(&req, &store).unwrap().allowed);
    }
}
