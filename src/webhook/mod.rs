//! Admission webhook: HTTP handler, decision logic, and error mapping.

pub mod admission;
pub mod error;
pub mod handler;

pub use admission::{AdmissionVerdict, Figures, ceiling, check_budget};
pub use error::{AdmissionError, BudgetViolation, MissingCapacityData, ResourceType};
pub use handler::{
    AppState, Clock, DecisionOutcome, DecisionSummary, DecisionVerdict, Freshness, ResourceFigures,
    assess_freshness, classify_error, handle, healthz, metrics_router, router, with_catch_unwind,
    with_timeout,
};
