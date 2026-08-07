//! Library facade for the capacity admission webhook.
//!
//! Re-exports the public modules so integration and BDD tests can drive the
//! admission decision logic directly without depending on internal paths.

pub mod config;
pub mod controllers;
pub mod crd;
pub mod equalizer;
pub mod metrics;
pub mod resources;
pub mod time_util;
pub mod webhook;
