//! Allocation Controller (T019) — the *demand* side.
//!
//! Sums pod resource requests across non-terminal pods, reads the budget from the
//! `cluster-allocation` `Allocation` CRD `spec`, computes the ceiling from the
//! `ClusterCapacity` supply, and writes the result back to the `Allocation`
//! `.status`. See `contracts/allocation-crd.md` §Controller Behaviour.

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Patch, PatchParams};
use kube::runtime::reflector::Store;
use kube::runtime::{reflector, watcher};
use kube::{Api, Client, ResourceExt};
use std::time::Duration;
use tracing::{debug, warn};

use crate::crd::{
    Allocation, AllocationStatus, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity,
};
use crate::resources::quantity::extract_pod_requests;
use crate::time_util::now_rfc3339;
use crate::webhook::admission::ceiling;

/// A pod counts toward current allocation unless its phase is terminal.
///
/// Per `contracts/allocation-crd.md` §Pod Phase Filtering: `Pending`,
/// `Running`, and `Unknown` are counted; `Succeeded` and `Failed` are not. A pod
/// with no phase yet (just created, not scheduled) is counted — its requests are
/// reserved.
pub fn is_non_terminal(phase: Option<&str>) -> bool {
    !matches!(phase, Some("Failed") | Some("Succeeded"))
}

/// Sum effective CPU (milli) and memory (bytes) requests across non-terminal
/// pods, applying the Kubernetes defaulting convention via
/// [`extract_pod_requests`]. Pure and unit-tested.
pub fn sum_pod_allocation<'a, I>(pods: I) -> (i64, i64)
where
    I: IntoIterator<Item = &'a Pod>,
{
    let mut cpu = 0i64;
    let mut memory = 0i64;
    for pod in pods {
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        if !is_non_terminal(phase) {
            continue;
        }
        let Some(spec) = pod.spec.as_ref() else {
            continue;
        };
        // A pod with an unparseable quantity is skipped here; such a pod could not
        // have passed admission (the webhook rejects unparseable quantities), so it
        // never reaches the running set.
        if let Ok((c, m)) = extract_pod_requests(spec) {
            cpu += c;
            memory += m;
        }
    }
    (cpu, memory)
}

/// Build the full `AllocationStatus` from the raw figures. The ceiling is
/// `floor(supply * budget / 100)` per resource; utilisation is
/// `allocated / ceiling` (0 when there is no ceiling).
pub fn build_allocation_status(
    allocated: (i64, i64),
    total_supply: (i64, i64),
    budget_percent: i32,
) -> AllocationStatus {
    let ceilings = ceiling(total_supply, budget_percent);
    AllocationStatus {
        allocated_cpu_milli: allocated.0,
        allocated_memory_bytes: allocated.1,
        ceiling_cpu_milli: ceilings.0,
        ceiling_memory_bytes: ceilings.1,
        utilization_percent_cpu: ratio(allocated.0, ceilings.0),
        utilization_percent_memory: ratio(allocated.1, ceilings.1),
        last_updated: now_rfc3339(),
    }
}

fn ratio(allocated: i64, ceiling: i64) -> f64 {
    if ceiling == 0 {
        0.0
    } else {
        allocated as f64 / ceiling as f64
    }
}

/// Recompute allocation from the caches and merge-patch the `Allocation` status.
async fn recompute(
    pod_store: &Store<Pod>,
    capacity_store: &Store<ClusterCapacity>,
    allocation_api: &Api<Allocation>,
) {
    // The budget lives in the Allocation CRD spec. It changes rarely; a periodic
    // GET is cheap relative to the recompute interval and avoids a third cache.
    let budget = match allocation_api.get(CLUSTER_ALLOCATION_NAME).await {
        Ok(allocation) => allocation.spec.budget_percent,
        Err(err) => {
            debug!(%err, "Allocation CRD not present; skipping recompute");
            return;
        }
    };

    let pods = pod_store.state();
    let allocated = sum_pod_allocation(pods.iter().map(|pod| pod.as_ref()));

    let supply = capacity_store
        .find(|c| c.name_any() == CLUSTER_CAPACITY_NAME)
        .and_then(|c| c.status.clone())
        .map(|s| {
            (
                s.total_allocatable_cpu_milli,
                s.total_allocatable_memory_bytes,
            )
        })
        .unwrap_or((0, 0));

    let status = build_allocation_status(allocated, supply, budget);
    let patch = Patch::Merge(status);
    let params = PatchParams::apply("allocation-controller");
    if let Err(err) = allocation_api
        .patch_status(CLUSTER_ALLOCATION_NAME, &params, &patch)
        .await
    {
        warn!(%err, "failed to patch Allocation status");
    }
}

/// Run the controller until the runtime is shut down.
///
/// Keeps pod and `ClusterCapacity` reflector caches warm in background tasks,
/// then recomputes the `Allocation` status on a short interval. Every recompute
/// reads only from the in-process caches (plus a single budget GET); the
/// admission hot path never touches this.
pub async fn run(client: Client) {
    let pods_api = Api::<Pod>::all(client.clone());
    let capacity_api = Api::<ClusterCapacity>::all(client.clone());
    let allocation_api = Api::<Allocation>::all(client);

    let (pod_store, pod_writer) = reflector::store::<Pod>();
    let (capacity_store, capacity_writer) = reflector::store::<ClusterCapacity>();

    tokio::spawn(
        reflector::reflector(
            pod_writer,
            watcher::watcher(pods_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "pod watch error");
            }
        }),
    );
    tokio::spawn(
        reflector::reflector(
            capacity_writer,
            watcher::watcher(capacity_api, watcher::Config::default()),
        )
        .for_each(|event| async {
            if let Err(err) = event {
                warn!(%err, "ClusterCapacity watch error");
            }
        }),
    );

    // Bounded-latency recompute: any change is reflected within the tick window.
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        recompute(&pod_store, &capacity_store, &allocation_api).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, PodSpec, ResourceRequirements};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    fn pod_with(phase: Option<&str>, cpu: &str, memory: &str) -> Pod {
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity(cpu.to_string()));
        requests.insert("memory".to_string(), Quantity(memory.to_string()));
        Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    resources: Some(ResourceRequirements {
                        requests: Some(requests),
                        limits: None,
                        claims: None,
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(k8s_openapi::api::core::v1::PodStatus {
                phase: phase.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // ---- is_non_terminal ----

    #[test]
    fn pending_running_unknown_are_counted() {
        assert!(is_non_terminal(Some("Pending")));
        assert!(is_non_terminal(Some("Running")));
        assert!(is_non_terminal(Some("Unknown")));
        assert!(is_non_terminal(None), "unscheduled pod is counted");
    }

    #[test]
    fn succeeded_and_failed_are_terminal() {
        assert!(!is_non_terminal(Some("Succeeded")));
        assert!(!is_non_terminal(Some("Failed")));
    }

    // ---- sum_pod_allocation ----

    #[test]
    fn sums_running_pod_requests() {
        let pods = vec![
            pod_with(Some("Running"), "1", "1Gi"),
            pod_with(Some("Pending"), "2", "2Gi"),
        ];
        let (cpu, memory) = sum_pod_allocation(&pods);
        assert_eq!(cpu, 3_000);
        assert_eq!(memory, 3 * 1024 * 1024 * 1024);
    }

    #[test]
    fn terminal_pods_excluded() {
        let pods = vec![
            pod_with(Some("Running"), "5", "5Gi"),
            pod_with(Some("Succeeded"), "100", "100Gi"),
            pod_with(Some("Failed"), "100", "100Gi"),
        ];
        let (cpu, memory) = sum_pod_allocation(&pods);
        assert_eq!(cpu, 5_000, "only the Running pod counts");
        assert_eq!(memory, 5 * 1024 * 1024 * 1024);
    }

    #[test]
    fn no_pods_is_zero() {
        assert_eq!(sum_pod_allocation(Vec::<Pod>::new().iter()), (0, 0));
    }

    // ---- build_allocation_status ----

    #[test]
    fn status_computes_ceiling_and_utilisation() {
        // supply 100 CPU / 200 GiB, budget 80% → ceiling 80 CPU / 160 GiB.
        // allocated 70 CPU / 110 GiB.
        let status = build_allocation_status(
            (70_000, 110 * 1024 * 1024 * 1024),
            (100_000, 200 * 1024 * 1024 * 1024),
            80,
        );
        assert_eq!(status.ceiling_cpu_milli, 80_000);
        assert_eq!(status.ceiling_memory_bytes, 160 * 1024 * 1024 * 1024);
        assert!((status.utilization_percent_cpu - 0.875).abs() < 1e-9);
        assert!((status.utilization_percent_memory - (110.0 / 160.0)).abs() < 1e-9);
        assert!(status.last_updated.ends_with('Z'));
    }

    #[test]
    fn zero_budget_yields_zero_ceiling() {
        let status = build_allocation_status((10_000, 10_000), (100_000, 100_000), 0);
        assert_eq!(status.ceiling_cpu_milli, 0);
        assert_eq!(status.ceiling_memory_bytes, 0);
        assert_eq!(status.utilization_percent_cpu, 0.0);
    }
}
