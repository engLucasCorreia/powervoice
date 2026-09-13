//! Built-in rack modules: gain (M1); dynamics (compressor + limiter) and noise gate (S3-02);
//! noise reduction, EQ, true-peak limiter (later slices).
//!
//! This crate knows nothing about the rack: the composition roots (`cli`, `src-tauri`) register
//! [`builtin_factories`] into the rack's module registry (SPEC-012 §2.10).

use std::sync::Arc;

use vox_module_api::ModuleFactory;

mod dynamics;
mod gain;
mod noise_gate;
mod noise_reduction;
mod parametric_eq;
mod schema;
mod true_peak_limiter;

pub use dynamics::{Dynamics, DynamicsFactory};
pub use gain::{Gain, GainFactory};
pub use noise_gate::{NoiseGate, NoiseGateFactory};
pub use noise_reduction::{NoiseReduction, NoiseReductionFactory};
pub use parametric_eq::{ParametricEq, ParametricEqFactory};
pub use true_peak_limiter::{TruePeakLimiter, TruePeakLimiterFactory};

/// Factories of every built-in module, for the composition roots to register.
pub fn builtin_factories() -> Vec<Arc<dyn ModuleFactory>> {
    vec![
        Arc::new(GainFactory::new()),
        Arc::new(DynamicsFactory::new()),
        Arc::new(NoiseGateFactory::new()),
        Arc::new(TruePeakLimiterFactory::new()),
        Arc::new(NoiseReductionFactory::new()),
        Arc::new(ParametricEqFactory::new()),
    ]
}
