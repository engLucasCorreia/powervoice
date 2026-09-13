//! The engine facade (ADR-002 §1).
//!
//! - [`Engine`] owns the control thread (which owns the reader and device-poll threads and the
//!   output stream). Dropping it shuts everything down.
//! - [`EngineHandle`] is the cheap, cloneable, synchronous API the app calls from any thread.
//!   Never call it from inside an [`EventSink`] or a telemetry sink: those run on the control
//!   thread and the call would wait for itself.
//! - [`ManualEngine`] runs the same control logic inline, without threads, for deterministic
//!   tests with the fake backend (the caller advances fake time and calls
//!   [`ManualEngine::tick`]).

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use vox_project::{ChunkStore, DocSnapshot};
use vox_rack::{RackModel, Registry};

use crate::backend::{Backend, BufferRequest, DeviceSnapshot, HostId, app_now_ns};
use crate::control::{self, Control, ControlMsg};
use crate::device_state::DeviceStatus;
use crate::devices::DeviceNotice;
use crate::prefs::DevicePrefs;
use crate::telemetry::TelemetrySink;
use crate::transport::{TransportCommand, TransportState};

/// The app clock (ns since the process epoch); tests pass the fake backend's simulated time.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Receives engine events on the control thread (must not block, must not call the engine).
pub type EventSink = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// The document to play: the session's chunk store and an immutable snapshot of it. The engine
/// keeps its own references (the reader thread holds the snapshot, never the audio thread).
#[derive(Clone)]
pub struct PlaybackDoc {
    /// The session's chunk store.
    pub store: Arc<ChunkStore>,
    /// The revision to play.
    pub snapshot: Arc<DocSnapshot>,
}

impl fmt::Debug for PlaybackDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaybackDoc")
            .field("rate", &self.snapshot.sample_rate_hz)
            .field("len", &self.snapshot.len_samples)
            .field("audio_rev", &self.snapshot.audio_rev)
            .finish()
    }
}

/// Engine → app notifications (the app maps them to `transport_state`, `devices_changed` and
/// `notice` events).
#[derive(Clone, Debug)]
pub enum EngineEvent {
    /// The transport state changed.
    Transport(TransportState),
    /// The device list or the output device changed.
    Devices(DevicesView),
    /// A device notice (fallback, not found, lost, reconnected, …).
    Notice(DeviceNotice),
}

/// Devices as the Settings dialog shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicesView {
    /// Available hosts.
    pub hosts: Vec<HostId>,
    /// The host in use.
    pub host: Option<HostId>,
    /// Its devices.
    pub snapshot: DeviceSnapshot,
    /// The saved intent.
    pub prefs: DevicePrefs,
    /// The configured output device.
    pub output_device: Option<String>,
    /// The open output stream's rate.
    pub output_rate_hz: Option<u32>,
    /// The open output stream's buffer request.
    pub output_buffer: Option<BufferRequest>,
    /// Output status dot.
    pub output_status: DeviceStatus,
}

/// Everything the engine needs at start.
pub struct EngineConfig {
    /// Audio backend (cpal in the app, fake in tests).
    pub backend: Arc<dyn Backend>,
    /// Module registry (the composition root registers the built-ins).
    pub registry: Arc<Registry>,
    /// The rack to play through.
    pub rack: RackModel,
    /// Saved device preferences.
    pub prefs: DevicePrefs,
    /// Event sink.
    pub events: EventSink,
    /// App clock.
    pub clock: Clock,
}

impl EngineConfig {
    /// Defaults: empty rack, default prefs, no events, the process app clock.
    pub fn new(backend: Arc<dyn Backend>, registry: Arc<Registry>) -> Self {
        Self {
            backend,
            registry,
            rack: RackModel { slots: Vec::new() },
            prefs: DevicePrefs::default(),
            events: Arc::new(|_| {}),
            clock: Arc::new(app_now_ns),
        }
    }
}

/// The running engine. Dropping it stops playback, closes the stream and joins its threads.
pub struct Engine {
    handle: EngineHandle,
    join: Option<JoinHandle<()>>,
}

impl Engine {
    /// Starts the control thread (devices are enumerated and the output opened asynchronously;
    /// watch [`EngineEvent::Devices`]).
    pub fn start(config: EngineConfig) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let inbox = tx.clone();
        let join = std::thread::Builder::new()
            .name("vox-control".into())
            .spawn(move || {
                let mut control = Control::new(config, Some(inbox));
                control::run(&mut control, &rx);
                control.shutdown();
            })?;
        Ok(Self {
            handle: EngineHandle { tx },
            join: Some(join),
        })
    }

    /// A handle for the app's command layer.
    pub fn handle(&self) -> EngineHandle {
        self.handle.clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.handle.tx.send(ControlMsg::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Synchronous engine API (ADR-002 §1). Every call is answered by the control thread; a call on
/// a stopped engine returns the default/`None`.
#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::Sender<ControlMsg>,
}

impl fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineHandle").finish_non_exhaustive()
    }
}

impl EngineHandle {
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Control) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .send(ControlMsg::Call(Box::new(move |c| {
                let _ = tx.send(f(c));
            })))
            .ok()?;
        rx.recv().ok()
    }

    /// Applies a transport command; returns the new state.
    pub fn transport(&self, cmd: TransportCommand) -> TransportState {
        self.call(move |c| c.transport_command(cmd))
            .unwrap_or_default()
    }

    /// The current transport state.
    pub fn transport_state(&self) -> TransportState {
        self.call(|c| c.transport_state()).unwrap_or_default()
    }

    /// Sets the document to play (`None`: none). Stops playback first (Pause semantics).
    pub fn set_document(&self, doc: Option<PlaybackDoc>) {
        let _ = self.call(move |c| c.set_document(doc));
    }

    /// Sets the time selection used by Play from start (`None`: no selection).
    pub fn set_selection(&self, selection: Option<(u64, u64)>) {
        let _ = self.call(move |c| c.set_selection(selection));
    }

    /// The device view for the Settings dialog.
    pub fn devices(&self) -> Option<DevicesView> {
        self.call(|c| c.devices_view())
    }

    /// Applies new device preferences; returns the resulting view (check `output_status`
    /// before persisting the prefs, SPEC-001 §2.5).
    pub fn select_devices(&self, prefs: DevicePrefs) -> Option<DevicesView> {
        self.call(move |c| c.select_devices(prefs))
    }

    /// Installs (or removes) the 60 Hz telemetry sink.
    pub fn set_telemetry_sink(&self, sink: Option<TelemetrySink>) {
        let _ = self.call(move |c| c.set_telemetry_sink(sink));
    }
}

/// The engine without threads (deterministic tests, benches): the reader runs inline, devices are
/// polled by [`poll_devices`](Self::poll_devices), time advances only with the fake backend.
pub struct ManualEngine {
    control: Control,
}

impl ManualEngine {
    /// A manual engine; call [`poll_devices`](Self::poll_devices) to enumerate and open.
    pub fn new(config: EngineConfig) -> Self {
        Self {
            control: Control::new(config, None),
        }
    }

    /// One control tick.
    pub fn tick(&mut self) {
        self.control.tick();
    }

    /// One device-poll pass (hosts + devices).
    pub fn poll_devices(&mut self) {
        self.control.poll_devices();
    }

    /// See [`EngineHandle::transport`].
    pub fn transport(&mut self, cmd: TransportCommand) -> TransportState {
        self.control.transport_command(cmd)
    }

    /// See [`EngineHandle::transport_state`].
    pub fn transport_state(&self) -> TransportState {
        self.control.transport_state()
    }

    /// See [`EngineHandle::set_document`].
    pub fn set_document(&mut self, doc: Option<PlaybackDoc>) {
        self.control.set_document(doc);
    }

    /// See [`EngineHandle::set_selection`].
    pub fn set_selection(&mut self, selection: Option<(u64, u64)>) {
        self.control.set_selection(selection);
    }

    /// See [`EngineHandle::devices`].
    pub fn devices(&self) -> DevicesView {
        self.control.devices_view()
    }

    /// See [`EngineHandle::select_devices`].
    pub fn select_devices(&mut self, prefs: DevicePrefs) -> DevicesView {
        self.control.select_devices(prefs)
    }

    /// See [`EngineHandle::set_telemetry_sink`].
    pub fn set_telemetry_sink(&mut self, sink: Option<TelemetrySink>) {
        self.control.set_telemetry_sink(sink);
    }
}

impl Drop for ManualEngine {
    fn drop(&mut self) {
        self.control.shutdown();
    }
}
