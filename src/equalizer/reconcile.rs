//! The equalizer reconcile loop (spec-013, data-model.md §3).
//!
//! [`reconcile`] is the full read → compute → patch cycle for ONE tick: it reads
//! each target cluster's kubeconfig `Secret` from the home cluster, builds a
//! per-target `kube::Client`, GETs the target's `Allocation` (utilization) +
//! `ClusterCapacity` (allocatable) status, computes budgets via the pure
//! [`equalize`](crate::equalizer::algorithm::equalize) function (once per resource
//! dimension), PATCHes the target's `Allocation.spec` per-resource overrides, and
//! returns the [`EqualizerConfigStatus`] for the caller to persist. It is
//! infallible at the top level: per-cluster failures (Secret missing, kubeconfig
//! malformed, apiserver unreachable) are recorded in status as `ConfigError` /
//! `Unreachable`, and the cycle continues with the remaining clusters (FR-009).

use std::collections::HashMap;

use futures::future::join_all;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use tracing::warn;

use crate::crd::{Allocation, CLUSTER_ALLOCATION_NAME, CLUSTER_CAPACITY_NAME, ClusterCapacity};
use crate::equalizer::algorithm::{
    BudgetState, ClusterResourceObservation, ComputedBudget, equalize,
};
use crate::equalizer::cluster_client::build_target_client;
use crate::equalizer::crd::{
    ClusterObservation, ClusterState, EqualizerConfig, EqualizerConfigSpec, EqualizerConfigStatus,
    TargetCluster, fleet_condition,
};
use crate::time_util::now_rfc3339;

/// The result of resolving one target cluster: either its observed state (+ a
/// live client), or a classified failure.
enum ResolvedTarget {
    /// The cluster was reached and read; `client` is kept to patch it.
    Read {
        name: String,
        client: Client,
        cpu_util: f64,
        mem_util: f64,
        cpu_alloc: i64,
        mem_alloc: i64,
    },
    /// The cluster could not be reached; recorded in status as `state`.
    Failed {
        name: String,
        state: ClusterState,
        error: String,
    },
}

/// Run one equalizer reconcile cycle. See the module docs for the full state
/// machine. Pure of top-level error handling: every per-cluster failure lands in
/// the returned status, so a single bad target never aborts the fleet cycle.
pub async fn reconcile(home_client: &Client, eq_config: &EqualizerConfig) -> EqualizerConfigStatus {
    let spec = &eq_config.spec;
    let now = now_rfc3339();

    // Phase A — resolve client + read state for every target, concurrently. A
    // slow/unreachable target only delays its own resolution, not the fleet.
    let reads: Vec<ResolvedTarget> =
        join_all(spec.targets.iter().map(|t| resolve_target(home_client, t))).await;

    // Phase B — equalize CPU and memory independently over the reachable clusters
    // (FR-014). Failed clusters contribute no observation.
    let cpu_observations: Vec<ClusterResourceObservation> =
        reads.iter().filter_map(cpu_observation).collect();
    let mem_observations: Vec<ClusterResourceObservation> =
        reads.iter().filter_map(memory_observation).collect();
    let cpu_budgets = equalize(&cpu_observations, spec.cpu_target_budget_percent);
    let mem_budgets = equalize(&mem_observations, spec.memory_target_budget_percent);
    let cpu_by_name: HashMap<&str, &ComputedBudget> =
        cpu_budgets.iter().map(|b| (b.name.as_str(), b)).collect();
    let mem_by_name: HashMap<&str, &ComputedBudget> =
        mem_budgets.iter().map(|b| (b.name.as_str(), b)).collect();

    // Phase C — patch the computed overrides onto every reachable target,
    // concurrently. The patch sets ONLY the two override keys (FR-007); a patch
    // failure is logged, not fatal (the computed budget is still reported in
    // status and retried next cycle).
    let patches: Vec<_> = reads
        .iter()
        .filter_map(|r| match r {
            ResolvedTarget::Read { name, client, .. } => {
                let cpu = cpu_by_name
                    .get(name.as_str())
                    .map(|b| b.budget_percent)
                    .unwrap_or(spec.cpu_target_budget_percent);
                let mem = mem_by_name
                    .get(name.as_str())
                    .map(|b| b.budget_percent)
                    .unwrap_or(spec.memory_target_budget_percent);
                Some(patch_overrides(client, name, cpu, mem))
            }
            ResolvedTarget::Failed { .. } => None,
        })
        .collect();
    for result in join_all(patches).await {
        if let Err((cluster, err)) = result {
            warn!(%cluster, %err, "failed to patch Allocation overrides; retrying next cycle");
        }
    }

    // Phase D — build the status, one observation per target in spec order. Log
    // any unresolved target so operators see unreachable/config-error clusters in
    // the logs, not only in the status.
    let clusters: Vec<ClusterObservation> = spec
        .targets
        .iter()
        .zip(reads.iter())
        .map(|(target, read)| {
            if let ResolvedTarget::Failed {
                name, state, error, ..
            } = read
            {
                warn!(cluster = %name, state = ?state, %error, "target cluster unresolved this cycle");
            }
            build_observation(target, read, &cpu_by_name, &mem_by_name, spec, &now)
        })
        .collect();
    let states: Vec<ClusterState> = clusters.iter().map(|c| c.state).collect();
    let condition = fleet_condition(&states);
    EqualizerConfigStatus {
        clusters,
        condition,
        last_reconciled: now,
    }
}

/// Resolve one target: read its kubeconfig `Secret` from the home cluster, build
/// the target client, then GET its `Allocation` + `ClusterCapacity` status. Maps
/// each failure to the contract's classification (Secret/kubeconfig →
/// `ConfigError`; apiserver → `Unreachable`).
async fn resolve_target(home_client: &Client, target: &TargetCluster) -> ResolvedTarget {
    let name = target.name.clone();
    let secret_ref = &target.kubeconfig_secret_ref;

    // 1. Read the kubeconfig Secret from the home cluster (missing/forbidden →
    //    ConfigError — the EqualizerConfig references a Secret that isn't usable).
    let secret = {
        let secrets = Api::<Secret>::namespaced(home_client.clone(), &secret_ref.namespace);
        match secrets.get(&secret_ref.name).await {
            Ok(s) => s,
            Err(err) => {
                return ResolvedTarget::Failed {
                    name,
                    state: ClusterState::ConfigError,
                    error: format!(
                        "reading kubeconfig Secret `{}/{}`: {err}",
                        secret_ref.namespace, secret_ref.name
                    ),
                };
            }
        }
    };

    // 2. Extract the kubeconfig bytes (key absent → ConfigError).
    let kubeconfig_bytes: &[u8] = match secret.data.as_ref().and_then(|d| d.get(&secret_ref.key)) {
        Some(bytes) => &bytes.0,
        None => {
            return ResolvedTarget::Failed {
                name,
                state: ClusterState::ConfigError,
                error: format!(
                    "key `{}` not found in Secret `{}/{}`",
                    secret_ref.key, secret_ref.namespace, secret_ref.name
                ),
            };
        }
    };

    // 3. Build the target client (malformed/unusable kubeconfig → ConfigError).
    let client = match build_target_client(kubeconfig_bytes).await {
        Ok(c) => c,
        Err(err) => {
            return ResolvedTarget::Failed {
                name,
                state: ClusterState::ConfigError,
                error: format!("building target client from kubeconfig: {err}"),
            };
        }
    };

    // 4. GET the Allocation singleton → utilization (apiserver issue → Unreachable).
    let (cpu_util, mem_util) = match read_allocation_utilization(&client).await {
        Ok(u) => u,
        Err(err) => {
            return ResolvedTarget::Failed {
                name,
                state: ClusterState::Unreachable,
                error: err,
            };
        }
    };

    // 5. GET the ClusterCapacity singleton → allocatable.
    let (cpu_alloc, mem_alloc) = match read_cluster_capacity(&client).await {
        Ok(a) => a,
        Err(err) => {
            return ResolvedTarget::Failed {
                name,
                state: ClusterState::Unreachable,
                error: err,
            };
        }
    };

    ResolvedTarget::Read {
        name,
        client,
        cpu_util,
        mem_util,
        cpu_alloc,
        mem_alloc,
    }
}

/// Read the target's CPU/memory utilization from its `Allocation` status.
async fn read_allocation_utilization(client: &Client) -> Result<(f64, f64), String> {
    let allocations = Api::<Allocation>::all(client.clone());
    let alloc = allocations
        .get(CLUSTER_ALLOCATION_NAME)
        .await
        .map_err(|e| format!("reading Allocation status: {e}"))?;
    let status = alloc
        .status
        .as_ref()
        .ok_or_else(|| "Allocation has no status".to_string())?;
    Ok((
        status.utilization_percent_cpu,
        status.utilization_percent_memory,
    ))
}

/// Read the target's allocatable CPU/memory from its `ClusterCapacity` status.
async fn read_cluster_capacity(client: &Client) -> Result<(i64, i64), String> {
    let capacities = Api::<ClusterCapacity>::all(client.clone());
    let cc = capacities
        .get(CLUSTER_CAPACITY_NAME)
        .await
        .map_err(|e| format!("reading ClusterCapacity status: {e}"))?;
    let status = cc
        .status
        .as_ref()
        .ok_or_else(|| "ClusterCapacity has no status".to_string())?;
    Ok((
        status.total_allocatable_cpu_milli,
        status.total_allocatable_memory_bytes,
    ))
}

/// PATCH the target's `Allocation.spec` with ONLY the two per-resource override
/// fields (FR-007). Strategic/JSON merge-patch semantics leave `budgetPercent`,
/// `enforcementMode`, and every other field untouched. Returns the cluster name
/// alongside the error so the caller can log it.
async fn patch_overrides(
    client: &Client,
    name: &str,
    cpu_budget: i32,
    mem_budget: i32,
) -> Result<(), (String, String)> {
    let patch_body = serde_json::json!({
        "spec": {
            "cpuBudgetPercent": cpu_budget,
            "memoryBudgetPercent": mem_budget,
        }
    });
    let allocations = Api::<Allocation>::all(client.clone());
    allocations
        .patch(
            CLUSTER_ALLOCATION_NAME,
            &PatchParams::default(),
            &Patch::Merge(&patch_body),
        )
        .await
        .map(|_| ())
        .map_err(|e| {
            (
                name.to_string(),
                format!("patching Allocation overrides: {e}"),
            )
        })
}

/// Build the CPU observation for a resolved target (None if it failed).
fn cpu_observation(r: &ResolvedTarget) -> Option<ClusterResourceObservation> {
    match r {
        ResolvedTarget::Read {
            name,
            cpu_util,
            cpu_alloc,
            ..
        } => Some(ClusterResourceObservation {
            name: name.clone(),
            utilization_percent: *cpu_util,
            total_allocatable: *cpu_alloc,
        }),
        ResolvedTarget::Failed { .. } => None,
    }
}

/// Build the memory observation for a resolved target (None if it failed).
fn memory_observation(r: &ResolvedTarget) -> Option<ClusterResourceObservation> {
    match r {
        ResolvedTarget::Read {
            name,
            mem_util,
            mem_alloc,
            ..
        } => Some(ClusterResourceObservation {
            name: name.clone(),
            utilization_percent: *mem_util,
            total_allocatable: *mem_alloc,
        }),
        ResolvedTarget::Failed { .. } => None,
    }
}

/// Build the status observation for one target, classifying its state from the
/// computed budgets (Over in either resource → `Over`, else `Healthy`).
fn build_observation(
    target: &TargetCluster,
    read: &ResolvedTarget,
    cpu_by_name: &HashMap<&str, &ComputedBudget>,
    mem_by_name: &HashMap<&str, &ComputedBudget>,
    spec: &EqualizerConfigSpec,
    now: &str,
) -> ClusterObservation {
    let name = &target.name;
    match read {
        ResolvedTarget::Read {
            cpu_util,
            mem_util,
            cpu_alloc,
            mem_alloc,
            ..
        } => {
            let cpu_budget = cpu_by_name
                .get(name.as_str())
                .map(|b| b.budget_percent)
                .unwrap_or(spec.cpu_target_budget_percent);
            let mem_budget = mem_by_name
                .get(name.as_str())
                .map(|b| b.budget_percent)
                .unwrap_or(spec.memory_target_budget_percent);
            let over = cpu_by_name
                .get(name.as_str())
                .is_some_and(|b| b.state == BudgetState::Over)
                || mem_by_name
                    .get(name.as_str())
                    .is_some_and(|b| b.state == BudgetState::Over);
            ClusterObservation {
                name: name.clone(),
                cpu_utilization_percent: *cpu_util,
                memory_utilization_percent: *mem_util,
                total_allocatable_cpu_milli: *cpu_alloc,
                total_allocatable_memory_bytes: *mem_alloc,
                computed_cpu_budget_percent: cpu_budget,
                computed_memory_budget_percent: mem_budget,
                state: if over {
                    ClusterState::Over
                } else {
                    ClusterState::Healthy
                },
                last_error: None,
                last_observed: now.to_string(),
            }
        }
        ResolvedTarget::Failed { state, error, .. } => ClusterObservation {
            name: name.clone(),
            cpu_utilization_percent: 0.0,
            memory_utilization_percent: 0.0,
            total_allocatable_cpu_milli: 0,
            total_allocatable_memory_bytes: 0,
            // The cluster was not equalized this cycle (no observation), so no
            // budget was computed/applied; report zero rather than fabricate one.
            computed_cpu_budget_percent: 0,
            computed_memory_budget_percent: 0,
            state: *state,
            last_error: Some(error.clone()),
            last_observed: String::new(),
        },
    }
}
