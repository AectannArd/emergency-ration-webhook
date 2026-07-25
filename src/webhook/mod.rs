//! Admission webhook: HTTP handler, decision logic, and error mapping.

pub mod admission;
pub mod error;
pub mod handler;

pub use admission::{AdmissionVerdict, Figures, ceiling, check_budget};
pub use error::{AdmissionError, BudgetViolation, MissingCapacityData, ResourceType};
