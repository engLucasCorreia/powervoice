//! `vox-testkit`: deterministic signal generators, objective measurements
//! (peak/RMS/LUFS/true-peak/noise-floor), and golden-file helpers used by
//! DSP acceptance tests and `powervoice-cli`.

pub mod bandlimit;
pub mod error;
pub mod golden;
pub mod measure;
pub mod prng;
pub mod signal;
pub mod true_peak;
pub mod units;
pub mod wav;

pub use error::{Result, TestkitError};
