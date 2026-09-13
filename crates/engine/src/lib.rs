//! Audio engine: backends, audio threads, transport, recording, monitoring.
//!
//! Device layer (T-102, SPEC-001):
//! - [`backend`]: the [`Backend`] trait, the cpal backend (feature `backend-cpal`) and the
//!   deterministic [`backend::fake::FakeBackend`].
//! - [`devices`]: selection and fallback rules, input-channel deinterleaving, hot-plug polling.
//! - [`device_state`]: the device-lost / recovery state machine.
//! - [`prefs`]: persisted device preferences (saved intent vs applied value).

pub mod backend;
pub mod device_state;
pub mod devices;
pub mod prefs;

pub use backend::{
    Backend, BackendError, BufferRequest, DeviceInfo, DeviceKey, DeviceSnapshot, Direction,
    DirectionCaps, Enumerate, HostId, InputCallback, InputTimestamp, OutputCallback,
    OutputTimestamp, StreamHandle, StreamInfo, StreamRequest, StreamStatus,
};
pub use prefs::DevicePrefs;

// Unit tests of RT wrappers run under the allocation checker.
#[cfg(test)]
vox_module_api::install_test_allocator!();
