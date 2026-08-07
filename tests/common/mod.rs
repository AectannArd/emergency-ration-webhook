//! Shared mock infrastructure for the equalizer integration + BDD tests
//! (spec-013). Never built as a standalone test target — it is `mod`-included
//! (`#[path]`) by `tests/equalizer/reconcile.rs` and the BDD steps.
//!
//! Two kinds of mock:
//! - **Home apiserver** (tower-test, in-memory): serves the kubeconfig `Secret`
//!   GETs the reconcile loop issues against the home cluster. Driven by a handle
//!   that scripts request/response pairs.
//! - **Target apiservers** (real local axum HTTP servers): each serves the target
//!   cluster's `Allocation` + `ClusterCapacity` GETs and captures the `Allocation`
//!   spec-override PATCH. The kubeconfig `Secret` (served by the home mock) points
//!   at each target's `http://127.0.0.1:<port>` address, so the reconcile loop's
//!   `build_target_client` → real-HTTP path is exercised faithfully.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, Request, Response, StatusCode, Uri};
use axum::response::{IntoResponse, Response as AxumResponse};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose};
use capacity_admission_webhook::crd::{
    Allocation, AllocationSpec, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME,
    ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use kube::Client;
use kube::client::{Body, ClientBuilder};
use tower_test::mock;

// ---------------------------------------------------------------------------
// Home apiserver mock (tower-test): serves kubeconfig Secret GETs.
// ---------------------------------------------------------------------------

/// Handle used to script the home-cluster mock apiserver request/response pairs.
pub type MockHandle = mock::Handle<Request<Body>, Response<Body>>;

/// Build a [`kube::Client`] whose HTTP calls are served by a mock home apiserver,
/// returning the [`MockHandle`] used to script responses.
pub fn mock_home_client() -> (Client, MockHandle) {
    let (svc, handle) = mock::pair::<Request<Body>, Response<Body>>();
    let client = ClientBuilder::new(svc, "default").build();
    (client, handle)
}

/// A 200 OK response carrying a serialised object body (used for Secrets).
pub fn ok_object<T: serde::Serialize>(obj: &T) -> Response<Body> {
    let body = serde_json::to_vec(obj).expect("test object serialises");
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(body))
        .expect("static object response builds")
}

/// Extract the Secret name from a home-cluster GET path
/// `/api/v1/namespaces/<ns>/secrets/<name>`.
pub fn secret_name_from_path(path: &str) -> Option<&str> {
    path.split("/secrets/").nth(1)
}

// ---------------------------------------------------------------------------
// Target apiserver mock (real local axum HTTP server).
// ---------------------------------------------------------------------------

/// Captured state for one mocked target cluster, shared between the axum handler
/// and the test driver.
#[derive(Clone)]
struct TargetFixture {
    allocation: serde_json::Value,
    capacity: serde_json::Value,
    patches: Arc<Mutex<Vec<serde_json::Value>>>,
}

/// A running mocked target apiserver. `patches` collects the spec-override PATCH
/// bodies the reconcile loop issued against this cluster.
pub struct MockTarget {
    pub addr: SocketAddr,
    pub patches: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl MockTarget {
    /// The captured PATCH bodies (the `{"spec": {...}}` JSON documents).
    pub fn patches(&self) -> Vec<serde_json::Value> {
        self.patches.lock().expect("patches lock").clone()
    }
}

/// Route one mocked target apiserver request: GET Allocation → its status; GET
/// ClusterCapacity → its status; PATCH Allocation → capture body, return the
/// (unchanged) Allocation. Anything else → 404.
async fn target_route(
    method: Method,
    uri: Uri,
    State(state): State<TargetFixture>,
    body: Bytes,
) -> AxumResponse {
    let path = uri.path();
    if path.ends_with("/clustercapacities/cluster-capacity") {
        return Json(state.capacity.clone()).into_response();
    }
    if path.ends_with("/allocations/cluster-allocation") {
        return match method {
            Method::GET => Json(state.allocation.clone()).into_response(),
            Method::PATCH => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                    state.patches.lock().expect("patches lock").push(val);
                }
                Json(state.allocation.clone()).into_response()
            }
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    StatusCode::NOT_FOUND.into_response()
}

/// Bind a mocked target apiserver on an ephemeral local port, serving an
/// `Allocation` at the given CPU/memory utilization + a `ClusterCapacity` with the
/// given allocatable figures, and capturing any spec-override PATCH.
pub async fn spawn_mock_target(
    cpu_util: f64,
    mem_util: f64,
    cpu_milli: i64,
    mem_bytes: i64,
) -> MockTarget {
    let patches = Arc::new(Mutex::new(Vec::new()));
    let fixture = TargetFixture {
        allocation: allocation_json(cpu_util, mem_util),
        capacity: capacity_json(cpu_milli, mem_bytes),
        // The router captures a clone of this Arc; the shared inner Mutex is how
        // the handler's captured PATCHes reach the returned `MockTarget`.
        patches: Arc::clone(&patches),
    };
    let app = Router::new().fallback(target_route).with_state(fixture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock target");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            eprintln!("mock target server error: {err}");
        }
    });
    MockTarget { addr, patches }
}

/// A `cluster-allocation` singleton carrying the given utilization in its status.
fn allocation_json(cpu_util: f64, mem_util: f64) -> serde_json::Value {
    let mut alloc = Allocation::new(
        CLUSTER_ALLOCATION_NAME,
        AllocationSpec {
            budget_percent: 80,
            enforcement_mode: None,
            excluded_namespaces: None,
            excluded_priority_classes: None,
            cpu_budget_percent: None,
            memory_budget_percent: None,
        },
    );
    alloc.status = Some(AllocationStatus {
        allocated_cpu_milli: 0,
        allocated_memory_bytes: 0,
        ceiling_cpu_milli: 0,
        ceiling_memory_bytes: 0,
        utilization_percent_cpu: cpu_util,
        utilization_percent_memory: mem_util,
        last_updated: "2026-08-06T00:00:00Z".to_string(),
        effective_cpu_budget_percent: 80,
        effective_memory_budget_percent: 80,
    });
    serde_json::to_value(&alloc).expect("Allocation serialises")
}

/// A `cluster-capacity` singleton carrying the given allocatable figures.
fn capacity_json(cpu_milli: i64, mem_bytes: i64) -> serde_json::Value {
    let mut cc = ClusterCapacity::new(
        CLUSTER_CAPACITY_NAME,
        ClusterCapacitySpec {
            node_selectors: None,
        },
    );
    cc.status = Some(ClusterCapacityStatus {
        total_allocatable_cpu_milli: cpu_milli,
        total_allocatable_memory_bytes: mem_bytes,
        node_count: 1,
        last_updated: "2026-08-06T00:00:00Z".to_string(),
        excluded_node_count: 0,
        excluded_by_unschedulable: 0,
        excluded_by_selector: 0,
    });
    serde_json::to_value(&cc).expect("ClusterCapacity serialises")
}

/// Build a kubeconfig YAML pointing at `http://<addr>` (plain HTTP, no TLS, empty
/// user). `Config::from_custom_kubeconfig` does not connect, so the address only
/// needs to resolve once requests are issued.
pub fn kubeconfig_yaml(addr: &SocketAddr) -> Vec<u8> {
    format!(
        r#"
apiVersion: v1
kind: Config
clusters:
- name: target
  cluster:
    server: http://{addr}
contexts:
- name: target
  context:
    cluster: target
    user: target
current-context: target
users:
- name: target
  user: {{}}
"#
    )
    .into_bytes()
}

/// Build a `Secret` JSON object (raw) carrying base64-encoded kubeconfig bytes
/// under the given key (default `"kubeconfig"`).
pub fn kubeconfig_secret_value(
    name: &str,
    namespace: &str,
    kubeconfig_bytes: &[u8],
) -> serde_json::Value {
    let b64 = general_purpose::STANDARD.encode(kubeconfig_bytes);
    serde_json::json!({
        "kind": "Secret",
        "apiVersion": "v1",
        "metadata": {"name": name, "namespace": namespace},
        "data": {"kubeconfig": b64}
    })
}

/// Install the rustls ring CryptoProvider idempotently (kube-rs touches rustls
/// even for plain-HTTP configs). Safe to call from many tests.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
