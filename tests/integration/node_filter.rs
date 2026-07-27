//! Integration tests for the schedulable-node-filter (spec-006 T038-T040).
//!
//! Drives the real [`reconcile_now`] path through a mocked kube-apiserver so the
//! full filter pipeline — read selector → list nodes → sum with exclusion → patch
//! status — is exercised end-to-end, not just its pure pieces. Two scenarios:
//!
//! - `cordon_*`: a node list with one `unschedulable: true` node; the status
//!   PATCH must exclude it and report `excludedByUnschedulable: 1`.
//! - `selector_*`: a `ClusterCapacity` spec carrying a `nodeSelector` that matches
//!   control-plane nodes; the status PATCH must exclude them and report
//!   `excludedBySelector`.
//!
//! The mock harness here mirrors `src/controllers/mock_api.rs`, which is
//! `#[cfg(test)]`/`pub(crate)` and therefore unreachable from a separate test
//! target.

use axum::http::{Request, Response, StatusCode};
use capacity_admission_webhook::controllers::node_capacity::reconcile_now;
use k8s_openapi::api::core::v1::Node;
use capacity_admission_webhook::crd::{CLUSTER_CAPACITY_NAME, ClusterCapacity, ClusterCapacitySpec};
use http_body_util::BodyExt;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};
use kube::client::{Body, ClientBuilder};
use kube::{Api, Client};
use tower_test::mock;

type MockHandle = mock::Handle<Request<Body>, Response<Body>>;

/// A `kube::Client` whose HTTP calls are served by a mock apiserver.
fn mock_client() -> (Client, MockHandle) {
    let (svc, handle) = mock::pair::<Request<Body>, Response<Body>>();
    let client = ClientBuilder::new(svc, "default").build();
    (client, handle)
}

/// A 200 OK response carrying a serialised object body.
fn ok<T: serde::Serialize>(obj: &T) -> Response<Body> {
    let body = serde_json::to_vec(obj).expect("test object serialises");
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(body))
        .expect("static response builds")
}

/// The default `cluster-capacity` singleton (no nodeSelector → unschedulable-only).
fn default_capacity() -> ClusterCapacity {
    ClusterCapacity::new(
        CLUSTER_CAPACITY_NAME,
        ClusterCapacitySpec {
            node_selector: None,
        },
    )
}

/// A selector that matches nodes carrying the control-plane role label.
fn control_plane_selector() -> LabelSelector {
    LabelSelector {
        match_labels: None,
        match_expressions: Some(vec![LabelSelectorRequirement {
            key: "node-role.kubernetes.io/control-plane".to_string(),
            operator: "Exists".to_string(),
            values: None,
        }]),
    }
}

/// Collect a PATCH request's body and parse it as JSON.
async fn patch_body(req: Request<Body>) -> serde_json::Value {
    let bytes = BodyExt::collect(req.into_body())
        .await
        .expect("patch body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("patch body is JSON")
}

// ---------------------------------------------------------------------------
// US1: cordon exclusion (T038)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cordon_event_excludes_unschedulable_node_from_capacity() {
    let (client, mut handle) = mock_client();
    let nodes = Api::<Node>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client);
    let task = tokio::spawn(async move {
        reconcile_now(&nodes, &capacity_api).await;
    });

    // 1. Read the selector — GET the singleton (default, no nodeSelector).
    let (req, respond) = handle.next_request().await.expect("capacity GET");
    assert_eq!(req.method().as_str(), "GET");
    respond.send_response(ok(&default_capacity()));

    // 2. Node LIST — one schedulable worker + one cordoned node.
    let (req, respond) = handle.next_request().await.expect("node LIST");
    assert_eq!(req.method().as_str(), "GET");
    let node_list = serde_json::json!({
        "apiVersion": "v1",
        "kind": "NodeList",
        "items": [
            {"metadata": {"name": "worker"},
             "status": {"allocatable": {"cpu": "8", "memory": "16Gi"}}},
            {"metadata": {"name": "cordoned"},
             "spec": {"unschedulable": true},
             "status": {"allocatable": {"cpu": "16", "memory": "32Gi"}}}
        ]
    });
    respond.send_response(ok(&node_list));

    // 3. Status PATCH — the cordoned node is excluded from the aggregate.
    let (req, respond) = handle.next_request().await.expect("status PATCH");
    assert_eq!(req.method().as_str(), "PATCH");
    let payload = patch_body(req).await;
    assert_eq!(
        payload["status"]["nodeCount"].as_i64(),
        Some(1),
        "cordoned node excluded from nodeCount: {payload}"
    );
    assert_eq!(
        payload["status"]["totalAllocatableCpuMilli"].as_i64(),
        Some(8_000)
    );
    assert_eq!(
        payload["status"]["excludedByUnschedulable"].as_i64(),
        Some(1)
    );
    assert_eq!(payload["status"]["excludedBySelector"].as_i64(), Some(0));
    assert_eq!(payload["status"]["excludedNodeCount"].as_i64(), Some(1));
    respond.send_response(ok(&default_capacity()));

    task.await.expect("reconcile did not panic");
}

// ---------------------------------------------------------------------------
// US2: label-selector exclusion (T039)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn selector_excludes_control_plane_nodes_from_capacity() {
    let (client, mut handle) = mock_client();
    let nodes = Api::<Node>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client);
    let task = tokio::spawn(async move {
        reconcile_now(&nodes, &capacity_api).await;
    });

    // 1. Read the selector — GET the singleton carrying a nodeSelector that
    //    matches control-plane nodes.
    let (_req, respond) = handle.next_request().await.expect("capacity GET");
    let capacity_with_selector = ClusterCapacity::new(
        CLUSTER_CAPACITY_NAME,
        ClusterCapacitySpec {
            node_selector: Some(control_plane_selector()),
        },
    );
    respond.send_response(ok(&capacity_with_selector));

    // 2. Node LIST — 2 workers + 1 control-plane node (label-matched).
    let (_req, respond) = handle.next_request().await.expect("node LIST");
    let node_list = serde_json::json!({
        "apiVersion": "v1",
        "kind": "NodeList",
        "items": [
            {"metadata": {"name": "w1", "labels": {"role": "worker"}},
             "status": {"allocatable": {"cpu": "8", "memory": "16Gi"}}},
            {"metadata": {"name": "w2", "labels": {"role": "worker"}},
             "status": {"allocatable": {"cpu": "8", "memory": "16Gi"}}},
            {"metadata": {"name": "cp",
                          "labels": {"node-role.kubernetes.io/control-plane": ""}},
             "status": {"allocatable": {"cpu": "16", "memory": "32Gi"}}}
        ]
    });
    respond.send_response(ok(&node_list));

    // 3. Status PATCH — the control-plane node is excluded by the selector.
    let (req, respond) = handle.next_request().await.expect("status PATCH");
    let payload = patch_body(req).await;
    assert_eq!(
        payload["status"]["nodeCount"].as_i64(),
        Some(2),
        "control-plane node excluded by selector: {payload}"
    );
    assert_eq!(
        payload["status"]["totalAllocatableCpuMilli"].as_i64(),
        Some(16_000)
    );
    assert_eq!(payload["status"]["excludedByUnschedulable"].as_i64(), Some(0));
    assert_eq!(payload["status"]["excludedBySelector"].as_i64(), Some(1));
    assert_eq!(payload["status"]["excludedNodeCount"].as_i64(), Some(1));
    respond.send_response(ok(&default_capacity()));

    task.await.expect("reconcile did not panic");
}
