//! Built-in rack modules: gain (M1); gate, noise reduction, EQ, dynamics, true-peak limiter
//! (M4/M5).
//!
//! This crate knows nothing about the rack: the composition roots (`cli`, `src-tauri`) register
//! [`builtin_factories`] into the rack's module registry (SPEC-012 §2.10).

use std::sync::Arc;

use vox_module_api::ModuleFactory;

mod gain;

pub use gain::{Gain, GainFactory};

/// Factories of every built-in module, for the composition roots to register.
pub fn builtin_factories() -> Vec<Arc<dyn ModuleFactory>> {
    vec![Arc::new(GainFactory::new())]
}
