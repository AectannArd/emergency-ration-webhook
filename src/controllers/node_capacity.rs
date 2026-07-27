//! Node Capacity Controller (T018) — the *supply* side.
//!
//! Watches nodes and keeps the `cluster-capacity` `ClusterCapacity` CRD's
//! `.status` equal to the sum of every node's `.status.allocatable`. Read-only on
//! nodes; never interrupts node lifecycle (Principle V). See
//! `contracts/clustercapacity-crd.md` §Controller Behaviour.

use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::{ListParams, Patch, PatchParams, PostParams};
use kube::runtime::{reflector, watcher};
use kube::{Api, Client};
use tracing::{debug, info, warn};

use super::node_filter::{ExclusionBreakdown, is_node_counted, validate_selector};
use crate::crd::{
    CLUSTER_CAPACITY_NAME, ClusterCapacity, ClusterCapacitySpec, ClusterCapacityStatus,
};
use crate::resources::quantity::{parse_cpu, parse_memory};
use crate::time_util::now_rfc3339;

/// Sum `cpu` (→ milli-CPUs) and `memory` (→ bytes) from every node's
/// `.status.allocatable`, applying the spec-006 node filter first. Pure: takes
/// references, no client, exhaustively tested.
///
/// The filter excludes (1) unschedulable nodes (`spec.unschedulable = true`,
/// always — FR-001) and (2) nodes matching the optional `selector` (FR-003). A
/// node counted toward capacity passes both layers (FR-004). An excluded node
/// still appears in the [`ExclusionBreakdown`] so the controller can report
/// *why* capacity changed (spec-006 US3).
///
/// A node missing `.status.allocatable` (e.g. NotReady, no reported capacity) is
/// subject to the exclusion checks for counting but, if it passes them,
/// contributes nothing to the CPU/memory sum and is not counted (existing
/// behaviour). Individual unparseable quantities are skipped — node allocatable
/// is kubelet-authored and always well-formed in practice.
pub fn sum_node_allocatable<'a, I>(
    nodes: I,
    selector: Option<&LabelSelector>,
) -> (i64, i64, i32, ExclusionBreakdown)
where
    I: IntoIterator<Item = &'a Node>,
{
    // Validate the selector once for the whole cycle (FR-010): an invalid
    // selector is dropped to None so the cycle falls back to unschedulable-only
    // exclusion — capacity tracking continues, the filter is never applied with
    // a partial/invalid match. Re-validated on every event, so a corrected
    // selector takes effect immediately.
    let selector = effective_selector(selector);
    let mut cpu = 0i64;
    let mut memory = 0i64;
    let mut breakdown = ExclusionBreakdown::default();
    for node in nodes {
        let unschedulable = node
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false);
        let labels = node.metadata.labels.as_ref();
        if !is_node_counted(unschedulable, labels, selector) {
            // Attribute to the layer that excluded it. Unschedulable is checked
            // first inside is_node_counted, so a node excluded by both counts
            // under excluded_unschedulable only (no double-count).
            if unschedulable {
                breakdown.excluded_unschedulable += 1;
            } else {
                breakdown.excluded_by_selector += 1;
            }
            continue;
        }
        let Some(allocatable) = node.status.as_ref().and_then(|s| s.allocatable.as_ref()) else {
            // Counted candidate that reports no allocatable: contributes nothing
            // and is not counted (existing behaviour).
            continue;
        };
        breakdown.counted += 1;
        if let Some(q) = allocatable.get("cpu") {
            cpu += parse_cpu(&q.0).unwrap_or(0);
        }
        if let Some(q) = allocatable.get("memory") {
            memory += parse_memory(&q.0).unwrap_or(0);
        }
    }
    (cpu, memory, breakdown.counted, breakdown)
}

/// Validate the configured selector once per reconciliation cycle (spec-006
/// FR-010). A structurally invalid selector is logged and dropped to `None` so
/// the cycle falls back to unschedulable-only exclusion — the safe default that
/// keeps capacity tracking functional. Validity is re-checked on every event, so
/// a corrected selector takes effect immediately.
fn effective_selector(selector: Option<&LabelSelector>) -> Option<&LabelSelector> {
    let Some(sel) = selector else {
        return None;
    };
    match validate_selector(sel) {
        Ok(()) => Some(sel),
        Err(err) => {
            warn!(
                error = %err,
                "invalid ClusterCapacity spec.nodeSelector; \
                 falling back to unschedulable-only exclusion for this cycle"
            );
            None
        }
    }
}

/// Read the runtime `nodeSelector` from the `cluster-capacity` singleton spec
/// (FR-007/FR-011). Read on every reconciliation so a `kubectl patch` takes
/// effect on the next node event without a restart. Any failure (missing
/// singleton, transient error) falls back to `None` — unschedulable-only
/// exclusion — keeping capacity tracking functional.
async fn read_selector(capacity_api: &Api<ClusterCapacity>) -> Option<LabelSelector> {
    match capacity_api.get(CLUSTER_CAPACITY_NAME).await {
        Ok(cc) => cc.spec.node_selector,
        Err(err) if is_not_found(&err) => {
            // Singleton vanished mid-run; the patch will recreate it. Fall back
            // to no selector for this cycle.
            None
        }
        Err(err) => {
            debug!(
                %err,
                "failed to read ClusterCapacity spec.nodeSelector; \
                 using unschedulable-only exclusion for this cycle"
            );
            None
        }
    }
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
    ClusterCapacity::new(CLUSTER_CAPACITY_NAME, ClusterCapacitySpec { node_selector: None })
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

/// Recompute the aggregate and merge-patch the CRD's `.status` subresource,
/// including the spec-006 exclusion breakdown (US3).
///
/// If the singleton was deleted after startup the patch 404s; we recreate it
/// (via [`ensure_singleton`]) and retry once so this event's figures are not
/// lost. Any other failure is logged and retried on the next node event.
#[allow(clippy::too_many_arguments)]
pub async fn patch_status(
    api: &Api<ClusterCapacity>,
    cpu: i64,
    memory: i64,
    node_count: i32,
    excluded_node_count: i32,
    excluded_by_unschedulable: i32,
    excluded_by_selector: i32,
) {
    let params = PatchParams::default();
    match patch_once(
        api,
        &params,
        cpu,
        memory,
        node_count,
        excluded_node_count,
        excluded_by_unschedulable,
        excluded_by_selector,
    )
    .await
    {
        Ok(_) => debug!(
            node_count,
            cpu_milli = cpu,
            memory_bytes = memory,
            excluded_node_count,
            excluded_by_unschedulable,
            excluded_by_selector,
            "patched ClusterCapacity status"
        ),
        Err(err) if is_not_found(&err) => {
            warn!(
                %err,
                "ClusterCapacity singleton missing during patch; recreating and retrying once"
            );
            ensure_singleton(api).await;
            match patch_once(
                api,
                &params,
                cpu,
                memory,
                node_count,
                excluded_node_count,
                excluded_by_unschedulable,
                excluded_by_selector,
            )
            .await
            {
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
#[allow(clippy::too_many_arguments)]
async fn patch_once(
    api: &Api<ClusterCapacity>,
    params: &PatchParams,
    cpu: i64,
    memory: i64,
    node_count: i32,
    excluded_node_count: i32,
    excluded_by_unschedulable: i32,
    excluded_by_selector: i32,
) -> kube::Result<ClusterCapacity> {
    let status = ClusterCapacityStatus {
        total_allocatable_cpu_milli: cpu,
        total_allocatable_memory_bytes: memory,
        node_count,
        last_updated: now_rfc3339(),
        excluded_node_count,
        excluded_by_unschedulable,
        excluded_by_selector,
    };
    api.patch_status(
        CLUSTER_CAPACITY_NAME,
        params,
        &Patch::Merge(super::status_merge_patch(&status)),
    )
    .await
}

/// Bootstrap reconcile: read nodes directly and patch the aggregate immediately,
/// rather than waiting for the reflector's first watch event. The reflector
/// cache is empty at startup; without this the singleton stays status-less until
/// the initial node list is delivered (and the Allocation Controller, which
/// derives its ceiling from this supply, then has nothing to compute from).
/// Later node events from the reflector keep the status fresh.
async fn reconcile_now(nodes: &Api<Node>, capacity_api: &Api<ClusterCapacity>) {
    // spec-006 US2 (T032): read the runtime nodeSelector from the singleton spec
    // so a kubectl patch takes effect without a restart (FR-007/FR-011).
    let selector = read_selector(capacity_api).await;
    let (cpu, memory, node_count, breakdown) = match nodes.list(&ListParams::default()).await {
        Ok(list) => sum_node_allocatable(&list.items, selector.as_ref()),
        Err(err) => {
            warn!(%err, "initial node list failed; deferring to watch events");
            (0, 0, 0, ExclusionBreakdown::default())
        }
    };
    patch_status(
        capacity_api,
        cpu,
        memory,
        node_count,
        breakdown.excluded_node_count(),
        breakdown.excluded_unschedulable,
        breakdown.excluded_by_selector,
    )
    .await;
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

    // Write the aggregate now from a direct node list (the reflector cache is
    // still cold) so the singleton has status before the first watch event.
    reconcile_now(&nodes, &capacity_api).await;

    let stream = reflector::reflector(writer, watcher::watcher(nodes, watcher::Config::default()));
    stream
        .for_each(|event| {
            let store = store.clone();
            let capacity_api = capacity_api.clone();
            async move {
                match event {
                    Ok(_) => {
                        let snapshot = store.state();
                        // spec-006 US2 (T032): read the runtime nodeSelector on
                        // each event so spec patches take effect immediately.
                        let selector = read_selector(&capacity_api).await;
                        let (cpu, memory, node_count, breakdown) = sum_node_allocatable(
                            snapshot.iter().map(|node| node.as_ref()),
                            selector.as_ref(),
                        );
                        patch_status(
                            &capacity_api,
                            cpu,
                            memory,
                            node_count,
                            breakdown.excluded_node_count(),
                            breakdown.excluded_unschedulable,
                            breakdown.excluded_by_selector,
                        )
                        .await;
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
        let (cpu, memory, count, _) = sum_node_allocatable(&nodes, None);
        assert_eq!(cpu, 24_000); // (16 + 8) cores
        assert_eq!(memory, 48 * 1024 * 1024 * 1024); // 48 GiB
        assert_eq!(count, 2);
    }

    #[test]
    fn skips_nodes_without_status() {
        let bare = Node::default();
        let healthy = node("a", "4", "8Gi");
        let (cpu, memory, count, _) = sum_node_allocatable(&[bare, healthy], None);
        assert_eq!(cpu, 4_000);
        assert_eq!(memory, 8 * 1024 * 1024 * 1024);
        assert_eq!(count, 1, "the status-less node is not counted");
    }

    #[test]
    fn empty_cluster_is_zero() {
        let (cpu, memory, count, _) = sum_node_allocatable(Vec::<Node>::new().iter(), None);
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
        let (cpu, memory, count, _) = sum_node_allocatable(&[n], None);
        assert_eq!(cpu, 2_000);
        assert_eq!(memory, 0);
        assert_eq!(count, 1);
    }

    // ---- spec-006 US1: unschedulable nodes excluded from the aggregate ----

    /// Build a node with the given allocatable that is cordoned
    /// (`spec.unschedulable = true`).
    fn cordoned(name: &str, cpu: &str, memory: &str) -> Node {
        let mut n = node(name, cpu, memory);
        n.spec = Some(k8s_openapi::api::core::v1::NodeSpec {
            unschedulable: Some(true),
            ..Default::default()
        });
        n
    }

    /// Build a schedulable node carrying the given labels (for selector tests).
    fn labeled(name: &str, cpu: &str, memory: &str, labels: &[(&str, &str)]) -> Node {
        let mut n = node(name, cpu, memory);
        n.metadata.labels = Some(
            labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        );
        n
    }

    #[test]
    fn sum_excludes_unschedulable_nodes() {
        // 3 nodes, one cordoned. The aggregate reflects only the 2 schedulable
        // nodes; the breakdown reports 1 excluded-by-unschedulable (FR-001).
        use crate::controllers::node_filter::ExclusionBreakdown;
        let nodes = vec![node("a", "8", "16Gi"), node("b", "4", "8Gi"), cordoned("cp", "16", "32Gi")];
        let (cpu, memory, count, breakdown) = sum_node_allocatable(&nodes, None);
        assert_eq!(cpu, 12_000, "only the 2 schedulable nodes' CPU counts");
        assert_eq!(memory, 24 * 1024 * 1024 * 1024);
        assert_eq!(count, 2);
        assert_eq!(
            breakdown,
            ExclusionBreakdown {
                counted: 2,
                excluded_unschedulable: 1,
                excluded_by_selector: 0,
            }
        );
        assert_eq!(breakdown.excluded_node_count(), 1);
    }

    #[test]
    fn all_unschedulable_cluster_is_zero_capacity() {
        // Constitution Principle I interaction: when every node is excluded,
        // capacity drops to zero — the webhook then fails closed on every
        // admission. This is correct, not a bug.
        let nodes = vec![cordoned("a", "16", "32Gi"), cordoned("b", "16", "32Gi")];
        let (cpu, memory, count, breakdown) = sum_node_allocatable(&nodes, None);
        assert_eq!((cpu, memory, count), (0, 0, 0));
        assert_eq!(breakdown.counted, 0);
        assert_eq!(breakdown.excluded_unschedulable, 2);
        assert_eq!(breakdown.excluded_by_selector, 0);
        assert_eq!(breakdown.excluded_node_count(), 2);
    }

    // ---- spec-006 US2: label-selector exclusion ----

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};

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

    #[test]
    fn sum_excludes_nodes_matching_label_selector() {
        // T026: 2 workers + 1 control-plane node; the selector excludes the
        // control-plane node → aggregate reflects the 2 workers, breakdown
        // reports 1 excluded-by-selector (FR-003).
        use crate::controllers::node_filter::ExclusionBreakdown;
        let sel = control_plane_selector();
        let nodes = vec![
            labeled("w1", "8", "16Gi", &[("role", "worker")]),
            labeled("w2", "8", "16Gi", &[("role", "worker")]),
            labeled(
                "cp",
                "16",
                "32Gi",
                &[("node-role.kubernetes.io/control-plane", "")],
            ),
        ];
        let (cpu, memory, count, breakdown) = sum_node_allocatable(&nodes, Some(&sel));
        assert_eq!(cpu, 16_000, "only the 2 workers' CPU counts");
        assert_eq!(memory, 32 * 1024 * 1024 * 1024);
        assert_eq!(count, 2);
        assert_eq!(
            breakdown,
            ExclusionBreakdown {
                counted: 2,
                excluded_unschedulable: 0,
                excluded_by_selector: 1,
            }
        );
        assert_eq!(breakdown.excluded_node_count(), 1);
    }

    #[test]
    fn invalid_selector_falls_back_to_unschedulable_only() {
        // T027 / FR-010: a structurally invalid selector (unknown operator) is
        // ignored for this cycle — no selector-based exclusion — while
        // unschedulable nodes are still excluded (the safe default).
        let invalid = LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "role".to_string(),
                operator: "Matches".to_string(),
                values: None,
            }]),
        };
        let nodes = vec![
            labeled("w1", "8", "16Gi", &[("role", "worker")]),
            cordoned("cp", "16", "32Gi"),
        ];
        let (cpu, memory, count, breakdown) = sum_node_allocatable(&nodes, Some(&invalid));
        // Fallback: the worker is counted (no selector applied), the cordoned
        // node excluded by unschedulable.
        assert_eq!((cpu, memory, count), (8_000, 16 * 1024 * 1024 * 1024, 1));
        assert_eq!(
            breakdown.excluded_by_selector, 0,
            "invalid selector must not exclude any node"
        );
        assert_eq!(breakdown.excluded_unschedulable, 1);
    }

    #[test]
    fn effective_selector_passes_valid_and_drops_invalid() {
        // T031: the per-cycle selector decision. A valid selector passes through;
        // an invalid one is dropped to None (FR-010 fallback); None stays None.
        let valid = control_plane_selector();
        assert_eq!(effective_selector(Some(&valid)), Some(&valid));
        let invalid = LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "role".to_string(),
                operator: "Bogus".to_string(),
                values: None,
            }]),
        };
        assert_eq!(effective_selector(Some(&invalid)), None);
        assert_eq!(effective_selector(None), None);
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
            ClusterCapacitySpec { node_selector: None },
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
    fn default_singleton_spec_has_no_node_selector() {
        // spec-006: the auto-created singleton must not set a nodeSelector —
        // unschedulable-only exclusion is the default (FR-005). An operator's
        // later patch is the only thing that populates it.
        let cc = default_capacity_singleton();
        assert!(
            cc.spec.node_selector.is_none(),
            "auto-created singleton must not set a nodeSelector (FR-005 default)"
        );
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
            patch_status(&api, 16_000, 32 * 1024 * 1024 * 1024, 1, 0, 0, 0).await;
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

    #[tokio::test]
    async fn reconcile_now_lists_nodes_then_patches_status() {
        // The bootstrap reconcile must write status immediately on startup,
        // independent of the reflector: it lists nodes, sums their allocatable,
        // and patches the singleton — so the supply is published before the
        // first watch event ever arrives.
        let (client, mut handle) = mock_client();
        let nodes = Api::<Node>::all(client.clone());
        let capacity_api = Api::<ClusterCapacity>::all(client);
        let task = tokio::spawn(async move {
            reconcile_now(&nodes, &capacity_api).await;
        });

        // 1. spec-006: read the runtime nodeSelector — GET the singleton (here
        // the default, no nodeSelector → unschedulable-only exclusion).
        let (req, respond) = handle.next_request().await.expect("capacity GET");
        assert_eq!(req.method().as_str(), "GET");
        assert!(
            req.uri().path().ends_with("/cluster-capacity"),
            "GET targets the singleton for the selector read: {}",
            req.uri()
        );
        respond.send_response(ok_object(&default_capacity_singleton()));

        // 2. Node LIST → one node with 16 CPU / 32 GiB allocatable.
        let (req, respond) = handle.next_request().await.expect("node LIST");
        assert_eq!(req.method().as_str(), "GET");
        assert!(
            req.uri().path().ends_with("/nodes"),
            "LIST targets nodes: {}",
            req.uri()
        );
        let node_list = serde_json::json!({
            "apiVersion": "v1",
            "kind": "NodeList",
            "items": [{
                "metadata": {"name": "a"},
                "status": {"allocatable": {"cpu": "16", "memory": "32Gi"}}
            }]
        });
        respond.send_response(ok_object(&node_list));

        // 3. Status PATCH on the singleton → 200. The body must carry the
        // aggregate under a top-level "status" key: a bare status object is a
        // silent no-op on the /status subresource (patch returns 200, nothing
        // persists). Assert both the envelope and the summed figures.
        let (req, respond) = handle.next_request().await.expect("status PATCH");
        assert_eq!(req.method().as_str(), "PATCH");
        let path = req.uri().path().to_string();
        assert!(
            path.ends_with("/cluster-capacity/status"),
            "PATCH targets the ClusterCapacity status subresource: {path}"
        );
        let body = http_body_util::BodyExt::collect(req.into_body())
            .await
            .expect("patch body collects")
            .to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("patch body is JSON");
        assert!(
            payload.get("status").is_some(),
            "PATCH body must wrap status under \"status\": {payload}"
        );
        assert_eq!(
            payload["status"]["totalAllocatableCpuMilli"].as_i64(),
            Some(16_000)
        );
        assert_eq!(
            payload["status"]["totalAllocatableMemoryBytes"].as_i64(),
            Some(32 * 1024 * 1024 * 1024)
        );
        assert_eq!(payload["status"]["nodeCount"].as_i64(), Some(1));
        // spec-006: the exclusion breakdown is always patched (here 0 — the
        // single mock node is schedulable and no selector is configured).
        assert_eq!(payload["status"]["excludedNodeCount"].as_i64(), Some(0));
        assert_eq!(payload["status"]["excludedByUnschedulable"].as_i64(), Some(0));
        assert_eq!(payload["status"]["excludedBySelector"].as_i64(), Some(0));
        respond.send_response(ok_object(&default_capacity_singleton()));

        task.await.expect("reconcile_now did not panic");
    }
}
