//! The engine facade (ADR-002 §1).
//!
//! - [`Engine`] owns the control thread (which owns the reader and device-poll threads and the
//!   output stream). Dropping it shuts everything down.
//! - [`EngineHandle`] is the cheap, cloneable, synchronous API the app calls from any thread.
//!   Never call it from inside an [`EventSink`] or a telemetry sink: those run on the control
//!   thread and the call would wait for itself.
//! - [`ManualEngine`] runs the same control logic inline, without threads, for deterministic
//!   tests with the fake backend (the caller advances fake time and calls
//!   [`ManualEngine::tick`]); its capture-writer also runs inline.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use vox_project::{ChunkStore, DocSnapshot, FreeSpaceProvider, SystemFreeSpace, TakeCapture};
use vox_rack::{ModuleDescriptor, ModulePreset, ModuleState, RackModel, RackNotice, Registry};

use crate::analyzer::{AnalyzerResponse, AnalyzerSink, InspectorConfig, InspectorSink, VoiceSink};
use crate::backend::{Backend, BufferRequest, DeviceSnapshot, HostId, app_now_ns};
use crate::control::{self, Control, ControlMsg};
use crate::device_state::DeviceStatus;
use crate::devices::DeviceNotice;
use crate::prefs::DevicePrefs;
use crate::rack_api::{
    NrCapturePrep, RackApiError, RackCommand, RackSnapshot, ResponseCurvePoints,
};
use crate::record::{
    CalibrationError, CalibrationStatus, LiveTakePeaks, MonitorMode, RecordDone, RecordError,
    RecordPhaseInfo, RecordState,
};
use crate::record_op::{RecordPlan, RecordPrefs};
use crate::telemetry::{ModuleTelemetrySink, TelemetrySink};
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
    /// A rack notice (S3-01, SPEC-012 §2.1–§2.6): a parameter mirror echo, a latency change, a
    /// slot failure or a restart. The app maps `ParamChanged` to `param_changed` and
    /// `LatencyChanged` to `rack_latency`; `SlotFailed`/`SlotRestarted` also arrive as a
    /// [`EngineEvent::RackChanged`] snapshot (built on the control thread, H-01 handoff: re-read
    /// `slot_info` on `SlotRestarted`).
    Rack(RackNotice),
    /// The rack's slot list, status or latency changed: every mutating [`RackCommand`]'s result,
    /// and any later async change (a failure, a restart). Maps to the `rack_changed` event.
    RackChanged(RackSnapshot),
    /// The record panel state changed (S1-04).
    Record(RecordState),
    /// T-304 (SPEC-022 §4.9): a record operation changed phase (`record_phase`).
    RecordPhase(RecordPhaseInfo),
    /// T-304 (SPEC-022 §4.6): the aligned record window of `take` opened at take sample
    /// `k_start` — the app journals it (`Session::note_take_window`) so recovery reproduces the
    /// live alignment.
    RecordWindow { take: u32, k_start: u64 },
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
    /// The configured input device (S1-04).
    pub input_device: Option<String>,
    /// Input status dot.
    pub input_status: DeviceStatus,
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
    /// Preferred rate of new recordings (Settings → Default format, SPEC-002 §2.2): the input
    /// opens at it when the device supports it.
    pub record_rate_hz: u32,
    /// Free-space query for the recording disk floor and the record panel's remaining-time
    /// display (SPEC-002 §2.5, A-010): the real provider, or a fake one in tests.
    pub disk_space: Arc<dyn FreeSpaceProvider>,
    /// The volume recordings are written to (the sessions directory) — queried about once a
    /// second while the record panel is relevant (armed or recording), never on the RT input
    /// thread.
    pub record_volume: PathBuf,
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
            record_rate_hz: 48_000,
            disk_space: Arc::new(SystemFreeSpace),
            record_volume: PathBuf::new(),
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
                // Handle calls made while shutting down (e.g. from a take's done callback on the
                // writer thread) now fail fast instead of waiting for this thread.
                drop(rx);
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

    /// Sets the time selection used by Play from start and (H-37) as the loop region
    /// (`None`: no selection); returns the new state.
    pub fn set_selection(&self, selection: Option<(u64, u64)>) -> TransportState {
        self.call(move |c| c.set_selection(selection))
            .unwrap_or_default()
    }

    /// H-37 (SPEC-003 §2.1): the Loop toggle; returns the new state.
    pub fn set_loop(&self, enabled: bool) -> TransportState {
        self.call(move |c| c.set_loop(enabled)).unwrap_or_default()
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

    /// Installs (or removes) the module-telemetry sink (H-03, `VXMT`): the rack slots' meter
    /// values (e.g. the true-peak limiter's gain reduction), one frame per telemetry tick while
    /// at least one slot has a `Telemetry` extension.
    pub fn set_module_telemetry_sink(&self, sink: Option<ModuleTelemetrySink>) {
        let _ = self.call(move |c| c.set_module_telemetry_sink(sink));
    }

    /// H-16 (`Settings.telemetry_rate_hz`, SPEC-003 §3): applies a new telemetry rate (60 or
    /// 30 Hz) to the `VXTM`/`VXMT`/`VXSA` publish cadence, changeable at runtime.
    pub fn set_telemetry_rate_hz(&self, rate_hz: u32) {
        let _ = self.call(move |c| c.set_telemetry_rate_hz(rate_hz));
    }

    /// Subscribes to the live output analyzer (`VXSA`, T-208): a BH4 FFT + 1/24-octave band +
    /// `response`-averaged spectrum of the post-rack (+ dry monitor) signal, at the telemetry
    /// rate, only while at least one subscriber exists. Returns the subscriber id.
    pub fn analyzer_subscribe(&self, sink: AnalyzerSink, response: AnalyzerResponse) -> u32 {
        self.call(move |c| c.analyzer_subscribe(sink, response))
            .unwrap_or(0)
    }

    /// Changes a subscriber's averaging response (Fast/Medium/Slow).
    pub fn analyzer_set_response(&self, id: u32, response: AnalyzerResponse) {
        let _ = self.call(move |c| c.analyzer_set_response(id, response));
    }

    /// Removes a subscriber of any kind (`VXSA`, voice, Inspector); the tap turns off once none
    /// remain.
    pub fn analyzer_unsubscribe(&self, id: u32) {
        let _ = self.call(move |c| c.analyzer_unsubscribe(id));
    }

    /// H-42 (SPEC-007 §8.8): subscribes to live voice diagnostics — a
    /// [`vox_dsp::diagnostics::VoiceReport`] about 10 times a second, only when it changed.
    /// Unsubscribe with [`Self::analyzer_unsubscribe`].
    pub fn analyzer_voice_subscribe(&self, sink: VoiceSink) -> u32 {
        self.call(move |c| c.analyzer_voice_subscribe(sink))
            .unwrap_or(0)
    }

    /// H-42 (SPEC-007 §8.3): subscribes a Spectrum Inspector stream (`VXIS`) at `config`.
    /// Unsubscribe with [`Self::analyzer_unsubscribe`].
    pub fn analyzer_inspector_subscribe(
        &self,
        sink: InspectorSink,
        config: InspectorConfig,
    ) -> u32 {
        self.call(move |c| c.analyzer_inspector_subscribe(sink, config))
            .unwrap_or(0)
    }

    /// H-42: changes an Inspector stream's FFT size / window / response.
    pub fn analyzer_inspector_configure(&self, id: u32, config: InspectorConfig) {
        let _ = self.call(move |c| c.analyzer_inspector_configure(id, config));
    }

    /// The registered modules (the Add-module menu; S3-01).
    pub fn rack_registry(&self) -> Vec<ModuleDescriptor> {
        self.call(|c| c.rack_registry()).unwrap_or_default()
    }

    /// The current rack state (S3-01).
    pub fn rack_snapshot(&self) -> RackSnapshot {
        self.call(|c| c.rack_snapshot()).unwrap_or_default()
    }

    /// Applies a rack edit or parameter change (S3-01); returns the resulting snapshot.
    pub fn rack_command(&self, cmd: RackCommand) -> Result<RackSnapshot, RackApiError> {
        self.call(move |c| c.rack_apply(cmd))
            .unwrap_or(Err(RackApiError::Unavailable))
    }

    /// The current rack as plain data (H-08 handoff: lets export build its offline chain from the
    /// live rack instead of `RackModel::default()`). `None` only if the engine is stopped.
    pub fn rack_model(&self) -> Option<RackModel> {
        self.call(|c| c.rack_model())
    }

    /// T-306: replaces the whole rack (a sidecar's `rack` section on document open, or a
    /// recovered `state` record). `None` only if the engine is stopped.
    pub fn rack_load_model(&self, model: RackModel) -> Option<Result<RackSnapshot, RackApiError>> {
        self.call(move |c| c.rack_load_model(model))
    }

    /// Arms (opens the input stream, starts the input meter) or disarms. Locked on while
    /// recording (S1-04, SPEC-002 §2.1).
    pub fn set_armed(&self, armed: bool) -> Option<RecordState> {
        self.call(move |c| c.set_armed(armed))
    }

    /// Sets the monitoring mode (Off / Dry).
    pub fn set_monitor_mode(&self, mode: MonitorMode) -> Option<RecordState> {
        self.call(move |c| c.set_monitor_mode(mode))
    }

    /// Preferred rate of new recordings; applies the next time the input opens.
    pub fn set_record_rate(&self, rate_hz: u32) {
        let _ = self.call(move |c| c.set_record_rate(rate_hz));
    }

    /// Starts recording into `capture` (from `Session::begin_take` at `doc_rate_hz`, which must
    /// equal the input rate); `done` runs on the capture-writer thread when the take is finished
    /// (see [`crate::record`]). On `Err` the take was not used: discard it.
    pub fn record_start(
        &self,
        doc_rate_hz: u32,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        self.call(move |c| c.record_start(doc_rate_hz, capture, done))
            .unwrap_or(Err(RecordError::EngineStopped))
    }

    /// Stops the recording (the take is finished and handed to its done callback).
    pub fn record_stop(&self) -> Option<RecordState> {
        self.call(|c| c.record_stop())
    }

    /// The record panel state.
    pub fn record_state(&self) -> Option<RecordState> {
        self.call(|c| c.record_state())
    }

    /// H-07: a snapshot of the take being captured's running peaks (`None`: the engine is
    /// stopped, or no take is being captured). `start_bucket`/`max` page through
    /// [`crate::record::LIVE_PEAKS_SPB`]-sample buckets.
    pub fn live_take_peaks(&self, start_bucket: u32, max: u32) -> Option<LiveTakePeaks> {
        self.call(move |c| c.live_take_peaks(start_bucket, max))
            .flatten()
    }

    /// T-304 (SPEC-022 §2.2): resolves a Record press on a document that has audio — stops a
    /// playing transport first (Pause semantics: the cursor becomes the heard position), then
    /// applies the resolution table to `selection` and `prefs`. The app begins the matching take
    /// (`Session::begin_take_with`) and hands it to [`Self::record_start_op`].
    pub fn record_prepare(
        &self,
        selection: Option<(u64, u64)>,
        prefs: RecordPrefs,
    ) -> Result<RecordPlan, RecordError> {
        self.call(move |c| c.record_prepare(selection, prefs))
            .unwrap_or(Err(RecordError::EngineStopped))
    }

    /// T-304: runs a resolved record operation (Insert / Overwrite / Punch with pre-roll,
    /// post-roll and latency-compensated alignment) into `capture`; `done` receives the result
    /// with its record window (`RecordingResult::op`). A `New` plan is [`Self::record_start`].
    pub fn record_start_op(
        &self,
        plan: RecordPlan,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        self.call(move |c| c.record_start_op(plan, capture, done))
            .unwrap_or(Err(RecordError::EngineStopped))
    }

    /// T-304 (SPEC-022 §2.14): starts a loopback calibration run (monitoring forced off, the
    /// input armed, 5 sweeps after the rack). `offset_ns`: δ applied for a Verify pass (0 for a
    /// measurement). Poll with [`Self::calibration_poll`].
    pub fn calibration_start(&self, offset_ns: i64) -> Result<(), CalibrationError> {
        self.call(move |c| c.calibration_start(offset_ns))
            .unwrap_or(Err(CalibrationError::Interrupted))
    }

    /// T-304: the calibration run's progress, or its capture once (then [`CalibrationStatus::Idle`]).
    pub fn calibration_poll(&self) -> CalibrationStatus {
        self.call(|c| c.calibration_poll())
            .unwrap_or(CalibrationStatus::Idle)
    }

    /// T-304: aborts a calibration run (monitoring and arming are restored).
    pub fn calibration_cancel(&self) {
        let _ = self.call(|c| c.calibration_cancel());
    }

    /// Capture Noise Print, step 1 (S3-06, SPEC-014 §2.3): resolves the target slot (inserting a
    /// default one if none exists) and reads out what the capture job needs before it starts
    /// reading audio. Call from any thread (the job's own worker), never from an [`EventSink`].
    pub fn nr_capture_prepare(&self, hint: Option<usize>) -> Result<NrCapturePrep, RackApiError> {
        self.call(move |c| c.nr_capture_prepare(hint))
            .unwrap_or(Err(RackApiError::Unavailable))
    }

    /// Capture Noise Print, step 2: installs `blob` as slot `index`'s committed noise print, live.
    pub fn nr_capture_apply(
        &self,
        index: usize,
        blob: Vec<u8>,
    ) -> Result<RackSnapshot, RackApiError> {
        self.call(move |c| c.nr_capture_apply(index, blob))
            .unwrap_or(Err(RackApiError::Unavailable))
    }

    /// The EQ graph's response curve (S3-07, SPEC-015 §2.6.6): evaluates slot `index`'s
    /// `ResponseCurve` extension at `freqs_hz` from the parameter mirror's current values, at the
    /// rack's rate. `freqs_hz` is truncated to `MAX_RESPONSE_CURVE_POINTS` if longer. Read-only;
    /// call from any thread.
    pub fn response_curve(
        &self,
        index: usize,
        freqs_hz: Vec<f64>,
    ) -> Result<ResponseCurvePoints, RackApiError> {
        self.call(move |c| c.response_curve(index, freqs_hz))
            .unwrap_or(Err(RackApiError::Unavailable))
    }

    /// The committed state of slot `index` (T-406: what "Save as preset" reads). Read-only.
    pub fn rack_slot_state(&self, index: usize) -> Result<ModuleState, RackApiError> {
        self.call(move |c| c.rack_slot_state(index))
            .unwrap_or(Err(RackApiError::Unavailable))
    }

    /// The module id of slot `index` (registry key, no `@version`); `None` for a placeholder, an
    /// out-of-range index, or when there is no live rack (T-406: which preset menu to show).
    pub fn rack_slot_module_id(&self, index: usize) -> Option<String> {
        self.call(move |c| c.rack_slot_module_id(index)).flatten()
    }

    /// `module_id`'s factory presets (T-406, ADR-005 §2 `ModuleFactory::presets`); empty for an
    /// unregistered id. Always available, even with no live rack.
    pub fn rack_module_presets(&self, module_id: &str) -> Vec<ModulePreset> {
        let module_id = module_id.to_owned();
        self.call(move |c| c.rack_module_presets(&module_id))
            .unwrap_or_default()
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
    pub fn set_selection(&mut self, selection: Option<(u64, u64)>) -> TransportState {
        self.control.set_selection(selection)
    }

    /// See [`EngineHandle::set_loop`].
    pub fn set_loop(&mut self, enabled: bool) -> TransportState {
        self.control.set_loop(enabled)
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

    /// See [`EngineHandle::set_module_telemetry_sink`].
    pub fn set_module_telemetry_sink(&mut self, sink: Option<ModuleTelemetrySink>) {
        self.control.set_module_telemetry_sink(sink);
    }

    /// See [`EngineHandle::set_telemetry_rate_hz`].
    pub fn set_telemetry_rate_hz(&mut self, rate_hz: u32) {
        self.control.set_telemetry_rate_hz(rate_hz);
    }

    /// See [`EngineHandle::analyzer_subscribe`].
    pub fn analyzer_subscribe(&mut self, sink: AnalyzerSink, response: AnalyzerResponse) -> u32 {
        self.control.analyzer_subscribe(sink, response)
    }

    /// See [`EngineHandle::analyzer_set_response`].
    pub fn analyzer_set_response(&mut self, id: u32, response: AnalyzerResponse) {
        self.control.analyzer_set_response(id, response);
    }

    /// See [`EngineHandle::analyzer_unsubscribe`].
    pub fn analyzer_unsubscribe(&mut self, id: u32) {
        self.control.analyzer_unsubscribe(id);
    }

    /// See [`EngineHandle::analyzer_voice_subscribe`].
    pub fn analyzer_voice_subscribe(&mut self, sink: VoiceSink) -> u32 {
        self.control.analyzer_voice_subscribe(sink)
    }

    /// See [`EngineHandle::analyzer_inspector_subscribe`].
    pub fn analyzer_inspector_subscribe(
        &mut self,
        sink: InspectorSink,
        config: InspectorConfig,
    ) -> u32 {
        self.control.analyzer_inspector_subscribe(sink, config)
    }

    /// See [`EngineHandle::analyzer_inspector_configure`].
    pub fn analyzer_inspector_configure(&mut self, id: u32, config: InspectorConfig) {
        self.control.analyzer_inspector_configure(id, config);
    }

    /// See [`EngineHandle::rack_registry`].
    pub fn rack_registry(&self) -> Vec<ModuleDescriptor> {
        self.control.rack_registry()
    }

    /// See [`EngineHandle::rack_snapshot`].
    pub fn rack_snapshot(&self) -> RackSnapshot {
        self.control.rack_snapshot()
    }

    /// See [`EngineHandle::rack_command`].
    pub fn rack_command(&mut self, cmd: RackCommand) -> Result<RackSnapshot, RackApiError> {
        self.control.rack_apply(cmd)
    }

    /// See [`EngineHandle::rack_model`].
    pub fn rack_model(&self) -> RackModel {
        self.control.rack_model()
    }

    /// See [`EngineHandle::set_armed`].
    pub fn set_armed(&mut self, armed: bool) -> RecordState {
        self.control.set_armed(armed)
    }

    /// See [`EngineHandle::set_monitor_mode`].
    pub fn set_monitor_mode(&mut self, mode: MonitorMode) -> RecordState {
        self.control.set_monitor_mode(mode)
    }

    /// See [`EngineHandle::set_record_rate`].
    pub fn set_record_rate(&mut self, rate_hz: u32) {
        self.control.set_record_rate(rate_hz);
    }

    /// See [`EngineHandle::record_start`]. The capture-writer runs inline in [`Self::tick`]
    /// and calls `done` from there.
    pub fn record_start(
        &mut self,
        doc_rate_hz: u32,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        self.control.record_start(doc_rate_hz, capture, done)
    }

    /// See [`EngineHandle::record_stop`].
    pub fn record_stop(&mut self) -> RecordState {
        self.control.record_stop()
    }

    /// See [`EngineHandle::record_state`].
    pub fn record_state(&self) -> RecordState {
        self.control.record_state()
    }

    /// See [`EngineHandle::live_take_peaks`].
    pub fn live_take_peaks(&self, start_bucket: u32, max: u32) -> Option<LiveTakePeaks> {
        self.control.live_take_peaks(start_bucket, max)
    }

    /// See [`EngineHandle::record_prepare`].
    pub fn record_prepare(
        &mut self,
        selection: Option<(u64, u64)>,
        prefs: RecordPrefs,
    ) -> Result<RecordPlan, RecordError> {
        self.control.record_prepare(selection, prefs)
    }

    /// See [`EngineHandle::record_start_op`]. The capture-writer runs inline in [`Self::tick`].
    pub fn record_start_op(
        &mut self,
        plan: RecordPlan,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        self.control.record_start_op(plan, capture, done)
    }

    /// See [`EngineHandle::calibration_start`].
    pub fn calibration_start(&mut self, offset_ns: i64) -> Result<(), CalibrationError> {
        self.control.calibration_start(offset_ns)
    }

    /// See [`EngineHandle::calibration_poll`].
    pub fn calibration_poll(&mut self) -> CalibrationStatus {
        self.control.calibration_poll()
    }

    /// See [`EngineHandle::calibration_cancel`].
    pub fn calibration_cancel(&mut self) {
        self.control.calibration_cancel();
    }

    /// See [`EngineHandle::nr_capture_prepare`].
    pub fn nr_capture_prepare(
        &mut self,
        hint: Option<usize>,
    ) -> Result<NrCapturePrep, RackApiError> {
        self.control.nr_capture_prepare(hint)
    }

    /// See [`EngineHandle::nr_capture_apply`].
    pub fn nr_capture_apply(
        &mut self,
        index: usize,
        blob: Vec<u8>,
    ) -> Result<RackSnapshot, RackApiError> {
        self.control.nr_capture_apply(index, blob)
    }

    /// See [`EngineHandle::response_curve`].
    pub fn response_curve(
        &self,
        index: usize,
        freqs_hz: Vec<f64>,
    ) -> Result<ResponseCurvePoints, RackApiError> {
        self.control.response_curve(index, freqs_hz)
    }

    /// See [`EngineHandle::rack_slot_state`].
    pub fn rack_slot_state(&self, index: usize) -> Result<ModuleState, RackApiError> {
        self.control.rack_slot_state(index)
    }

    /// See [`EngineHandle::rack_slot_module_id`].
    pub fn rack_slot_module_id(&self, index: usize) -> Option<String> {
        self.control.rack_slot_module_id(index)
    }

    /// See [`EngineHandle::rack_module_presets`].
    pub fn rack_module_presets(&self, module_id: &str) -> Vec<ModulePreset> {
        self.control.rack_module_presets(module_id)
    }
}

impl Drop for ManualEngine {
    fn drop(&mut self) {
        self.control.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use vox_rack::Registry;

    use super::EngineConfig;
    use crate::backend::fake::FakeBackend;
    use crate::control::{self, Control, ControlMsg};

    /// The control loop ticks on time while its inbox never runs empty (H-04 backlog).
    #[test]
    fn control_ticks_between_queued_messages() {
        let registry = Arc::new(Registry::with_factories(Vec::new()).unwrap());
        let config = EngineConfig::new(Arc::new(FakeBackend::new(1)), registry);
        let mut control = Control::new(config, None);
        let ticks = Arc::new(AtomicU64::new(0));
        let t = ticks.clone();
        control.set_telemetry_sink(Some(Box::new(move |_| {
            t.fetch_add(1, Ordering::Relaxed);
        })));
        let (tx, rx) = mpsc::channel();
        // ≥ 200 ms of queued work: 12 ticks due.
        for _ in 0..200 {
            let busy = |_: &mut Control| std::thread::sleep(Duration::from_millis(1));
            tx.send(ControlMsg::Call(Box::new(busy))).unwrap();
        }
        tx.send(ControlMsg::Quit).unwrap();
        control::run(&mut control, &rx);
        control.shutdown();
        let n = ticks.load(Ordering::Relaxed);
        assert!(n >= 6, "{n} ticks during ≥ 200 ms of queued messages");
    }
}
