//! Node Capacity Controller (T018) — the *supply* side.
//!
//! Watches nodes and keeps the `cluster-capacity` `ClusterCapacity` CRD's
//! `.status` equal to the sum of every node's `.status.allocatable`. Read-only on
//! nodes; never interrupts node lifecycle (Principle V). See
//! `contracts/clustercapacity-crd.md` §Controller Behaviour.

use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use kube::api::{Patch, PatchParams, PostParams};
use kube::runtime::{reflector, watcher};
use kube::{Api, Client};
use tracing::{debug, info, warn};

use crate::crd::{
    CLUSTER_CAPACITY_NAME, ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use crate::resources::quantity::{parse_cpu, parse_memory};
use crate::time_util::now_rfc3339;

/// Sum `cpu` (→ milli-CPUs) and `memory` (→ bytes) from every node's
/// `.status.allocatable`. Pure: takes references, no client, exhaustively tested.
///
/// A node missing `.status.allocatable` (e.g. NotReady, no reported capacity)
/// contributes nothing. Individual unparseable quantities are skipped — node
/// allocatable is kubelet-authored and always well-formed in practice.
pub fn sum_node_allocatable<'a, I>(nodes: I) -> (i64, i64, i32)
where
    I: IntoIterator<Item = &'a Node>,
{
    let mut cpu = 0i64;
    let mut memory = 0i64;
    let mut count = 0i32;
    for node in nodes {
        let Some(allocatable) = node.status.as_ref().and_then(|s| s.allocatable.as_ref()) else {
            continue;
        };
        count += 1;
        if let Some(q) = allocatable.get("cpu") {
            cpu += parse_cpu(&q.0).unwrap_or(0);
        }
        if let Some(q) = allocatable.get("memory") {
            memory += parse_memory(&q.0).unwrap_or(0);
        }
    }
    (cpu, memory, count)
}

/// The decision [`ensure_singleton`] reaches from a singleton existence check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingletonCheck {
    /// The instance exists — leave it untouched (never overwrite).
    Exists,
    /// The instance is missing (404) — create the default singleton.
    Missing,
    /// The check errored unexpectedly — log and retry next cycle.
    Error,
}

/// Whether a kube error is a 404 NotFound — i.e. the singleton is absent. The
/// HTTP code is matched directly so the behaviour is robust to reason-string
/// variations across Kubernetes versions.
fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 404)
}

/// Classify the result of `Api::get` on the singleton: `Ok` → exists, 404 →
/// create, anything else → retry.
fn classify_check<T>(result: &Result<T, kube::Error>) -> SingletonCheck {
    match result {
        Ok(_) => SingletonCheck::Exists,
        Err(err) if is_not_found(err) => SingletonCheck::Missing,
        Err(_) => SingletonCheck::Error,
    }
}

/// Whether a `create` attempt left the singleton in place. `AlreadyExists` (409)
/// is a success — another replica won the race, but the singleton now exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateOutcome {
    Created,
    AlreadyExists,
    Error,
}

fn classify_create<T>(result: &Result<T, kube::Error>) -> CreateOutcome {
    match result {
        Ok(_) => CreateOutcome::Created,
        Err(kube::Error::Api(status)) if status.code == 409 => CreateOutcome::AlreadyExists,
        Err(_) => CreateOutcome::Error,
    }
}

/// The default `cluster-capacity` instance created when the singleton is absent:
/// empty spec (the CRD is supply-side and controller-written, no user fields).
fn default_capacity_singleton() -> ClusterCapacity {
    ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec {})
}

/// Idempotent get-or-create of the `cluster-capacity` singleton.
///
/// Called once at controller start. A 409 `AlreadyExists` (e.g. another replica
/// won the race) is treated as success, and an existing instance is never
/// overwritten. Without this, `patch_status` 404s on the missing instance and the
/// supply figures are never written — leaving the webhook with no capacity data,
/// so it fails closed on every admission (Constitution Principle I).
pub async fn ensure_singleton(api: &Api<ClusterCapacity>) {
    let lookup = api.get(CLUSTER_CAPACITY_NAME).await;
    match classify_check(&lookup) {
        SingletonCheck::Exists => debug!(
            name = CLUSTER_CAPACITY_NAME,
            "ClusterCapacity singleton already exists"
        ),
        SingletonCheck::Missing => {
            let created = api
                .create(&PostParams::default(), &default_capacity_singleton())
                .await;
            match classify_create(&created) {
                CreateOutcome::Created => info!(
                    name = CLUSTER_CAPACITY_NAME,
                    "created ClusterCapacity singleton"
                ),
                CreateOutcome::AlreadyExists => debug!(
                    name = CLUSTER_CAPACITY_NAME,
                    "ClusterCapacity singleton already exists (race with another replica)"
                ),
                CreateOutcome::Error => {
                    if let Err(err) = &created {
                        warn!(%err, "failed to create ClusterCapacity singleton; retrying next cycle");
                    }
                }
            }
        }
        SingletonCheck::Error => {
            if let Err(err) = &lookup {
                warn!(%err, "failed to check ClusterCapacity singleton; retrying next cycle");
            }
        }
    }
}

/// Recompute the aggregate and merge-patch the CRD's `.status` subresource.
///
/// If the singleton was deleted after startup the patch 404s; we recreate it
/// (via [`ensure_singleton`]) and retry once so this event's figures are not
/// lost. Any other failure is logged and retried on the next node event.
pub async fn patch_status(api: &Api<ClusterCapacity>, cpu: i64, memory: i64, node_count: i32) {
    let params = PatchParams::apply("node-capacity-controller");
    match patch_once(api, &params, cpu, memory, node_count).await {
        Ok(_) => debug!(
            node_count,
            cpu_milli = cpu,
            memory_bytes = memory,
            "patched ClusterCapacity status"
        ),
        Err(err) if is_not_found(&err) => {
            warn!(
                %err,
                "ClusterCapacity singleton missing during patch; recreating and retrying once"
            );
            ensure_singleton(api).await;
            match patch_once(api, &params, cpu, memory, node_count).await {
                Ok(_) => debug!(
                    node_count,
                    "patched ClusterCapacity status after recreating singleton"
                ),
                Err(retry_err) => {
                    warn!(%retry_err, "failed to patch ClusterCapacity status after recreate")
                }
            }
        }
        Err(err) => warn!(%err, "failed to patch ClusterCapacity status"),
    }
}

/// Build the status from the raw figures and merge-patch it onto the singleton.
async fn patch_once(
    api: &Api<ClusterCapacity>,
    params: &PatchParams,
    cpu: i64,
    memory: i64,
    node_count: i32,
) -> kube::Result<ClusterCapacity> {
    let status = ClusterCapacityStatus {
        total_allocatable_cpu_milli: cpu,
        total_allocatable_memory_bytes: memory,
        node_count,
        last_updated: now_rfc3339(),
    };
    api.patch_status(CLUSTER_CAPACITY_NAME, params, &Patch::Merge(status))
        .await
}

/// Run the controller until the runtime is shut down. Owns a node reflector; on
/// every node event it recomputes the aggregate from the cache and patches the
/// `cluster-capacity` status (no network reads on the hot path).
pub async fn run(client: Client) {
    let nodes = Api::<Node>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client);
    let (store, writer) = reflector::store::<Node>();

    // Create the singleton before any patch_status — otherwise the first node
    // event 404s on a missing instance and the supply figures are never written.
    ensure_singleton(&capacity_api).await;

    let stream = reflector::reflector(writer, watcher::watcher(nodes, watcher::Config::default()));
    stream
        .for_each(|event| {
            let store = store.clone();
            let capacity_api = capacity_api.clone();
            async move {
                match event {
                    Ok(_) => {
                        let snapshot = store.state();
                        let (cpu, memory, node_count) =
                            sum_node_allocatable(snapshot.iter().map(|node| node.as_ref()));
                        patch_status(&capacity_api, cpu, memory, node_count).await;
                    }
                    Err(err) => warn!(%err, "node watch error"),
                }
            }
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::mock_api::{
        already_exists, created_object, mock_client, not_found, ok_object,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use kube::core::Status;
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn node(name: &str, cpu: &str, memory: &str) -> Node {
        let mut allocatable = BTreeMap::new();
        allocatable.insert("cpu".to_string(), Quantity(cpu.to_string()));
        allocatable.insert("memory".to_string(), Quantity(memory.to_string()));
        Node {
            status: Some(k8s_openapi::api::core::v1::NodeStatus {
                allocatable: Some(allocatable),
                ..Default::default()
            }),
            ..Default::default()
        }
        .with_name(name) // helper below
    }

    // k8s_openapi `Node` is `ResourceExt`, but constructing metadata by hand is
    // noisy; use a small extension.
    trait Named {
        fn with_name(self, name: &str) -> Self;
    }
    impl Named for Node {
        fn with_name(mut self, name: &str) -> Self {
            self.metadata.name = Some(name.to_string());
            self
        }
    }

    #[test]
    fn sums_allocatable_across_nodes() {
        let nodes = vec![node("a", "16", "32Gi"), node("b", "8", "16Gi")];
        let (cpu, memory, count) = sum_node_allocatable(&nodes);
        assert_eq!(cpu, 24_000); // (16 + 8) cores
        assert_eq!(memory, 48 * 1024 * 1024 * 1024); // 48 GiB
        assert_eq!(count, 2);
    }

    #[test]
    fn skips_nodes_without_status() {
        let bare = Node::default();
        let healthy = node("a", "4", "8Gi");
        let (cpu, memory, count) = sum_node_allocatable(&[bare, healthy]);
        assert_eq!(cpu, 4_000);
        assert_eq!(memory, 8 * 1024 * 1024 * 1024);
        assert_eq!(count, 1, "the status-less node is not counted");
    }

    #[test]
    fn empty_cluster_is_zero() {
        let (cpu, memory, count) = sum_node_allocatable(Vec::<Node>::new().iter());
        assert_eq!((cpu, memory, count), (0, 0, 0));
    }

    #[test]
    fn missing_resource_key_contributes_zero() {
        let mut allocatable = BTreeMap::new();
        allocatable.insert("cpu".to_string(), Quantity("2".to_string())); // no memory
        let n = Node {
            status: Some(k8s_openapi::api::core::v1::NodeStatus {
                allocatable: Some(allocatable),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (cpu, memory, count) = sum_node_allocatable(&[n]);
        assert_eq!(cpu, 2_000);
        assert_eq!(memory, 0);
        assert_eq!(count, 1);
    }

    // ---- singleton autocreation (ensure_singleton decision logic) ----

    /// Build a `kube::Error::Api` carrying a status with the given HTTP code and
    /// reason, mirroring what the apiserver returns for 404/409/etc.
    fn api_error(code: u16, reason: &str, message: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    #[test]
    fn existing_singleton_is_not_recreated() {
        let lookup: Result<ClusterCapacity, kube::Error> = Ok(ClusterCapacity::new(
            CLUSTER_CAPACITY_NAME,
            ClusterCapacitySpec {},
        ));
        // Exists ⇒ ensure_singleton does not call create (no overwrite).
        assert_eq!(classify_check(&lookup), SingletonCheck::Exists);
    }

    #[test]
    fn missing_singleton_triggers_create() {
        let lookup: Result<ClusterCapacity, kube::Error> =
            Err(api_error(404, "NotFound", "clustercapacities not found"));
        assert_eq!(classify_check(&lookup), SingletonCheck::Missing);
    }

    #[test]
    fn unexpected_get_error_is_retried_not_created() {
        // A 403 (RBAC) is not a missing instance — never try to create blindly.
        let lookup: Result<ClusterCapacity, kube::Error> =
            Err(api_error(403, "Forbidden", "forbidden"));
        assert_eq!(classify_check(&lookup), SingletonCheck::Error);
    }

    #[test]
    fn create_conflict_is_treated_as_success() {
        // 409 = another replica won the race; the singleton now exists.
        let created: Result<ClusterCapacity, kube::Error> =
            Err(api_error(409, "AlreadyExists", "already exists"));
        assert_eq!(classify_create(&created), CreateOutcome::AlreadyExists);
    }

    #[test]
    fn create_unexpected_error_is_failure() {
        let created: Result<ClusterCapacity, kube::Error> =
            Err(api_error(403, "Forbidden", "forbidden"));
        assert_eq!(classify_create(&created), CreateOutcome::Error);
    }

    #[test]
    fn default_singleton_carries_singleton_name_and_empty_spec() {
        let cc = default_capacity_singleton();
        assert_eq!(cc.metadata.name.as_deref(), Some(CLUSTER_CAPACITY_NAME));
        // Spec is empty (no user fields); constructing it is the whole point.
    }

    #[test]
    fn patch_not_found_triggers_recreate_and_retry() {
        // A 404 on patch_status means the singleton vanished mid-run: the
        // controller must recreate it (then retry on the next event), not treat
        // it as a permanent failure. Other errors are left to retry naturally.
        assert!(is_not_found(&api_error(
            404,
            "NotFound",
            "clustercapacities not found"
        )));
        assert!(!is_not_found(&api_error(
            409,
            "AlreadyExists",
            "already exists"
        )));
        assert!(!is_not_found(&api_error(403, "Forbidden", "forbidden")));
    }

    // ---- ensure_singleton against a mocked apiserver (Principle VI) ----
    //
    // These drive a real `kube::Api` through `mock_api`, scripting the apiserver
    // responses to prove the get-or-create wiring end-to-end — not just the
    // decision logic covered by the pure tests above.

    #[tokio::test]
    async fn ensure_singleton_creates_cluster_capacity_when_absent() {
        let (client, mut handle) = mock_client();
        let api = Api::<ClusterCapacity>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // 1. Existence GET → 404 NotFound (singleton absent).
        let (req, respond) = handle.next_request().await.expect("existence GET");
        assert_eq!(req.method().as_str(), "GET");
        assert!(
            req.uri().path().ends_with("/cluster-capacity"),
            "GET targets the singleton: {}",
            req.uri()
        );
        respond.send_response(not_found());

        // 2. Create POST → 201 Created.
        let (req, respond) = handle.next_request().await.expect("create POST");
        assert_eq!(req.method().as_str(), "POST");
        respond.send_response(created_object(&default_capacity_singleton()));

        task.await.expect("ensure_singleton did not panic");
    }

    #[tokio::test]
    async fn ensure_singleton_skips_create_when_singleton_present() {
        let (client, mut handle) = mock_client();
        let api = Api::<ClusterCapacity>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // Existence GET → 200 OK carrying the existing instance.
        let (req, respond) = handle.next_request().await.expect("existence GET");
        assert_eq!(req.method().as_str(), "GET");
        respond.send_response(ok_object(&default_capacity_singleton()));

        task.await.expect("ensure_singleton did not panic");

        // No create may follow: with the instance present, ensure_singleton must
        // return after the GET. A short timeout proves no second request is
        // issued (it would resolve immediately if a create were attempted).
        match tokio::time::timeout(Duration::from_millis(100), handle.next_request()).await {
            Err(_elapsed) => {}
            Ok(Some(req)) => panic!(
                "ensure_singleton issued an unexpected create {}",
                req.0.method()
            ),
            Ok(None) => {}
        }
    }

    #[tokio::test]
    async fn ensure_singleton_tolerates_create_conflict() {
        let (client, mut handle) = mock_client();
        let api = Api::<ClusterCapacity>::all(client);
        let task = tokio::spawn(async move {
            ensure_singleton(&api).await;
        });

        // Existence GET → 404.
        let (_req, respond) = handle.next_request().await.expect("existence GET");
        respond.send_response(not_found());

        // Create POST → 409 AlreadyExists (another replica won the race). This is
        // success — the singleton exists, just not via our create.
        let (req, respond) = handle.next_request().await.expect("create POST");
        assert_eq!(req.method().as_str(), "POST");
        respond.send_response(already_exists());

        // 409 is success: ensure_singleton completes without error.
        task.await.expect("ensure_singleton did not panic on 409");
    }

    #[tokio::test]
    async fn patch_status_recreates_missing_singleton_then_retries() {
        let (client, mut handle) = mock_client();
        let api = Api::<ClusterCapacity>::all(client);
        let task = tokio::spawn(async move {
            patch_status(&api, 16_000, 32 * 1024 * 1024 * 1024, 1).await;
        });

        // 1. Initial status PATCH → 404 (singleton vanished mid-run).
        let (req, respond) = handle.next_request().await.expect("initial PATCH");
        assert_eq!(req.method().as_str(), "PATCH");
        respond.send_response(not_found());

        // 2. ensure_singleton existence GET → 404.
        let (_req, respond) = handle.next_request().await.expect("ensure GET");
        respond.send_response(not_found());

        // 3. ensure_singleton create POST → 201 Created.
        let (_req, respond) = handle.next_request().await.expect("ensure create POST");
        respond.send_response(created_object(&default_capacity_singleton()));

        // 4. Retried status PATCH → 200 OK.
        let (req, respond) = handle.next_request().await.expect("retry PATCH");
        assert_eq!(req.method().as_str(), "PATCH");
        respond.send_response(ok_object(&default_capacity_singleton()));

        task.await.expect("patch_status did not panic");
    }
}
