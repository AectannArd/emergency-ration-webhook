//! Multi-cluster capacity equalizer (spec-013).
//!
//! A fleet-level controller that reads capacity/utilisation from N target
//! clusters (each via its own kubeconfig `Secret`), computes a fleet-wide budget
//! per resource via a pure equalisation function, and writes the computed
//! per-resource override fields (`cpuBudgetPercent` / `memoryBudgetPercent`) back
//! to each target cluster's `Allocation` singleton.
//!
//! This module is NOT on the admission path (Constitution Principle I does not
//! apply — the equalizer never admits or denies pods; it only tunes budgets). The
//! foundational phase adds the pure + wiring split: the `EqualizerConfig` CRD
//! (`crd`), the unit-testable equalisation logic (`algorithm`), the per-target
//! `kube::Client` builder (`cluster_client`), and the reconcile loop
//! (`reconcile`).

pub mod algorithm;
pub mod cluster_client;
pub mod crd;

/// Boxed error alias for equalizer operations that can fail heterogeneously
/// (UTF-8 parse, kubeconfig parse, kube client/config errors). The reconcile loop
/// catches these per-cluster and records `ConfigError`/`Unreachable` in status —
/// it never propagates them out of a cycle.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
