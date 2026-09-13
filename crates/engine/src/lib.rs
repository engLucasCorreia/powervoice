//! Audio engine: backends, audio threads, transport, recording, monitoring.
//!
//! Device layer (T-102, SPEC-001):
//! - [`backend`]: the [`Backend`] trait, the cpal backend (feature `backend-cpal`) and the
//!   deterministic [`backend::fake::FakeBackend`].
//! - [`devices`]: selection and fallback rules, input-channel deinterleaving, hot-plug polling.
//! - [`device_state`]: the device-lost / recovery state machine.
//! - [`prefs`]: persisted device preferences (saved intent vs applied value).
//!
//! Playback (S1-01, SPEC-003, ADR-002):
//! - [`Engine`] / [`EngineHandle`] / [`ManualEngine`]: the facade over the control thread.
//! - [`transport`]: the transport state machine and its commands.
//! - [`telemetry`]: `VXTM` frames (playhead anchor + output meter) at 60 Hz, and `VXMT` module
//!   telemetry (the rack slots' meters, e.g. gain reduction; H-03).
//! - Internals: `control` (16.7 ms tick, device handling), `output` (the RT output callback),
//!   `reader` (prefetch + resampling), `rt` (ring element types).
//!
//! Recording (S1-04, SPEC-002 §2.1–§2.3, §2.7 Off/Dry):
//! - [`record`]: arm, record start/stop, the take hand-off ([`record::RecordDone`]), monitoring.
//! - Internals: `input` (the RT input callback: meter, clip, capture + monitor rings),
//!   `capture` (the capture-writer and take-sync threads).
//!
//! Spectral display (T-204, SPEC-007):
//! - [`spectro`]: the spectrogram tile service (worker threads, content-keyed tile cache,
//!   `VXST` frames).

pub mod backend;
mod capture;
mod control;
pub mod device_state;
pub mod devices;
mod engine;
mod input;
mod output;
pub mod prefs;
mod rack_api;
mod reader;
pub mod record;
mod rt;
pub mod spectro;
pub mod telemetry;
pub mod transport;

pub use backend::{
    Backend, BackendError, BufferRequest, DeviceInfo, DeviceKey, DeviceSnapshot, Direction,
    DirectionCaps, Enumerate, HostId, InputCallback, InputTimestamp, OutputCallback,
    OutputTimestamp, StreamHandle, StreamInfo, StreamRequest, StreamStatus,
};
pub use engine::{
    Clock, DevicesView, Engine, EngineConfig, EngineEvent, EngineHandle, EventSink, ManualEngine,
    PlaybackDoc,
};
pub use prefs::DevicePrefs;
pub use rack_api::{
    MAX_RESPONSE_CURVE_POINTS, NOISE_REDUCTION_MODULE_ID, NrCapturePrep, RackApiError, RackCommand,
    RackSlot, RackSnapshot, ResponseCurvePoints,
};
pub use record::{
    DropoutMark, LIVE_PEAKS_SPB, LiveTakePeaks, MonitorMode, RecordDone, RecordError, RecordState,
    RecordingResult, StopReason,
};
pub use telemetry::{
    ModuleTelemetryFrame, ModuleTelemetryRecord, ModuleTelemetrySink, TelemetryFrame, TelemetrySink,
};
pub use transport::{TransportCommand, TransportState};

// Unit tests of RT wrappers run under the allocation checker.
#[cfg(test)]
vox_module_api::install_test_allocator!();
