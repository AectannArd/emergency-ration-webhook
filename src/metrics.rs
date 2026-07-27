//! Prometheus metric definitions and exposition (US2, T027/T029).
//!
//! Seven metrics are registered on a single [`Registry`] (data-model.md
//! §Metrics / research.md R10):
//!
//! | Metric | Type | Source |
//! |--------|------|--------|
//! | `capacity_admission_verdicts_total{resource,verdict}` | counter | webhook handler |
//! | `capacity_admission_decision_duration_seconds` | histogram | webhook handler |
//! | `capacity_admission_capacity_freshness_seconds` | gauge | Allocation `lastUpdated` |
//! | `capacity_admission_allocation_ratio{resource}` | gauge | Allocation `utilization_*` |
//! | `capacity_admission_total_allocatable{resource}` | gauge | ClusterCapacity status |
//! | `capacity_admission_current_allocation{resource}` | gauge | Allocation `allocated*` |
//! | `capacity_admission_ceiling{resource}` | gauge | Allocation `ceiling*` |
//! | `capacity_admission_exemptions_total{reason}` | counter | webhook handler (spec-008) |
//!
//! `resource` ∈ `{cpu, memory}`, `verdict` ∈ `{allow, deny, dry_run_deny, error}`,
//! `reason` ∈ `{namespace, priority_class, webhook_namespace}`.

use prometheus::{
    Encoder, GaugeVec, Histogram, HistogramOpts, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry, TextEncoder,
};

/// Which resource a metric series is labelled with. Serialized lower-case to
/// match the contract (`resource ∈ {cpu, memory}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLabel {
    Cpu,
    Memory,
}

impl ResourceLabel {
    /// Prometheus label value.
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceLabel::Cpu => "cpu",
            ResourceLabel::Memory => "memory",
        }
    }
}

/// A decision verdict, as recorded on `capacity_admission_verdicts_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictLabel {
    Allow,
    Deny,
    /// spec-004: a dry-run mode decision that would have denied but admitted
    /// with a warning. Distinct from `Deny` so dashboards can query
    /// `verdict="dry_run_deny"` separately or combine with `verdict=~"deny|dry_run_deny"`.
    DryRunDeny,
    Error,
}

impl VerdictLabel {
    /// Prometheus label value.
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictLabel::Allow => "allow",
            VerdictLabel::Deny => "deny",
            VerdictLabel::DryRunDeny => "dry_run_deny",
            VerdictLabel::Error => "error",
        }
    }
}

/// Per-resource capacity figures used to refresh the capacity gauges. Mirrors
/// the Allocation/ClusterCapacity status fields so the gauges reflect the exact
/// state used by the most recent admission decision (SC-003).
#[derive(Debug, Clone, Copy, Default)]
pub struct CapacityFigures {
    pub allocated: i64,
    pub ceiling: i64,
    pub total_allocatable: i64,
    /// `allocated / ceiling` (0.0 when there is no ceiling).
    pub ratio: f64,
}

/// All capacity-admission metrics on one registry. Cheap to share via `Arc`.
#[derive(Clone)]
pub struct Metrics {
    registry: Registry,
    verdicts: IntCounterVec,
    duration: Histogram,
    freshness: IntGauge,
    allocation_ratio: GaugeVec,
    total_allocatable: IntGaugeVec,
    current_allocation: IntGaugeVec,
    ceiling: IntGaugeVec,
    /// spec-008: pods admitted by exclusion policy, by reason.
    exemptions: IntCounterVec,
}

impl Metrics {
    /// Register all seven metrics on a fresh registry. Panics if a metric fails
    /// to register (a programming error — the names are compile-time constants).
    pub fn new() -> Self {
        let registry = Registry::new();

        let verdicts = IntCounterVec::new(
            Opts::new(
                "capacity_admission_verdicts_total",
                "Admission decisions by resource and verdict (allow/deny/dry_run_deny/error).",
            ),
            &["resource", "verdict"],
        )
        .expect("verdicts counter");
        let duration = Histogram::with_opts(
            HistogramOpts::new(
                "capacity_admission_decision_duration_seconds",
                "Admission decision latency in seconds.",
            )
            // Buckets focused on the millisecond hot path; 0.05 (p50 SLO) and
            // 0.1 (p99 SLO) boundaries are explicit so dashboards can alert.
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 1.0]),
        )
        .expect("duration histogram");
        let freshness = IntGauge::with_opts(Opts::new(
            "capacity_admission_capacity_freshness_seconds",
            "Seconds since the Allocation CRD status was last refreshed.",
        ))
        .expect("freshness gauge");
        let allocation_ratio = GaugeVec::new(
            Opts::new(
                "capacity_admission_allocation_ratio",
                "Allocated / ceiling ratio per resource (0.0–1.0+).",
            ),
            &["resource"],
        )
        .expect("allocation_ratio gauge vec");
        let total_allocatable = IntGaugeVec::new(
            Opts::new(
                "capacity_admission_total_allocatable",
                "Total allocatable capacity per resource.",
            ),
            &["resource"],
        )
        .expect("total_allocatable gauge vec");
        let current_allocation = IntGaugeVec::new(
            Opts::new(
                "capacity_admission_current_allocation",
                "Currently allocated capacity per resource.",
            ),
            &["resource"],
        )
        .expect("current_allocation gauge vec");
        let ceiling = IntGaugeVec::new(
            Opts::new("capacity_admission_ceiling", "Budget ceiling per resource."),
            &["resource"],
        )
        .expect("ceiling gauge vec");
        let exemptions = IntCounterVec::new(
            Opts::new(
                "capacity_admission_exemptions_total",
                "Pods admitted by exclusion policy, by reason.",
            ),
            &["reason"],
        )
        .expect("exemptions counter");

        registry
            .register(Box::new(verdicts.clone()))
            .expect("register verdicts");
        registry
            .register(Box::new(duration.clone()))
            .expect("register duration");
        registry
            .register(Box::new(freshness.clone()))
            .expect("register freshness");
        registry
            .register(Box::new(allocation_ratio.clone()))
            .expect("register allocation_ratio");
        registry
            .register(Box::new(total_allocatable.clone()))
            .expect("register total_allocatable");
        registry
            .register(Box::new(current_allocation.clone()))
            .expect("register current_allocation");
        registry
            .register(Box::new(ceiling.clone()))
            .expect("register ceiling");
        registry
            .register(Box::new(exemptions.clone()))
            .expect("register exemptions");

        // CounterVec/GaugeVec emit nothing until a child label-set is created.
        // Pre-create every expected series so all seven metrics appear in
        // /metrics from startup (a scrape must see the full surface at zero,
        // not an empty response before the first decision).
        for resource in [ResourceLabel::Cpu, ResourceLabel::Memory] {
            for verdict in [
                VerdictLabel::Allow,
                VerdictLabel::Deny,
                VerdictLabel::DryRunDeny,
                VerdictLabel::Error,
            ] {
                verdicts.with_label_values(&[resource.as_str(), verdict.as_str()]);
            }
            total_allocatable
                .with_label_values(&[resource.as_str()])
                .set(0);
            current_allocation
                .with_label_values(&[resource.as_str()])
                .set(0);
            ceiling.with_label_values(&[resource.as_str()]).set(0);
            allocation_ratio
                .with_label_values(&[resource.as_str()])
                .set(0.0);
        }

        // spec-008: pre-create every exemption `reason` series at zero so a
        // scrape sees the full exemption surface before the first exempt decision.
        for reason in ["namespace", "priority_class", "webhook_namespace"] {
            exemptions.with_label_values(&[reason]);
        }

        Self {
            registry,
            verdicts,
            duration,
            freshness,
            allocation_ratio,
            total_allocatable,
            current_allocation,
            ceiling,
            exemptions,
        }
    }

    /// The underlying registry (used to render `/metrics`).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Render the registry in Prometheus text exposition format.
    pub fn render(&self) -> String {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder
            .encode(&self.registry.gather(), &mut buffer)
            .expect("metrics are always encodable");
        String::from_utf8(buffer).expect("metrics exposition is UTF-8")
    }

    /// Increment the verdict counter for a resource.
    pub fn record_verdict(&self, resource: ResourceLabel, verdict: VerdictLabel) {
        self.verdicts
            .with_label_values(&[resource.as_str(), verdict.as_str()])
            .inc();
    }

    /// Increment the exemption counter for a reason (spec-008). Takes the label
    /// value as `&str` (from [`ExemptionReason::as_str`](crate::crd::ExemptionReason::as_str))
    /// so this module stays decoupled from `crate::crd`.
    pub fn record_exemption(&self, reason: &str) {
        self.exemptions.with_label_values(&[reason]).inc();
    }

    /// Record an admission decision latency observation (seconds).
    pub fn observe_duration(&self, seconds: f64) {
        self.duration.observe(seconds);
    }

    /// Set the capacity-freshness gauge (seconds since last CRD refresh).
    pub fn set_freshness(&self, seconds: i64) {
        self.freshness.set(seconds);
    }

    /// Refresh the four capacity gauges from the latest Allocation +
    /// ClusterCapacity status figures, per resource (T029 / SC-003).
    pub fn refresh_capacity(&self, cpu: CapacityFigures, memory: CapacityFigures) {
        for (resource, figures) in [(ResourceLabel::Cpu, cpu), (ResourceLabel::Memory, memory)] {
            self.total_allocatable
                .with_label_values(&[resource.as_str()])
                .set(figures.total_allocatable);
            self.current_allocation
                .with_label_values(&[resource.as_str()])
                .set(figures.allocated);
            self.ceiling
                .with_label_values(&[resource.as_str()])
                .set(figures.ceiling);
            self.allocation_ratio
                .with_label_values(&[resource.as_str()])
                .set(figures.ratio);
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_exposes_all_metrics() {
        let metrics = Metrics::new();
        let text = metrics.render();
        for name in [
            "capacity_admission_verdicts_total",
            "capacity_admission_decision_duration_seconds",
            "capacity_admission_capacity_freshness_seconds",
            "capacity_admission_allocation_ratio",
            "capacity_admission_total_allocatable",
            "capacity_admission_current_allocation",
            "capacity_admission_ceiling",
            "capacity_admission_exemptions_total",
        ] {
            assert!(
                text.contains(&format!("# HELP {name}"))
                    && text.contains(&format!("# TYPE {name} ")),
                "metric {name} must be declared in /metrics output"
            );
        }
    }

    #[test]
    fn record_verdict_increments_the_right_labelled_series() {
        let metrics = Metrics::new();
        metrics.record_verdict(ResourceLabel::Cpu, VerdictLabel::Allow);
        metrics.record_verdict(ResourceLabel::Cpu, VerdictLabel::Allow);
        metrics.record_verdict(ResourceLabel::Memory, VerdictLabel::Deny);
        let text = metrics.render();
        assert!(
            text.contains(r#"capacity_admission_verdicts_total{resource="cpu",verdict="allow"} 2"#),
            "{text}"
        );
        assert!(
            text.contains(
                r#"capacity_admission_verdicts_total{resource="memory",verdict="deny"} 1"#
            ),
            "{text}"
        );
    }

    // ---- spec-004: DryRunDeny verdict label ----

    #[test]
    fn dry_run_deny_verdict_serialises_as_dry_run_deny() {
        // T022: VerdictLabel::DryRunDeny serialises as "dry_run_deny".
        assert_eq!(VerdictLabel::DryRunDeny.as_str(), "dry_run_deny");
    }

    #[test]
    fn new_pre_creates_dry_run_deny_series_at_zero() {
        // T023: Metrics::new() pre-creates the dry_run_deny series at zero for
        // both resources (matching the existing pre-creation pattern).
        let metrics = Metrics::new();
        let text = metrics.render();
        assert!(
            text.contains(
                r#"capacity_admission_verdicts_total{resource="cpu",verdict="dry_run_deny"} 0"#
            ),
            "cpu dry_run_deny series must be pre-created at zero: {text}"
        );
        assert!(
            text.contains(
                r#"capacity_admission_verdicts_total{resource="memory",verdict="dry_run_deny"} 0"#
            ),
            "memory dry_run_deny series must be pre-created at zero: {text}"
        );
    }

    // ---- spec-008: capacity_admission_exemptions_total{reason} ----

    #[test]
    fn render_exposes_exemptions_counter() {
        let metrics = Metrics::new();
        let text = metrics.render();
        for needle in [
            "# HELP capacity_admission_exemptions_total",
            "# TYPE capacity_admission_exemptions_total counter",
        ] {
            assert!(text.contains(needle), "missing {needle}: {text}");
        }
    }

    #[test]
    fn new_pre_creates_exemption_series_at_zero() {
        // T004: all three reason labels are pre-created at zero at startup.
        let metrics = Metrics::new();
        let text = metrics.render();
        for reason in ["namespace", "priority_class", "webhook_namespace"] {
            assert!(
                text.contains(&format!(
                    r#"capacity_admission_exemptions_total{{reason="{reason}"}} 0"#
                )),
                "{reason} exemption series must be pre-created at zero: {text}"
            );
        }
    }

    #[test]
    fn record_exemption_increments_only_that_reason() {
        let metrics = Metrics::new();
        metrics.record_exemption("namespace");
        metrics.record_exemption("namespace");
        metrics.record_exemption("priority_class");
        let text = metrics.render();
        assert!(
            text.contains(r#"capacity_admission_exemptions_total{reason="namespace"} 2"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_exemptions_total{reason="priority_class"} 1"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_exemptions_total{reason="webhook_namespace"} 0"#),
            "untouched reason stays at zero: {text}"
        );
    }

    #[test]
    fn observe_duration_populates_the_histogram() {
        let metrics = Metrics::new();
        metrics.observe_duration(0.003);
        metrics.observe_duration(0.020);
        let text = metrics.render();
        assert!(
            text.contains("capacity_admission_decision_duration_seconds_count 2"),
            "{text}"
        );
        // A bucket at the p50 SLO boundary must exist for alerting.
        assert!(
            text.contains(r#"capacity_admission_decision_duration_seconds_bucket{le="0.05"}"#),
            "{text}"
        );
    }

    #[test]
    fn set_freshness_sets_the_gauge() {
        let metrics = Metrics::new();
        metrics.set_freshness(12);
        let text = metrics.render();
        assert!(
            text.contains("capacity_admission_capacity_freshness_seconds 12"),
            "{text}"
        );
    }

    #[test]
    fn refresh_capacity_sets_per_resource_gauges() {
        let metrics = Metrics::new();
        let cpu = CapacityFigures {
            allocated: 240_000,
            ceiling: 256_000,
            total_allocatable: 320_000,
            ratio: 0.9375,
        };
        let memory = CapacityFigures {
            allocated: 206_158_430_208,
            ceiling: 412_316_860_416,
            total_allocatable: 515_396_075_520,
            ratio: 0.5,
        };
        metrics.refresh_capacity(cpu, memory);
        let text = metrics.render();
        assert!(
            text.contains(r#"capacity_admission_total_allocatable{resource="cpu"} 320000"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_current_allocation{resource="cpu"} 240000"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_ceiling{resource="cpu"} 256000"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_allocation_ratio{resource="cpu"} 0.9375"#),
            "{text}"
        );
        assert!(
            text.contains(r#"capacity_admission_allocation_ratio{resource="memory"} 0.5"#),
            "{text}"
        );
    }
}
