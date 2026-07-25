//! Node Capacity Controller (T018) — the *supply* side.
//!
//! Watches nodes and keeps the `cluster-capacity` `ClusterCapacity` CRD's
//! `.status` equal to the sum of every node's `.status.allocatable`. Read-only on
//! nodes; never interrupts node lifecycle (Principle V). See
//! `contracts/clustercapacity-crd.md` §Controller Behaviour.

use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use kube::api::{Patch, PatchParams};
use kube::runtime::{reflector, watcher};
use kube::{Api, Client};
use tracing::{debug, warn};

use crate::crd::{CLUSTER_CAPACITY_NAME, ClusterCapacity, ClusterCapacityStatus};
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

/// Recompute the aggregate and merge-patch the CRD's `.status` subresource.
pub async fn patch_status(api: &Api<ClusterCapacity>, cpu: i64, memory: i64, node_count: i32) {
    let status = ClusterCapacityStatus {
        total_allocatable_cpu_milli: cpu,
        total_allocatable_memory_bytes: memory,
        node_count,
        last_updated: now_rfc3339(),
    };
    let patch = Patch::Merge(status);
    let params = PatchParams::apply("node-capacity-controller");
    match api
        .patch_status(CLUSTER_CAPACITY_NAME, &params, &patch)
        .await
    {
        Ok(_) => debug!(
            node_count,
            cpu_milli = cpu,
            memory_bytes = memory,
            "patched ClusterCapacity status"
        ),
        Err(err) => warn!(%err, "failed to patch ClusterCapacity status"),
    }
}

/// Run the controller until the runtime is shut down. Owns a node reflector; on
/// every node event it recomputes the aggregate from the cache and patches the
/// `cluster-capacity` status (no network reads on the hot path).
pub async fn run(client: Client) {
    let nodes = Api::<Node>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client);
    let (store, writer) = reflector::store::<Node>();

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
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

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
}
