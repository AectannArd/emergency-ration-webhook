//! BDD step definitions for the schedulable node filter (spec-006 T042-T043).
//!
//! Drives the real [`sum_node_allocatable`] aggregation through Cucumber
//! expressions so the three user-story acceptance scenarios read as Gherkin. The
//! `World` materialises a node set + an optional selector from the scenario's
//! Given steps, reconciles once (lazily, on the first assertion), and asserts on
//! the counted node count and the exclusion breakdown.
//!
//! Run with: `cargo test --test node_filter_bdd`.

use std::collections::BTreeMap;

use capacity_admission_webhook::controllers::{ExclusionBreakdown, sum_node_allocatable};
use cucumber::{World as _, given, then, when};
use k8s_openapi::api::core::v1::{Node, NodeSpec, NodeStatus};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};

/// Build a node with the given allocatable, labels, and cordoned state.
fn make_node(name: &str, cpu: &str, memory: &str, labels: &[(&str, &str)], cordoned: bool) -> Node {
    let mut allocatable = BTreeMap::new();
    allocatable.insert("cpu".to_string(), Quantity(cpu.to_string()));
    allocatable.insert("memory".to_string(), Quantity(memory.to_string()));
    let mut node = Node {
        status: Some(NodeStatus {
            allocatable: Some(allocatable),
            ..Default::default()
        }),
        ..Default::default()
    };
    node.metadata.name = Some(name.to_string());
    if !labels.is_empty() {
        node.metadata.labels = Some(
            labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        );
    }
    if cordoned {
        node.spec = Some(NodeSpec {
            unschedulable: Some(true),
            ..Default::default()
        });
    }
    node
}

#[derive(cucumber::World, Default)]
struct NodeFilterWorld {
    nodes: Vec<Node>,
    selectors: Option<Vec<LabelSelector>>,
    /// Lazily-computed aggregate: (cpu, memory, counted, breakdown).
    result: Option<(i64, i64, i32, ExclusionBreakdown)>,
}

impl std::fmt::Debug for NodeFilterWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeFilterWorld")
            .field("nodes", &self.nodes.len())
            .field("has_selectors", &self.selectors.is_some())
            .field("counted", &self.result.as_ref().map(|r| r.2))
            .finish()
    }
}

impl NodeFilterWorld {
    /// Run the aggregation once; idempotent so any order of When/Then steps works.
    fn reconcile(&mut self) {
        if self.result.is_none() {
            self.result = Some(sum_node_allocatable(&self.nodes, self.selectors.as_deref()));
        }
    }
}

// ---- Given ----

#[given(expr = "a cluster with {int} schedulable nodes each with {int} CPU and {int}Gi memory")]
fn schedulable_nodes(world: &mut NodeFilterWorld, count: i64, cpu: i64, mem_gib: i64) {
    let cpu = format!("{cpu}");
    let memory = format!("{mem_gib}Gi");
    for i in 0..count {
        world
            .nodes
            .push(make_node(&format!("node-{i}"), &cpu, &memory, &[], false));
    }
}

#[given(expr = "a cluster with {int} worker nodes and {int} control-plane node")]
fn worker_and_control_plane_nodes(world: &mut NodeFilterWorld, workers: i64, control_plane: i64) {
    for i in 0..workers {
        world.nodes.push(make_node(
            &format!("worker-{i}"),
            "8",
            "16Gi",
            &[("role", "worker")],
            false,
        ));
    }
    for i in 0..control_plane {
        world.nodes.push(make_node(
            &format!("control-plane-{i}"),
            "16",
            "32Gi",
            &[("node-role.kubernetes.io/control-plane", "")],
            false,
        ));
    }
}

#[given(
    expr = "a cluster with {int} nodes where {int} is cordoned and {int} matches the nodeSelector"
)]
fn mixed_cluster(world: &mut NodeFilterWorld, total: i64, cordoned: i64, matched: i64) {
    let workers = total - cordoned - matched;
    for i in 0..workers {
        world.nodes.push(make_node(
            &format!("worker-{i}"),
            "8",
            "16Gi",
            &[("role", "worker")],
            false,
        ));
    }
    for i in 0..cordoned {
        world
            .nodes
            .push(make_node(&format!("cordoned-{i}"), "16", "32Gi", &[], true));
    }
    for i in 0..matched {
        world.nodes.push(make_node(
            &format!("control-plane-{i}"),
            "16",
            "32Gi",
            &[("node-role.kubernetes.io/control-plane", "")],
            false,
        ));
    }
}

#[given(expr = "the nodeSelector excludes nodes labeled {string}")]
fn node_selector_excludes_label(world: &mut NodeFilterWorld, label: String) {
    world.selectors = Some(vec![LabelSelector {
        match_labels: None,
        match_expressions: Some(vec![LabelSelectorRequirement {
            key: label,
            operator: "Exists".to_string(),
            values: None,
        }]),
    }]);
}

// ---- spec-007: multi-selector OR scenarios ----

#[given(
    expr = "a cluster with {int} worker nodes, {int} control-plane node, and {int} experimental node"
)]
fn worker_control_plane_and_experimental_nodes(
    world: &mut NodeFilterWorld,
    workers: i64,
    control_plane: i64,
    experimental: i64,
) {
    for i in 0..workers {
        world.nodes.push(make_node(
            &format!("worker-{i}"),
            "8",
            "16Gi",
            &[("role", "worker")],
            false,
        ));
    }
    for i in 0..control_plane {
        world.nodes.push(make_node(
            &format!("control-plane-{i}"),
            "16",
            "32Gi",
            &[("node-role.kubernetes.io/control-plane", "")],
            false,
        ));
    }
    for i in 0..experimental {
        world.nodes.push(make_node(
            &format!("experimental-{i}"),
            "16",
            "32Gi",
            &[("node-type/experimental", "")],
            false,
        ));
    }
}

#[given(expr = "the nodeSelectors exclude control-plane and experimental nodes")]
fn node_selectors_exclude_control_plane_and_experimental(world: &mut NodeFilterWorld) {
    // spec-007 OR semantics: two selectors — a node matching EITHER is excluded.
    world.selectors = Some(vec![
        LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "node-role.kubernetes.io/control-plane".to_string(),
                operator: "Exists".to_string(),
                values: None,
            }]),
        },
        LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "node-type/experimental".to_string(),
                operator: "Exists".to_string(),
                values: None,
            }]),
        },
    ]);
}

// ---- When ----

#[when("one node is cordoned")]
fn cordon_one_node(world: &mut NodeFilterWorld) {
    if let Some(node) = world
        .nodes
        .iter_mut()
        .find(|n| n.spec.as_ref().and_then(|s| s.unschedulable) != Some(true))
    {
        node.spec = Some(NodeSpec {
            unschedulable: Some(true),
            ..Default::default()
        });
    }
}

#[when("the controller reconciles")]
fn reconcile(world: &mut NodeFilterWorld) {
    world.reconcile();
}

// ---- Then ----

#[then(expr = "the status reports nodeCount {int}")]
fn status_node_count(world: &mut NodeFilterWorld, expected: i64) {
    world.reconcile();
    let (_, _, counted, _) = world.result.as_ref().expect("reconciled");
    assert_eq!(*counted as i64, expected, "expected nodeCount {expected}");
}

#[then(expr = "the excludedByUnschedulable count is {int}")]
fn excluded_by_unschedulable(world: &mut NodeFilterWorld, expected: i64) {
    world.reconcile();
    let (_, _, _, breakdown) = world.result.as_ref().expect("reconciled");
    assert_eq!(
        breakdown.excluded_unschedulable as i64, expected,
        "expected excludedByUnschedulable {expected}"
    );
}

#[then(expr = "the excludedBySelector count is {int}")]
fn excluded_by_selector(world: &mut NodeFilterWorld, expected: i64) {
    world.reconcile();
    let (_, _, _, breakdown) = world.result.as_ref().expect("reconciled");
    assert_eq!(
        breakdown.excluded_by_selector as i64, expected,
        "expected excludedBySelector {expected}"
    );
}

#[then(expr = "the excludedNodeCount is {int}")]
fn excluded_node_count(world: &mut NodeFilterWorld, expected: i64) {
    world.reconcile();
    let (_, _, _, breakdown) = world.result.as_ref().expect("reconciled");
    assert_eq!(
        breakdown.excluded_node_count() as i64,
        expected,
        "expected excludedNodeCount {expected}"
    );
}

#[tokio::main]
async fn main() {
    NodeFilterWorld::run("tests/bdd/features/node_filter.feature").await;
}
