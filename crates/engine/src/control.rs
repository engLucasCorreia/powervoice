//! The control thread (ADR-002 §1): the only consumer of the engine inbox and the only producer
//! of every queue into the audio threads. Owns the device configuration and the device-lost state
//! machine, the output stream (with its rack host), the input stream (S1-04: armed or
//! recording), the recording and its capture-writer, the transport, the playback document and the
//! telemetry sink. Ticks every ~16.7 ms (60 Hz telemetry): drains the RT events, services the
//! capture-writer, ticks the rack host, checks stream flags and the stall detectors, sends a
//! telemetry frame.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use vox_dsp::async_resample::MonitorResampler;
use vox_dsp::capture_resample::CaptureResampler;
use vox_project::{FreeSpaceProvider, TakeCapture};
use vox_rack::{
    ActivateConfig, ChannelLayout, MAX_BLOCK, ModuleDescriptor, ModulePreset, ModuleState,
    ProcessMode, RackHost, RackModel, RackNotice, RackOptions, Registry,
};

use crate::analyzer::{
    ANALYZER_RING_FRAMES, AnalyzerPublisher, AnalyzerResponse, AnalyzerSink, AnalyzerTap,
};
use crate::backend::{
    Backend, BackendError, BufferRequest, Direction, Enumerate, HostId, StallDetector,
    StreamHandle, StreamRequest, choose_default_host, flags,
};
use crate::capture::{
    self, CaptureHome, CaptureWriter, GapHome, LivePeaks, LivePeaksHandle, take_gap_home, take_home,
};
use crate::device_state::{Activity, DeviceAction, DeviceEvent, DeviceStateMachine, LinkState};
use crate::devices::{
    DEVICE_POLL_INTERVAL, DeviceList, DeviceNotice, DevicePollThread, DeviceWatcher, InputChannel,
    PollEvent, resolve_devices, resolve_host,
};
use crate::engine::{Clock, DevicesView, EngineConfig, EngineEvent, EventSink, PlaybackDoc};
use crate::input::{
    CAPTURE_RING_SECONDS, GAP_EVENT_CAPACITY, GapEvent, INPUT_CMD_CAPACITY, INPUT_EVENT_CAPACITY,
    InputCmd, InputEvent, InputShared, InputSide, InputSideParts, MonitorSlot, MonitorTx,
    input_callback, stop_code, take_monitor,
};
use crate::monitor::{
    self, MONITOR_RING_FRAMES, MonitorLink, MonitorOut, MonitorTap, StreamLatency,
};
use crate::output::{OutputCb, OutputParts, PartsSlot, take_parts};
use crate::prefs::DevicePrefs;
use crate::rack_api::{
    MAX_RESPONSE_CURVE_POINTS, NOISE_REDUCTION_MODULE_ID, NrCapturePrep, RackApiError, RackCommand,
    RackSnapshot, ResponseCurvePoints,
};
use crate::reader::{self, Reader, ReaderCmd};
use crate::record::{
    DISK_FLOOR_BYTES, LiveTakePeaks, MonitorMode, RECORD_BYTES_PER_SAMPLE, RecordDone, RecordError,
    RecordState, StopReason,
};
use crate::rt::{
    AUDIO_CMD_CAPACITY, AudioCmd, PLAYBACK_RING_PACKETS, RT_EVENT_CAPACITY, RtCounters, RtEvent,
};
use crate::telemetry::{
    InputMeter, Meter, ModuleTelemetryPublisher, ModuleTelemetrySink, TelemetryFrame,
    TelemetryRateGate, TelemetrySink, effective_telemetry_rate_hz, vxtm_flags,
};
use crate::transport::{Action, Transport, TransportCommand, TransportState};

// T-304 (SPEC-022): record operations and the latency calibration run.
use std::collections::VecDeque;

use crate::backend::frames_to_ns;
use crate::capture::OpShared;
use crate::record::{
    CalibrationCapture, CalibrationError, CalibrationStatus, CancelReason, OpResult, RecordPhase,
    RecordPhaseInfo,
};
use crate::record_op::{
    CaptureBlock, LISTEN_FADE_MS, RecordOpKind, RecordPlan, RecordPrefs, RunSpec, resolve_record,
    take_index_at,
};

/// Control tick = telemetry period (60 Hz, ADR-009).
pub(crate) const TICK: Duration = Duration::from_nanos(16_666_667);

/// A stop the input callback doesn't complete within this (stalled stream) is forced.
const FORCE_FINISH_NS: u64 = 1_000_000_000;
/// SPEC-002 §2.5: the disk-space query (free-space provider) runs about this often, not every
/// tick — it may hit the file system, and the remaining-time display only needs ~1 Hz anyway.
const DISK_CHECK_INTERVAL_NS: u64 = 1_000_000_000;
/// Inline (manual) writer: sync the take WAV every this many ticks (~1 s).
const INLINE_SYNC_TICKS: u32 = 60;
/// T-107 (SPEC-002 §2.7: "recomputed within 1 s of any change"): the monitoring latency readout
/// is recomputed this often (and at once after a mode change).
const LATENCY_READOUT_INTERVAL_NS: u64 = 500_000_000;

/// Messages to the control thread.
pub(crate) enum ControlMsg {
    /// Run a closure on the control thread (the handle's synchronous API).
    Call(Box<dyn FnOnce(&mut Control) + Send>),
    /// From the device-poll thread.
    Poll(PollEvent),
    /// Shut down.
    Quit,
}

struct OutputStream {
    handle: StreamHandle,
    slot: PartsSlot,
    rack: RackHost,
    cmds: Producer<AudioCmd>,
    events: Consumer<RtEvent>,
    counters: Arc<RtCounters>,
    stall: StallDetector,
    device: String,
    rate_hz: u32,
    doc_rate_hz: u32,
    buffer: BufferRequest,
    /// Generation of this stream's monitor ring.
    monitor_gen: u32,
    /// The document rate converts to the stream rate (else the reader streams nothing).
    playable: bool,
}

/// The open input stream (S1-04).
struct InputStream {
    handle: StreamHandle,
    cmds: Producer<InputCmd>,
    events: Consumer<InputEvent>,
    shared: Arc<InputShared>,
    /// Where the dropped callback leaves its monitor producer.
    slot: MonitorSlot,
    /// The capture-ring consumer while no take uses it.
    capture_rx: Option<Consumer<f32>>,
    /// Where a finished capture-writer returns it.
    capture_home: CaptureHome,
    /// H-10 item 4: the gap-ring consumer while no take uses it (mirrors `capture_rx`/
    /// `capture_home` — same stream, same lifetime, `record_start` takes it only for a
    /// non-resampled take).
    gap_rx: Option<Consumer<GapEvent>>,
    gap_home: GapHome,
    stall: StallDetector,
    rate_hz: u32,
    nominal_frames: Option<u32>,
    /// Generation of the output monitor ring this input feeds (`None`: none).
    monitor_gen: Option<u32>,
}

enum WriterLink {
    /// The capture-writer thread (engine).
    Thread(JoinHandle<()>),
    /// Drained from the tick (manual engine).
    Inline {
        writer: Option<Box<CaptureWriter>>,
        ticks: u32,
    },
}

/// A take being captured or finished.
struct Recording {
    link: WriterLink,
    shared: Arc<InputShared>,
    /// The input stream's rate: pairs with `captured` (both counted by the RT input callback, in
    /// device-rate frames) for the telemetry anchor while recording.
    rate_hz: u32,
    /// The take's rate (H-06: may differ from `rate_hz` — the capture-writer resamples then).
    /// `LivePeaks` (H-07) are recorded post-resample, so this is what `live_take_peaks` reports.
    doc_rate_hz: u32,
    /// Take samples captured so far, reached at app time `anchor_ns`.
    captured: u64,
    anchor_ns: u64,
    /// App time Stop was requested (`Some`: finishing).
    stop_at: Option<u64>,
    /// H-07: the take's running min/max peaks, updated by the capture-writer thread.
    peaks: LivePeaksHandle,
    /// T-304: the record operation this take belongs to (`None`: a new recording, SPEC-002).
    op: Option<OpState>,
}

/// T-304 (SPEC-022 §2.1, §4.3): a record operation's control-thread state.
struct OpState {
    plan: RecordPlan,
    take: u32,
    phase: RecordPhase,
    shared: Arc<OpShared>,
    /// The playback run of an aligned operation: its epoch and what it plays.
    run: Option<(u32, RunSpec)>,
    /// `monitor_gen` of the output stream the run plays on (another one = that output is gone).
    out_gen: u32,
    /// The run reported its end (the post-roll ran out).
    run_ended: bool,
    /// The output the run played on is gone.
    output_lost: bool,
    /// App time the record point `at` is heard (§4.3 `t_at_ns`).
    t_at: Option<i128>,
    /// The run's latest heard document position and its app time (before `t_at` is known).
    anchor: Option<(u64, u64)>,
    /// Recent input blocks: take position ↔ capture time (≈ 3 s).
    blocks: VecDeque<CaptureBlock>,
    /// The input stream's rate (the blocks' take positions are device frames).
    in_rate: u32,
    /// The record window's first take sample (document rate).
    k_start: Option<u64>,
    /// The decided end (`Some` once the operation stopped).
    end: Option<OpEnd>,
}

/// How a record operation ends (SPEC-022 §2.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpEnd {
    /// Cancelled: no edit, the document is unchanged.
    Cancel(CancelReason),
    /// The window ends at this document position (`None`: at the take's last sample).
    At(Option<u64>),
}

/// What ended a record operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndTrigger {
    /// Stop / Space / Record.
    User,
    /// The run's post-roll ran out, or a punch whose output is gone reached `E`.
    RunOver,
    /// The output device is gone.
    OutputLost,
    /// The take ended or must end for another reason (input loss, overflow, disk, shutdown).
    Failure(StopReason),
}

/// Input block history kept for [`OpState::blocks`] (the window start may lie up to input
/// latency + max offset in the past, SPEC-022 §4.5).
const OP_BLOCK_HISTORY_NS: u64 = 3_000_000_000;
/// A stop's capture runs this long past the window's last sample (SPEC-022 §4.3).
const OP_STOP_MARGIN_NS: u64 = 20_000_000;
/// A calibration run that hasn't finished after this is interrupted.
const CALIB_TIMEOUT_NS: u64 = 20_000_000_000;

fn push_capture_block(
    blocks: &mut VecDeque<CaptureBlock>,
    take_frames: u64,
    frames: u32,
    end_ns: u64,
    rate_hz: u32,
    history_ns: Option<u64>,
) {
    let start_ns = end_ns.saturating_sub(frames_to_ns(u64::from(frames), rate_hz));
    blocks.push_back(CaptureBlock {
        end_take: take_frames,
        frames,
        start_ns,
    });
    if let Some(h) = history_ns {
        while blocks.len() > 1 && blocks.front().is_some_and(|b| b.start_ns + h < start_ns) {
            blocks.pop_front();
        }
    }
}

impl OpState {
    fn doc_rate(&self) -> i128 {
        i128::from(self.plan.doc_rate_hz.max(1))
    }

    /// The document position heard at app time `now` (heard positions advance at the document
    /// rate, §4.3), once `at`'s heard time is known.
    fn heard_q(&self, now: u64) -> Option<i128> {
        self.t_at.map(|t| {
            i128::from(self.plan.at_samples)
                + (i128::from(now) - t) * self.doc_rate() / 1_000_000_000
        })
    }

    /// App time document position `q` is heard.
    fn heard_time(&self, q: u64) -> Option<i128> {
        self.t_at.map(|t| {
            t + (i128::from(q) - i128::from(self.plan.at_samples)) * 1_000_000_000 / self.doc_rate()
        })
    }

    /// §4.3: once `at` was heard and an input block covers `t_at + δ`, the window's first take
    /// sample `k_start = k_b + round((t_at + δ − c_b)·r/1e9)` (document rate). Returns it once.
    fn resolve_k_start(&mut self) -> Option<u64> {
        if self.k_start.is_some() {
            return None;
        }
        let target = self.t_at? + i128::from(self.plan.offset_ns);
        let k_dev = take_index_at(&self.blocks, target, self.in_rate)?;
        let (doc, dev) = (
            i128::from(self.plan.doc_rate_hz.max(1)),
            i128::from(self.in_rate.max(1)),
        );
        let k = if doc == dev {
            k_dev
        } else {
            (k_dev * doc + dev / 2).div_euclid(dev)
        };
        let k = u64::try_from(k.max(0)).unwrap_or(0);
        self.k_start = Some(k);
        Some(k)
    }

    fn phase_info(&self, phase: RecordPhase, doc_pos: u64, app_ns: u64) -> RecordPhaseInfo {
        RecordPhaseInfo {
            take: self.take,
            kind: self.plan.kind,
            phase,
            doc_pos_samples: doc_pos,
            app_ns,
        }
    }
}

/// T-304 (SPEC-022 §2.14, §4.7): a running latency calibration.
struct CalibRun {
    /// δ applied to the repetition starts (a Verify pass).
    offset_ns: i64,
    /// The capture ring's consumer while the run holds it.
    rx: Option<Consumer<f32>>,
    /// The input stream the ring belongs to (the consumer goes back only to it).
    in_shared: Arc<InputShared>,
    recording: Vec<f32>,
    blocks: VecDeque<CaptureBlock>,
    /// Heard time of each repetition's first sample.
    rep_times: Vec<u64>,
    started_ns: u64,
    rate_hz: u32,
    out_gen: u32,
    restore_monitor: MonitorMode,
    restore_armed: bool,
    result: Option<Result<CalibrationCapture, CalibrationError>>,
}

/// Latest heard position reported by the output callback.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    epoch: u32,
    pos: u64,
    time_ns: u64,
}

enum PollLink {
    None,
    Thread(DevicePollThread),
    Manual(Option<DeviceWatcher>),
}

enum ReaderLink {
    Thread {
        tx: Option<mpsc::Sender<ReaderCmd>>,
        join: Option<JoinHandle<()>>,
    },
    Inline(Box<Reader>),
}

fn is_input_notice(n: &DeviceNotice) -> bool {
    matches!(
        n,
        DeviceNotice::ChannelFallback { .. }
            | DeviceNotice::DeviceNotFound {
                direction: Direction::Input,
                ..
            }
    )
}

pub(crate) struct Control {
    backend: Arc<dyn Backend>,
    registry: Arc<Registry>,
    rack_model: RackModel,
    clock: Clock,
    events: EventSink,
    prefs: DevicePrefs,
    hosts: Vec<HostId>,
    host: Option<HostId>,
    host_notice_sent: bool,
    poll: PollLink,
    list: DeviceList,
    devices: DeviceStateMachine,
    /// Backend id of the configured output device (twins: lookup prefers it).
    out_device_id: Option<String>,
    pending_open: bool,
    force_configure: bool,
    output: Option<OutputStream>,
    reader: ReaderLink,
    doc: Option<PlaybackDoc>,
    transport: Transport,
    anchor: Option<Anchor>,
    meter: Meter,
    last_underruns: u32,
    xrun: bool,
    telemetry: Option<TelemetrySink>,
    seq: u32,
    /// Module telemetry (`VXMT`, H-03): the rack slots' meters.
    module_telemetry: ModuleTelemetryPublisher,
    /// Live output analyzer (`VXSA`, T-208): the RT tap's consumer + BH4 FFT / band / EMA
    /// pipeline, published only while at least one subscriber exists.
    analyzer: AnalyzerPublisher,
    /// H-16 (SPEC-003 §3, ADR-009): gates how often `emit_telemetry`/`module_telemetry`/
    /// `analyzer` actually publish, independent of the fixed 60 Hz control tick.
    rate_gate: TelemetryRateGate,
    last_state: Option<TransportState>,
    // --- Input / recording (S1-04) ---
    /// Capture-writer on its own thread (`Engine`) or inline in the tick (`ManualEngine`).
    threaded: bool,
    /// Backend id of the configured input device (twins: lookup prefers it).
    in_device_id: Option<String>,
    input: Option<InputStream>,
    /// The input's capabilities are still pending: resolve again on the next device diff.
    input_pending: bool,
    /// Selected input channel (shared with the input callback: switches without reopening).
    in_channel: InputChannel,
    armed: bool,
    record_rate_hz: u32,
    monitor_mode: MonitorMode,
    /// Monitoring is audible (a tap other than Off was sent).
    monitor_active: bool,
    /// T-107: the input pushes into the monitor ring (linked and armed/recording, any mode).
    monitor_feeding: bool,
    /// T-107: the last `AudioCmd::Monitor` sent: (tap, input rate, F* in input frames).
    monitor_sent: Option<(MonitorTap, u32, u32)>,
    /// Producer of the current output's monitor ring while no input stream holds it.
    monitor_tx: Option<MonitorTx>,
    monitor_gen: u32,
    /// T-107 (SPEC-002 §4.4): observed input/output periods and latencies (F*, readout).
    in_lat: StreamLatency,
    out_lat: StreamLatency,
    /// T-107: the published latency readout (µs, 0.1 ms steps), its last recompute time, and
    /// whether a monitoring change asks for a recompute now.
    monitor_latency_us: Option<u32>,
    latency_checked_ns: Option<u64>,
    latency_dirty: bool,
    recording: Option<Recording>,
    in_meter: InputMeter,
    last_record: Option<RecordState>,
    /// H-11 (SPEC-002 §2.5, A-010): the free-space provider (the real one, or a fake in tests).
    disk_space: Arc<dyn FreeSpaceProvider>,
    /// The volume recordings are written to.
    record_volume: PathBuf,
    /// Fake/app clock time of the last [`Self::disk_space`] query.
    disk_checked_ns: Option<u64>,
    /// The last query's result (`None`: never queried yet, or it failed).
    disk_free_bytes: Option<u64>,
    /// T-304 (SPEC-022 §2.14): a latency calibration run (running, or finished until polled).
    calib: Option<CalibRun>,
}

impl Control {
    /// `inbox`: `Some` = threaded (reader, device-poll and capture-writer threads, poll events
    /// arrive through the inbox); `None` = manual (reader and writer inline, devices polled by
    /// [`Self::poll_devices`]).
    pub(crate) fn new(config: EngineConfig, inbox: Option<mpsc::Sender<ControlMsg>>) -> Self {
        let EngineConfig {
            backend,
            registry,
            rack,
            prefs,
            events,
            clock,
            record_rate_hz,
            disk_space,
            record_volume,
        } = config;
        let threaded = inbox.is_some();
        let (reader, poll) = match inbox {
            Some(tx) => {
                let (rtx, rrx) = mpsc::channel();
                let reader = match std::thread::Builder::new()
                    .name("vox-reader".into())
                    .spawn(move || reader::run(Reader::new(), rrx))
                {
                    Ok(join) => ReaderLink::Thread {
                        tx: Some(rtx),
                        join: Some(join),
                    },
                    Err(_) => ReaderLink::Inline(Box::new(Reader::new())),
                };
                let poll = match prefs.host.or_else(|| backend.default_host()) {
                    Some(h) => DevicePollThread::spawn(
                        backend.clone(),
                        h,
                        DEVICE_POLL_INTERVAL,
                        move |ev| {
                            let _ = tx.send(ControlMsg::Poll(ev));
                        },
                    )
                    .map_or(PollLink::None, PollLink::Thread),
                    None => PollLink::None,
                };
                (reader, poll)
            }
            None => (
                ReaderLink::Inline(Box::new(Reader::new())),
                PollLink::Manual(None),
            ),
        };
        let in_channel = InputChannel::new(prefs.input_channel);
        Self {
            backend,
            registry,
            rack_model: rack,
            clock,
            events,
            prefs,
            hosts: Vec::new(),
            host: None,
            host_notice_sent: false,
            poll,
            list: DeviceList::default(),
            devices: DeviceStateMachine::new(),
            out_device_id: None,
            pending_open: false,
            force_configure: false,
            output: None,
            reader,
            doc: None,
            transport: Transport::default(),
            anchor: None,
            meter: Meter::default(),
            last_underruns: 0,
            xrun: false,
            telemetry: None,
            seq: 0,
            module_telemetry: ModuleTelemetryPublisher::default(),
            analyzer: AnalyzerPublisher::default(),
            // SPEC-003 §3 factory default: 60 Hz (matches `AnalyzerPublisher::default`'s rate).
            rate_gate: TelemetryRateGate::new(60),
            last_state: None,
            threaded,
            in_device_id: None,
            input: None,
            input_pending: false,
            in_channel,
            armed: false,
            record_rate_hz: record_rate_hz.max(1),
            monitor_mode: MonitorMode::Off,
            monitor_active: false,
            monitor_feeding: false,
            monitor_sent: None,
            monitor_tx: None,
            monitor_gen: 0,
            in_lat: StreamLatency::default(),
            out_lat: StreamLatency::default(),
            monitor_latency_us: None,
            latency_checked_ns: None,
            latency_dirty: false,
            recording: None,
            in_meter: InputMeter::default(),
            last_record: None,
            disk_space,
            record_volume,
            disk_checked_ns: None,
            disk_free_bytes: None,
            calib: None,
        }
    }

    /// Finishes a running take (kept), closes the streams and stops the helper threads.
    pub(crate) fn shutdown(&mut self) {
        self.calibration_cancel();
        self.stop_recording(StopReason::Shutdown);
        if let Some(rec) = self.recording.take() {
            match rec.link {
                WriterLink::Thread(join) => {
                    let _ = join.join();
                }
                WriterLink::Inline {
                    writer: Some(mut w),
                    ..
                } => {
                    // Forced: the first drain completes.
                    w.drain();
                    (*w).finish();
                }
                WriterLink::Inline { writer: None, .. } => {}
            }
        }
        self.close_input(false);
        self.close_output();
        self.poll = PollLink::None;
        if let ReaderLink::Thread { tx, join } = &mut self.reader {
            drop(tx.take());
            if let Some(j) = join.take() {
                let _ = j.join();
            }
        }
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn notify(&self, n: DeviceNotice) {
        (self.events)(EngineEvent::Notice(n));
    }

    fn reader_send(&mut self, cmd: ReaderCmd) {
        match &mut self.reader {
            ReaderLink::Thread { tx: Some(tx), .. } => {
                let _ = tx.send(cmd);
            }
            ReaderLink::Thread { .. } => {}
            ReaderLink::Inline(r) => {
                r.handle(cmd);
                r.fill();
            }
        }
    }

    fn audio_cmd(&mut self, cmd: AudioCmd) {
        if let Some(out) = self.output.as_mut() {
            let _ = out.cmds.push(cmd);
        }
    }

    fn doc_rate(&self) -> u32 {
        self.doc.as_ref().map_or(0, |d| d.snapshot.sample_rate_hz)
    }

    fn can_play(&self) -> bool {
        self.transport.len() > 0 && self.output.as_ref().is_some_and(|o| o.playable)
    }

    /// A take is being captured (not yet stopping).
    fn capturing(&self) -> bool {
        self.recording.as_ref().is_some_and(|r| r.stop_at.is_none())
    }

    // --- Transport -------------------------------------------------------------------------

    pub(crate) fn transport_state(&self) -> TransportState {
        let playing = self.transport.playing();
        TransportState {
            playing,
            playhead_samples: if playing {
                self.transport.display_pos()
            } else {
                self.transport.playhead()
            },
            play_start_samples: self.transport.play_start(),
            doc_len_samples: self.transport.len(),
            doc_rate_hz: self.doc_rate(),
            can_play: self.can_play(),
        }
    }

    fn emit_state_if_changed(&mut self) {
        let state = self.transport_state();
        if self.last_state.as_ref() != Some(&state) {
            self.last_state = Some(state.clone());
            (self.events)(EngineEvent::Transport(state));
        }
    }

    /// The heard position now: extrapolated from the latest anchor while playing.
    fn heard_now(&self, now: u64) -> u64 {
        if !self.transport.playing() {
            return self.transport.playhead();
        }
        match self.anchor {
            Some(a) if a.epoch == self.transport.epoch() => {
                let rate = i128::from(self.doc_rate());
                let dt = i128::from(now) - i128::from(a.time_ns);
                let p = i128::from(a.pos) + dt * rate / 1_000_000_000;
                let lo = i128::from(self.transport.display_pos().min(a.pos));
                p.clamp(lo, i128::from(self.transport.len()).max(lo)) as u64
            }
            _ => self.transport.display_pos(),
        }
    }

    /// While recording, Stop / Pause / Play / Play-pause stop the take (SPEC-002 §2.2: the Stop
    /// control and the Space key; the UI sends Play or Pause for Space) and every other transport
    /// command is ignored (seeking is disabled while recording).
    pub(crate) fn transport_command(&mut self, cmd: TransportCommand) -> TransportState {
        if self.calibrating() {
            // SPEC-022 §2.14: the calibration run owns the output until it ends.
            return self.transport_state();
        }
        if self.recording.is_some() {
            if self.capturing()
                && matches!(
                    cmd,
                    TransportCommand::Stop
                        | TransportCommand::Pause
                        | TransportCommand::Play
                        | TransportCommand::PlayPause
                )
            {
                self.stop_recording(StopReason::User);
                self.update_monitor();
                self.emit_record_if_changed();
            }
            return self.transport_state();
        }
        let heard = self.heard_now(self.now());
        let can_play = self.can_play();
        let alive = self.output.is_some();
        if let Some(a) = self.transport.command(cmd, can_play, heard, alive) {
            self.execute(a);
        }
        self.emit_state_if_changed();
        self.transport_state()
    }

    fn execute(&mut self, action: Action) {
        match action {
            Action::Start { epoch, pos, reset } => {
                self.anchor = None;
                self.reader_send(ReaderCmd::Start { epoch, pos });
                self.audio_cmd(AudioCmd::Play { epoch, pos, reset });
            }
            Action::Seek { epoch, pos } => {
                self.anchor = None;
                self.reader_send(ReaderCmd::Start { epoch, pos });
                self.audio_cmd(AudioCmd::Seek { epoch, pos });
            }
            Action::Stop => {
                self.audio_cmd(AudioCmd::Stop);
                self.reader_send(ReaderCmd::Stop);
            }
        }
    }

    /// Engine-initiated stop (device loss, new document): Pause semantics (D-018).
    fn engine_stop(&mut self) {
        if !self.transport.playing() {
            return;
        }
        let heard = self.heard_now(self.now());
        let alive = self.output.is_some();
        if let Some(a) = self.transport.pause(heard, alive) {
            self.execute(a);
        }
    }

    pub(crate) fn set_selection(&mut self, sel: Option<(u64, u64)>) {
        self.transport.set_selection(sel);
    }

    // --- Document --------------------------------------------------------------------------

    /// Replaces the playback document (stops playback first, Pause semantics). Reopens the
    /// output when the device rate should follow the new document rate (ADR-002 §5).
    pub(crate) fn set_document(&mut self, doc: Option<PlaybackDoc>) {
        self.engine_stop();
        let len = doc.as_ref().map_or(0, |d| d.snapshot.len_samples);
        self.doc = doc.clone();
        self.reader_send(ReaderCmd::SetDoc(doc));
        self.transport.set_doc(len);
        let reopen = match (&self.output, &self.doc) {
            (Some(out), Some(d)) => {
                let doc_rate = d.snapshot.sample_rate_hz;
                let supported = self
                    .list
                    .snapshot
                    .get(&out.device)
                    .and_then(|i| i.caps(Direction::Output))
                    .is_some_and(|c| c.supports_rate(doc_rate));
                out.doc_rate_hz != doc_rate || (supported && out.rate_hz != doc_rate)
            }
            _ => false,
        };
        if reopen {
            self.open_output(false);
        }
        self.emit_state_if_changed();
    }

    // --- Rack (S3-01, SPEC-012 §2.1–§2.6) ---------------------------------------------------

    /// The registered modules (the Add-module menu; always available, even with no live rack).
    pub(crate) fn rack_registry(&self) -> Vec<ModuleDescriptor> {
        self.registry.descriptors()
    }

    /// `module_id`'s factory presets (T-406, ADR-005 §2 `ModuleFactory::presets`); empty for an
    /// unregistered id. Always available, even with no live rack — a pure function of the
    /// registry (assembled once at startup by the composition root).
    pub(crate) fn rack_module_presets(&self, module_id: &str) -> Vec<ModulePreset> {
        self.registry
            .get(module_id)
            .map(|f| f.presets())
            .unwrap_or_default()
    }

    /// The current rack state, empty when there is no live rack (no output stream open).
    pub(crate) fn rack_snapshot(&self) -> RackSnapshot {
        self.output
            .as_ref()
            .map(|out| RackSnapshot::read(&out.rack))
            .unwrap_or_default()
    }

    /// The current rack as plain data (H-08 handoff: export renders `RackModel::default()` today
    /// — this is the getter that later wires it to the live rack instead). Reads straight from
    /// the live host when an output stream is open (always current, unlike the `rack_model` field
    /// this struct only refreshes on a rack notice) and falls back to the last-known model
    /// otherwise (no live rack, e.g. no output device).
    pub(crate) fn rack_model(&self) -> RackModel {
        self.output
            .as_ref()
            .map_or_else(|| self.rack_model.clone(), |out| out.rack.model())
    }

    /// T-306: replaces the whole rack (document open, sidecar carrying a rack; or recovery).
    /// Unlike [`Self::rack_apply`], this succeeds even with no live output stream — `rack_model`
    /// (the source of truth across output open/close, see [`Self::rack_model`]) is updated either
    /// way, so a rack applied before the output opens is still the one it opens with.
    pub(crate) fn rack_load_model(
        &mut self,
        model: RackModel,
    ) -> Result<RackSnapshot, RackApiError> {
        if let Some(out) = self.output.as_mut() {
            out.rack
                .load_model(&model)
                .map_err(|e| RackApiError::Rack(e.to_string()))?;
            let notices = out.rack.take_notices();
            self.rack_model = out.rack.model();
            for n in notices {
                (self.events)(EngineEvent::Rack(n));
            }
        } else {
            self.rack_model = model;
        }
        let snapshot = self.rack_snapshot();
        (self.events)(EngineEvent::RackChanged(snapshot.clone()));
        Ok(snapshot)
    }

    /// Applies one rack command and returns the resulting snapshot.
    pub(crate) fn rack_apply(&mut self, cmd: RackCommand) -> Result<RackSnapshot, RackApiError> {
        let (result, notices) = {
            let Some(out) = self.output.as_mut() else {
                return Err(RackApiError::Unavailable);
            };
            let host = &mut out.rack;
            let result = match cmd {
                RackCommand::Add { module_id, index } => {
                    host.insert_module(index, &module_id).map(|_| ())
                }
                RackCommand::Remove { index } => host.remove(index),
                RackCommand::Move { from, to } => host.move_slot(from, to),
                RackCommand::SetBypass { index, on } => host.set_bypass(index, on),
                RackCommand::SetAb { on } => {
                    host.set_ab(on);
                    Ok(())
                }
                RackCommand::SetParamNormalized { index, id, value } => {
                    host.set_param_normalized(index, id, value).map(|_| ())
                }
                RackCommand::SetParamText { index, id, text } => {
                    host.set_param_text(index, id, &text).map(|_| ())
                }
                RackCommand::SetParamPlain { index, id, value } => {
                    host.set_param(index, id, value).map(|_| ())
                }
                RackCommand::Restart { index } => host.restart(index),
                RackCommand::ApplyModulePreset { index, state } => {
                    host.apply_module_preset(index, &state)
                }
                RackCommand::ResetToDefault { index } => host.reset_to_default(index),
            };
            (result, host.take_notices())
        };
        self.handle_rack_notices(notices);
        result.map_err(|e| RackApiError::Rack(e.to_string()))?;
        let snapshot = self.rack_snapshot();
        (self.events)(EngineEvent::RackChanged(snapshot.clone()));
        Ok(snapshot)
    }

    /// The committed state of slot `index` (T-406: what "Save as preset" reads). Read-only.
    pub(crate) fn rack_slot_state(&self, index: usize) -> Result<ModuleState, RackApiError> {
        let Some(out) = self.output.as_ref() else {
            return Err(RackApiError::Unavailable);
        };
        out.rack
            .slot_state(index)
            .map_err(|e| RackApiError::Rack(e.to_string()))
    }

    /// The module id of slot `index` (registry key, no `@version`); `None` for a placeholder, an
    /// out-of-range index, or when there is no live rack (T-406: which preset menu to show).
    pub(crate) fn rack_slot_module_id(&self, index: usize) -> Option<String> {
        self.output.as_ref()?.rack.slot_module_id(index)
    }

    /// Forwards rack notices (ADR-005 §7 mirror echoes, latency changes, failures, restarts),
    /// marks the monitoring latency readout for a recompute on a latency change (T-401),
    /// keeps `rack_model` (the source of truth across output open/close) in sync, and — for a
    /// failure or a restart, which can bring a new schema the per-parameter notices don't cover
    /// (H-01 handoff) — emits a full [`EngineEvent::RackChanged`] snapshot built right here, on
    /// the control thread (an [`EventSink`] must never call back into the engine).
    fn handle_rack_notices(&mut self, notices: Vec<RackNotice>) {
        if notices.is_empty() {
            return;
        }
        if let Some(out) = self.output.as_ref() {
            self.rack_model = out.rack.model();
        }
        let needs_refresh = notices.iter().any(|n| {
            matches!(
                n,
                RackNotice::SlotFailed { .. }
                    | RackNotice::SlotRestarted { .. }
                    | RackNotice::SlotLoaded { .. }
            )
        });
        // T-401 (SPEC-012 §2.5, AC-8): a rack latency change reaches the monitoring readout within
        // 100 ms — recomputed at this tick's end, not at the next 0.5 s refresh.
        if notices
            .iter()
            .any(|n| matches!(n, RackNotice::LatencyChanged { .. }))
        {
            self.latency_dirty = true;
        }
        for n in notices {
            (self.events)(EngineEvent::Rack(n));
        }
        if needs_refresh {
            (self.events)(EngineEvent::RackChanged(self.rack_snapshot()));
        }
    }

    /// Capture Noise Print, step 1 (SPEC-014 §2.3 "target slot"): resolves the last-focused NR
    /// slot, else the first one, else inserts a default Noise Reduction slot at the top — and
    /// reads out everything the job needs before it starts reading audio (the upstream model,
    /// the slot's current values, its `NoiseProfile` handle). A structural change (an insertion)
    /// is broadcast as a `RackChanged` snapshot, same as every mutating [`RackCommand`].
    pub(crate) fn nr_capture_prepare(
        &mut self,
        hint: Option<usize>,
    ) -> Result<NrCapturePrep, RackApiError> {
        let (prep, notices) = {
            let Some(out) = self.output.as_mut() else {
                return Err(RackApiError::Unavailable);
            };
            let host = &mut out.rack;
            let (index, inserted) = host
                .resolve_or_insert_noise_profile_slot(hint, NOISE_REDUCTION_MODULE_ID)
                .map_err(|e| RackApiError::Rack(e.to_string()))?;
            let notices = host.take_notices();
            let Some(extension) = host.noise_profile_extension(index) else {
                return Err(RackApiError::Rack(
                    "the target slot has no noise-profile support".into(),
                ));
            };
            let upstream_model = host.upstream_model(index);
            let values = host
                .slot_info(index)
                .map(|info| {
                    info.params
                        .iter()
                        .map(|p| host.param_value(index, p.id).unwrap_or(p.default))
                        .collect()
                })
                .unwrap_or_default();
            (
                NrCapturePrep {
                    index,
                    inserted,
                    upstream_model,
                    values,
                    extension,
                },
                notices,
            )
        };
        self.handle_rack_notices(notices);
        if prep.inserted {
            (self.events)(EngineEvent::RackChanged(self.rack_snapshot()));
        }
        Ok(prep)
    }

    /// Capture Noise Print, step 2 (SPEC-014 §2.3 "Result"): installs `blob` as the target slot's
    /// committed noise print, live (ADR-005 §12's replacement crossfade — the same path `Restart`
    /// and preset loading use).
    pub(crate) fn nr_capture_apply(
        &mut self,
        index: usize,
        blob: Vec<u8>,
    ) -> Result<RackSnapshot, RackApiError> {
        let (result, notices) = {
            let Some(out) = self.output.as_mut() else {
                return Err(RackApiError::Unavailable);
            };
            let host = &mut out.rack;
            let result = host.replace_noise_print(index, blob);
            (result, host.take_notices())
        };
        self.handle_rack_notices(notices);
        result.map_err(|e| RackApiError::Rack(e.to_string()))?;
        let snapshot = self.rack_snapshot();
        (self.events)(EngineEvent::RackChanged(snapshot.clone()));
        Ok(snapshot)
    }

    /// The EQ graph's response curve (S3-07, SPEC-015 §2.6.6): evaluates slot `index`'s
    /// `ResponseCurve` extension at `freqs_hz` (truncated to
    /// [`MAX_RESPONSE_CURVE_POINTS`](crate::rack_api::MAX_RESPONSE_CURVE_POINTS) if longer) from
    /// the parameter mirror's current (target) values, at the rack's rate. Read-only: never
    /// touches the audio thread, so it's safe to call every animation frame.
    pub(crate) fn response_curve(
        &self,
        index: usize,
        mut freqs_hz: Vec<f64>,
    ) -> Result<ResponseCurvePoints, RackApiError> {
        freqs_hz.truncate(MAX_RESPONSE_CURVE_POINTS);
        let Some(out) = self.output.as_ref() else {
            return Err(RackApiError::Unavailable);
        };
        let host = &out.rack;
        let ext = host.response_curve_extension(index).ok_or_else(|| {
            RackApiError::Rack("the target slot has no response-curve support".into())
        })?;
        let info = host.slot_info(index).ok_or(RackApiError::Rack(format!(
            "slot index {index} out of range"
        )))?;
        let values: Vec<f64> = info
            .params
            .iter()
            .map(|p| host.param_value(index, p.id).unwrap_or(p.default))
            .collect();
        let sample_rate_hz = host.config().sample_rate;
        let mut total_db = vec![0.0; freqs_hz.len()];
        ext.magnitude_db(&values, sample_rate_hz, &freqs_hz, &mut total_db);
        let component_count = ext.component_count(&values);
        let mut components_db = Vec::with_capacity(component_count);
        for c in 0..component_count {
            let mut out = vec![0.0; freqs_hz.len()];
            ext.component_magnitude_db(c, &values, sample_rate_hz, &freqs_hz, &mut out);
            components_db.push(out);
        }
        Ok(ResponseCurvePoints {
            freqs_hz,
            sample_rate_hz,
            total_db,
            components_db,
        })
    }

    // --- Devices ---------------------------------------------------------------------------

    pub(crate) fn devices_view(&self) -> DevicesView {
        DevicesView {
            hosts: self.hosts.clone(),
            host: self.host,
            snapshot: self.list.snapshot.clone(),
            prefs: self.prefs.clone(),
            output_device: self
                .output
                .as_ref()
                .map(|o| o.device.clone())
                .or_else(|| self.devices.device(Direction::Output).map(str::to_owned)),
            output_rate_hz: self.output.as_ref().map(|o| o.rate_hz),
            output_buffer: self.output.as_ref().map(|o| o.buffer),
            output_status: self.devices.status(Direction::Output),
            input_device: self.devices.device(Direction::Input).map(str::to_owned),
            input_status: self.devices.status(Direction::Input),
        }
    }

    /// Applies new device preferences (Settings → Audio Devices). Refused while recording
    /// (device settings are disabled then, SPEC-002 §2.2): the current view is returned.
    pub(crate) fn select_devices(&mut self, prefs: DevicePrefs) -> DevicesView {
        if self.recording.is_some() {
            return self.devices_view();
        }
        self.engine_stop();
        let new_host = if self.hosts.is_empty() {
            prefs.host.or(self.host)
        } else {
            resolve_host(prefs.host, &self.hosts, choose_default_host(&self.hosts)).map(|(h, _)| h)
        };
        let input_changed = prefs.input_device != self.prefs.input_device;
        self.prefs = prefs;
        self.out_device_id = None;
        if new_host.is_some() && new_host != self.host {
            self.switch_host(new_host);
            self.force_configure = true;
        } else {
            self.open_output(true);
            if input_changed {
                self.close_input(true);
                self.in_device_id = None;
                self.configure_input(true);
                if self.armed {
                    self.open_input();
                }
            } else {
                // Same device: a new channel applies without reopening.
                self.configure_input(false);
            }
        }
        let view = self.devices_view();
        (self.events)(EngineEvent::Devices(view.clone()));
        self.emit_state_if_changed();
        self.update_monitor();
        self.emit_record_if_changed();
        view
    }

    fn switch_host(&mut self, host: Option<HostId>) {
        if self.recording.is_some() {
            self.stop_recording(StopReason::InputLost);
        }
        self.close_output();
        self.close_input(false);
        self.device_event(DeviceEvent::Configured {
            dir: Direction::Output,
            device: None,
        });
        self.device_event(DeviceEvent::Configured {
            dir: Direction::Input,
            device: None,
        });
        self.host = host;
        self.list = DeviceList::default();
        self.pending_open = true;
        self.set_poll_host();
    }

    fn set_poll_host(&mut self) {
        let Some(h) = self.host else { return };
        match &mut self.poll {
            PollLink::Thread(t) => t.set_host(h),
            PollLink::Manual(w) => match w {
                Some(w) => w.set_host(h),
                None => *w = Some(DeviceWatcher::new(h)),
            },
            PollLink::None => {}
        }
    }

    /// Manual mode: one device-poll pass (hosts + devices), like the poll thread's.
    pub(crate) fn poll_devices(&mut self) {
        let available = self.backend.hosts();
        let default = choose_default_host(&available);
        self.on_poll(PollEvent::Hosts { available, default });
        let diffs = match &mut self.poll {
            PollLink::Manual(Some(w)) => w.pass(self.backend.as_ref(), Enumerate::Cached).0,
            _ => return,
        };
        for d in diffs {
            self.on_poll(PollEvent::Devices(d));
        }
    }

    pub(crate) fn on_poll(&mut self, ev: PollEvent) {
        match ev {
            PollEvent::Hosts { available, default } => {
                self.hosts = available;
                let Some((h, notice)) = resolve_host(self.prefs.host, &self.hosts, default) else {
                    return;
                };
                if let Some(n) = notice
                    && !self.host_notice_sent
                {
                    self.host_notice_sent = true;
                    self.notify(n);
                }
                if self.host != Some(h) {
                    self.switch_host(Some(h));
                } else if matches!(self.poll, PollLink::Manual(None)) {
                    self.set_poll_host();
                }
            }
            PollEvent::Devices(diff) => {
                if Some(diff.host) != self.host {
                    return;
                }
                self.list.apply(&diff);
                if let Some(present) = self.output_present() {
                    self.device_event(DeviceEvent::Poll {
                        dir: Direction::Output,
                        present,
                    });
                }
                let state = self.devices.state(Direction::Output);
                if self.output.is_none()
                    && (self.pending_open
                        || matches!(state, LinkState::NotSelected | LinkState::Idle))
                {
                    self.open_output(false);
                }
                self.poll_input();
                (self.events)(EngineEvent::Devices(self.devices_view()));
                self.emit_state_if_changed();
                self.update_monitor();
                self.emit_record_if_changed();
            }
            PollEvent::Error { .. } => {}
        }
    }

    /// Input side of a device diff: presence (loss/recovery), first configuration, deferred open.
    fn poll_input(&mut self) {
        if let Some(present) = self.input_present() {
            self.device_event(DeviceEvent::Poll {
                dir: Direction::Input,
                present,
            });
        }
        let state = self.devices.state(Direction::Input);
        if self.input_pending
            || (state == LinkState::NotSelected && self.prefs.input_device.is_some())
        {
            self.configure_input(false);
        }
        if self.armed
            && self.input.is_none()
            && matches!(
                self.devices.state(Direction::Input),
                LinkState::Idle | LinkState::NotSelected
            )
        {
            self.open_input();
        }
    }

    /// Is the configured output device listed? Prefers its backend id (twin devices).
    fn output_present(&self) -> Option<bool> {
        self.present(Direction::Output, self.out_device_id.as_ref())
    }

    /// Is the configured input device listed? Prefers its backend id (twin devices).
    fn input_present(&self) -> Option<bool> {
        self.present(Direction::Input, self.in_device_id.as_ref())
    }

    fn present(&self, dir: Direction, id: Option<&String>) -> Option<bool> {
        let name = self.devices.device(dir)?;
        Some(match id {
            Some(id) => self.list.snapshot.devices_for(dir).any(|d| &d.id == id),
            None => self.list.is_present(name, dir),
        })
    }

    fn device_event(&mut self, ev: DeviceEvent) {
        let act = Activity {
            playing: self.transport.playing(),
            recording: self.capturing(),
            monitoring: self.monitor_active,
        };
        for action in self.devices.handle(ev, act) {
            match action {
                DeviceAction::StopPlayback => self.engine_stop(),
                DeviceAction::StopRecording => self.stop_recording(StopReason::InputLost),
                DeviceAction::StopMonitoring => self.monitor_reset(),
                DeviceAction::CloseStream(Direction::Output) => self.close_output(),
                DeviceAction::CloseStream(Direction::Input) => self.close_input(false),
                DeviceAction::ReopenStream(Direction::Output) => self.open_output(false),
                DeviceAction::ReopenStream(Direction::Input) => self.open_input(),
                DeviceAction::Notice(n) => self.notify(n),
            }
        }
    }

    /// Resolves the prefs against the current device list and opens the output stream at the
    /// document rate when the device supports it (ADR-002 §5). `force`: a fresh user choice.
    fn open_output(&mut self, force: bool) {
        if self.output.is_some() {
            self.close_output();
        }
        let Some(host) = self.host else { return };
        let res = resolve_devices(&self.prefs, host, &self.list.snapshot);
        if res.caps_pending {
            // Wait for the capability diff (ALSA reads them in a second phase).
            self.pending_open = true;
            self.force_configure |= force;
            return;
        }
        let force = force || std::mem::take(&mut self.force_configure);
        self.pending_open = false;
        if force || self.devices.state(Direction::Output) == LinkState::NotSelected {
            for n in res.notices.iter().filter(|n| !is_input_notice(n)) {
                self.notify(n.clone());
            }
        }
        let fallback = res.notices.iter().any(|n| {
            matches!(
                n,
                DeviceNotice::RateFallback { .. } | DeviceNotice::BufferFallback { .. }
            )
        });
        let Some(mut req) = res.output_request() else {
            if self.devices.device(Direction::Output).is_some() {
                self.device_event(DeviceEvent::Configured {
                    dir: Direction::Output,
                    device: None,
                });
            }
            return;
        };
        if !force
            && let Some(id) = &self.out_device_id
            && let Some(d) = self
                .list
                .snapshot
                .devices_for(Direction::Output)
                .find(|d| &d.id == id)
        {
            req.device.name = d.name.clone();
        }
        let info = self.list.snapshot.get(&req.device.name).cloned();
        self.out_device_id = info.as_ref().map(|d| d.id.clone());
        let doc_rate = self.doc.as_ref().map(|d| d.snapshot.sample_rate_hz);
        if let (Some(r), Some(caps)) = (
            doc_rate,
            info.as_ref().and_then(|d| d.caps(Direction::Output)),
        ) && caps.supports_rate(r)
        {
            req.sample_rate_hz = r;
        }
        if force || self.devices.device(Direction::Output) != Some(req.device.name.as_str()) {
            self.device_event(DeviceEvent::Configured {
                dir: Direction::Output,
                device: Some(req.device.name.clone()),
            });
        }
        let doc_rate = doc_rate.unwrap_or(req.sample_rate_hz);
        match self.build_output(&req, doc_rate) {
            Ok(out) => {
                self.output = Some(out);
                self.monitor_reset();
                self.device_event(DeviceEvent::Opened {
                    dir: Direction::Output,
                    fallback,
                });
            }
            Err(_) => self.device_event(DeviceEvent::OpenFailed {
                dir: Direction::Output,
            }),
        }
        self.update_monitor();
    }

    fn build_output(
        &mut self,
        req: &StreamRequest,
        doc_rate: u32,
    ) -> Result<OutputStream, BackendError> {
        let rate = req.sample_rate_hz;
        let cfg = ActivateConfig {
            sample_rate: f64::from(rate),
            max_block: MAX_BLOCK,
            mode: ProcessMode::Realtime,
            layout: ChannelLayout::MONO,
        };
        let options = RackOptions::default();
        let (rack, live) =
            match RackHost::new(self.registry.clone(), cfg, options, &self.rack_model) {
                Ok(x) => x,
                Err(_) => {
                    let empty = RackModel { slots: Vec::new() };
                    RackHost::new(self.registry.clone(), cfg, options, &empty)
                        .map_err(|e| BackendError::Backend(e.to_string()))?
                }
            };
        let (pkt_tx, pkt_rx) = RingBuffer::new(PLAYBACK_RING_PACKETS);
        let (cmd_tx, cmd_rx) = RingBuffer::new(AUDIO_CMD_CAPACITY);
        let (ev_tx, ev_rx) = RingBuffer::new(RT_EVENT_CAPACITY);
        let (mon_tx, mon_rx) = RingBuffer::new(MONITOR_RING_FRAMES);
        let (an_tx, an_rx) = RingBuffer::new(ANALYZER_RING_FRAMES);
        let counters = Arc::new(RtCounters::default());
        let link = Arc::new(MonitorLink::default());
        let slot: PartsSlot = Arc::new(Mutex::new(None));
        let parts = OutputParts {
            live,
            packets: pkt_rx,
            cmds: cmd_rx,
            events: ev_tx,
            counters: counters.clone(),
            monitor: MonitorOut::new(mon_rx, link.clone(), counters.clone(), rate),
            analyzer_tap: AnalyzerTap::new(
                an_tx,
                self.analyzer.gate(),
                self.analyzer.dropped_counter(),
            ),
            // T-304 (SPEC-022 §4.7): built here, on the control thread, never on the RT thread.
            calib_sweep: vox_dsp::calibration::sweep(rate),
        };
        let cb = OutputCb::new(parts, slot.clone(), rate, doc_rate);
        match self.backend.open_output(req, Box::new(cb)) {
            Ok(handle) => {
                let stall = StallDetector::new(handle.info(), self.now());
                let buffer = handle.info().buffer;
                let nominal_frames = handle.info().nominal_frames;
                let playable = reader::resampler_for(doc_rate, rate).is_ok();
                if !playable {
                    self.notify(DeviceNotice::ResampleUnavailable {
                        device: req.device.name.clone(),
                        doc_rate_hz: doc_rate,
                        device_rate_hz: rate,
                    });
                }
                self.reader_send(ReaderCmd::Attach {
                    producer: pkt_tx,
                    dev_rate_hz: rate,
                });
                self.last_underruns = 0;
                // T-208: every successful (re)open is a fresh ring — exactly the reopen/rate-
                // change moments SPEC-007 §4.8.6 wants treated as an analyzer reset.
                self.analyzer.attach_ring(an_rx, rate);
                self.monitor_gen = self.monitor_gen.wrapping_add(1);
                self.monitor_tx = Some(MonitorTx {
                    generation: self.monitor_gen,
                    producer: mon_tx,
                    link,
                    pushed: 0,
                });
                self.out_lat.reset(nominal_frames);
                Ok(OutputStream {
                    handle,
                    slot,
                    rack,
                    cmds: cmd_tx,
                    events: ev_rx,
                    counters,
                    stall,
                    device: req.device.name.clone(),
                    rate_hz: rate,
                    doc_rate_hz: doc_rate,
                    buffer,
                    monitor_gen: self.monitor_gen,
                    playable,
                })
            }
            Err(e) => {
                // The backend dropped the callback: its parts are in the slot.
                if let Some(p) = take_parts(&slot) {
                    rack.teardown(p.live);
                }
                Err(e)
            }
        }
    }

    /// Drops the stream (the backend then drops the callback, which hands its parts back) and
    /// tears the rack down here, on the control thread. Playback stops first (Pause semantics at
    /// the heard position; no stop fade will report back) on every close path.
    fn close_output(&mut self) {
        let Some(out) = self.output.take() else {
            return;
        };
        if self.transport.playing() {
            let heard = self.heard_now(self.now());
            let _ = self.transport.pause(heard, false);
            self.reader_send(ReaderCmd::Stop);
        }
        let OutputStream {
            handle, slot, rack, ..
        } = out;
        drop(handle);
        match take_parts(&slot) {
            Some(p) => rack.teardown(p.live),
            None => drop(rack),
        }
        self.reader_send(ReaderCmd::Detach);
        self.transport.stream_closed();
        self.monitor_reset();
        self.analyzer.detach_ring();
    }

    // --- Input (S1-04) ---------------------------------------------------------------------

    /// Resolves the input device from the prefs without opening it (status, channel, notices).
    fn configure_input(&mut self, force: bool) {
        let Some(host) = self.host else { return };
        let res = resolve_devices(&self.prefs, host, &self.list.snapshot);
        if res.caps_pending {
            self.input_pending = true;
            return;
        }
        self.input_pending = false;
        if force {
            for n in res.notices.iter().filter(|n| is_input_notice(n)) {
                self.notify(n.clone());
            }
        }
        if let Some(i) = &res.input {
            self.in_channel.set(i.channel);
        }
        let name = res.input.map(|i| i.device.name);
        if force || name.as_deref() != self.devices.device(Direction::Input) {
            if self.input.is_some() {
                self.close_input(true);
            }
            self.in_device_id = name
                .as_ref()
                .and_then(|n| self.list.snapshot.get(n))
                .map(|d| d.id.clone());
            self.device_event(DeviceEvent::Configured {
                dir: Direction::Input,
                device: name,
            });
        }
    }

    /// Opens the input stream on the configured device (all channels, deinterleaved), at the
    /// preferred record rate when the device supports it, else at its default rate.
    fn open_input(&mut self) {
        if self.input.is_some() {
            return;
        }
        let Some(host) = self.host else { return };
        let res = resolve_devices(&self.prefs, host, &self.list.snapshot);
        if res.caps_pending {
            self.input_pending = true;
            return;
        }
        self.input_pending = false;
        let Some(applied) = res.input.clone() else {
            if self.devices.device(Direction::Input).is_some() {
                self.device_event(DeviceEvent::Configured {
                    dir: Direction::Input,
                    device: None,
                });
            }
            return;
        };
        let mut name = applied.device.name.clone();
        if let Some(id) = &self.in_device_id
            && let Some(d) = self
                .list
                .snapshot
                .devices_for(Direction::Input)
                .find(|d| &d.id == id)
        {
            name = d.name.clone();
        }
        let info = self.list.snapshot.get(&name).cloned();
        self.in_device_id = info.as_ref().map(|d| d.id.clone());
        let rate = match info.as_ref().and_then(|d| d.caps(Direction::Input)) {
            Some(c) if !c.supports_rate(self.record_rate_hz) => c.default_rate_hz,
            _ => self.record_rate_hz,
        };
        let Some(mut req) = res.input_request(rate) else {
            return;
        };
        req.device.name = name.clone();
        self.in_channel.set(applied.channel);
        if self.devices.device(Direction::Input) != Some(name.as_str()) {
            self.device_event(DeviceEvent::Configured {
                dir: Direction::Input,
                device: Some(name),
            });
        }
        match self.build_input(&req) {
            Ok(inp) => {
                self.in_meter.reset(inp.rate_hz);
                self.in_lat.reset(inp.nominal_frames);
                self.input = Some(inp);
                self.device_event(DeviceEvent::Opened {
                    dir: Direction::Input,
                    fallback: false,
                });
            }
            Err(_) => self.device_event(DeviceEvent::OpenFailed {
                dir: Direction::Input,
            }),
        }
        self.monitor_reset();
        self.update_monitor();
    }

    fn build_input(&mut self, req: &StreamRequest) -> Result<InputStream, BackendError> {
        let rate = req.sample_rate_hz;
        let (mut cmd_tx, cmd_rx) = RingBuffer::new(INPUT_CMD_CAPACITY);
        let (ev_tx, ev_rx) = RingBuffer::new(INPUT_EVENT_CAPACITY);
        let (cap_tx, cap_rx) = RingBuffer::new(rate as usize * CAPTURE_RING_SECONDS);
        let (gap_tx, gap_rx) = RingBuffer::new(GAP_EVENT_CAPACITY);
        let shared = Arc::new(InputShared::default());
        let slot: MonitorSlot = Arc::new(Mutex::new(None));
        let monitor = self.monitor_tx.take();
        let monitor_gen = monitor.as_ref().map(|m| m.generation);
        let side = InputSide::new(InputSideParts {
            cmds: cmd_rx,
            events: ev_tx,
            capture: cap_tx,
            monitor,
            shared: shared.clone(),
            slot: slot.clone(),
            rate_hz: rate,
            gap_events: gap_tx,
        });
        let cb = input_callback(self.in_channel.clone(), side);
        match self.backend.open_input(req, Box::new(cb)) {
            Ok(handle) => {
                if !handle.info().timestamps_reliable {
                    // H-23 (SPEC-002 §4.3): sent before any capture starts, so the callback has
                    // it applied well ahead of the first block it ever processes.
                    let _ = cmd_tx.push(InputCmd::TimestampsReliable(false));
                }
                Ok(InputStream {
                    stall: StallDetector::new(handle.info(), self.now()),
                    nominal_frames: handle.info().nominal_frames,
                    handle,
                    cmds: cmd_tx,
                    events: ev_rx,
                    shared,
                    slot,
                    capture_rx: Some(cap_rx),
                    capture_home: Arc::new(Mutex::new(None)),
                    gap_rx: Some(gap_rx),
                    gap_home: Arc::new(Mutex::new(None)),
                    rate_hz: rate,
                    monitor_gen,
                })
            }
            Err(e) => {
                // The backend dropped the callback: its monitor producer is in the slot.
                self.repark_monitor(&slot);
                Err(e)
            }
        }
    }

    /// Takes a dropped input callback's monitor producer back if it still feeds the current
    /// output's ring.
    fn repark_monitor(&mut self, slot: &MonitorSlot) {
        if let Some(m) = take_monitor(slot)
            && self
                .output
                .as_ref()
                .is_some_and(|o| o.monitor_gen == m.generation)
        {
            self.monitor_tx = Some(m);
        }
    }

    /// Drops the input stream. `deliberate`: a healthy stream closed on purpose (disarm, other
    /// device) — not a loss.
    fn close_input(&mut self, deliberate: bool) {
        let Some(inp) = self.input.take() else {
            return;
        };
        let InputStream { handle, slot, .. } = inp;
        drop(handle);
        self.repark_monitor(&slot);
        if deliberate {
            self.device_event(DeviceEvent::Closed {
                dir: Direction::Input,
            });
        }
        self.in_meter.reset(0);
        self.monitor_reset();
    }

    fn drain_input(&mut self) {
        let events: Vec<InputEvent> = {
            let Some(inp) = self.input.as_mut() else {
                return;
            };
            std::iter::from_fn(|| inp.events.pop().ok()).collect()
        };
        let mut period_grew = false;
        for e in events {
            match e {
                InputEvent::Block {
                    frames,
                    latency_ns,
                    peak,
                    sum_sq,
                    clipped,
                    capturing,
                    captured,
                    end_ns,
                    take_frames,
                } => {
                    period_grew |= self.in_lat.observe(frames, latency_ns);
                    self.in_meter.add(frames, peak, sum_sq, clipped);
                    if capturing && let Some(rec) = self.recording.as_mut() {
                        rec.captured = captured;
                        rec.anchor_ns = end_ns;
                        // T-304 (SPEC-022 §4.3): take position ↔ capture time for `k_start`.
                        if let Some(op) = rec.op.as_mut() {
                            push_capture_block(
                                &mut op.blocks,
                                take_frames,
                                frames,
                                end_ns,
                                op.in_rate,
                                Some(OP_BLOCK_HISTORY_NS),
                            );
                        }
                    }
                    if capturing
                        && let Some(c) = self.calib.as_mut()
                        && c.result.is_none()
                    {
                        push_capture_block(
                            &mut c.blocks,
                            take_frames,
                            frames,
                            end_ns,
                            c.rate_hz,
                            None,
                        );
                    }
                }
                InputEvent::CaptureEnded { samples } => {
                    if let Some(rec) = self.recording.as_mut() {
                        rec.captured = samples;
                    }
                }
            }
        }
        if period_grew {
            // T-107: a longer input period raises F* (ADR-002 §6: max observed period).
            self.update_monitor();
        }
    }

    // --- Recording (S1-04) -----------------------------------------------------------------

    pub(crate) fn record_state(&self) -> RecordState {
        RecordState {
            input_device: self.devices.device(Direction::Input).map(str::to_owned),
            input_channel: self.in_channel.get(),
            input_status: self.devices.status(Direction::Input),
            armed: self.armed,
            input_open: self.input.is_some(),
            input_rate_hz: self.input.as_ref().map(|i| i.rate_hz),
            recording: self.capturing(),
            finishing: self.recording.as_ref().is_some_and(|r| r.stop_at.is_some()),
            monitor: self.monitor_mode,
            monitoring: self.monitor_active,
            monitor_latency_us: self.monitor_latency_us,
            monitor_underruns: self
                .output
                .as_ref()
                .map_or(0, |o| o.counters.mon_underruns.load(Ordering::Relaxed)),
            monitor_overruns: self
                .output
                .as_ref()
                .map_or(0, |o| o.counters.mon_overruns.load(Ordering::Relaxed)),
            dropout_count: self
                .recording
                .as_ref()
                .map_or(0, |r| r.shared.dropout_events.load(Ordering::Relaxed)),
            disk_remaining_s: self.disk_remaining_seconds(),
        }
    }

    /// SPEC-002 §2.5, AC-13: `(free − 512 MiB) / (8 × rate)`, at the open input's rate while
    /// armed/recording, else the configured default rate a new recording would use next.
    /// Floored to whole seconds (settles at ~1 Hz instead of jittering every tick) — `None`
    /// before the first successful [`Self::poll_disk`].
    fn disk_remaining_seconds(&self) -> Option<u64> {
        let free = self.disk_free_bytes?;
        let rate = u64::from(
            self.input
                .as_ref()
                .map_or(self.record_rate_hz, |i| i.rate_hz)
                .max(1),
        );
        let usable = free.saturating_sub(DISK_FLOOR_BYTES);
        Some(usable / (RECORD_BYTES_PER_SAMPLE * rate))
    }

    /// SPEC-002 §2.5: refreshes [`Self::disk_free_bytes`] about once a second — never more often,
    /// since the provider may hit the file system (CLAUDE.md: no I/O on the RT input thread, but
    /// this runs on the control thread, so an occasional blocking call is acceptable, unlike every
    /// 16.7 ms tick).
    fn poll_disk(&mut self, now: u64) {
        if self
            .disk_checked_ns
            .is_some_and(|t| now.saturating_sub(t) < DISK_CHECK_INTERVAL_NS)
        {
            return;
        }
        self.disk_checked_ns = Some(now);
        self.disk_free_bytes = self.disk_space.free_bytes(&self.record_volume).ok();
    }

    /// SPEC-002 §2.5, AC-13: below the hard floor while actively recording, flag it like a write
    /// error ([`InputShared::disk_full`] mirrors `writer_failed`) — `service_recording` turns it
    /// into a graceful [`StopReason::DiskFull`] stop that keeps the take.
    fn check_disk_floor(&mut self) {
        let Some(rec) = self.recording.as_ref() else {
            return;
        };
        if rec.stop_at.is_some() {
            return;
        }
        if self
            .disk_free_bytes
            .is_some_and(|free| free < DISK_FLOOR_BYTES)
        {
            rec.shared.disk_full.store(true, Ordering::Release);
        }
    }

    /// H-07: a snapshot of the take being captured's running peaks (`None`: no take is being
    /// captured). Reads a lock only the capture-writer thread and this call ever take — never the
    /// RT input callback.
    pub(crate) fn live_take_peaks(&self, start_bucket: u32, max: u32) -> Option<LiveTakePeaks> {
        let rec = self.recording.as_ref()?;
        let (len_samples, spb, buckets) = rec
            .peaks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot(start_bucket, max);
        Some(LiveTakePeaks {
            sample_rate_hz: rec.doc_rate_hz,
            len_samples,
            // H-10 item 6: `spb` doubles as `LivePeaks` decimates a multi-hour take, so it must
            // come from the same snapshot as `buckets` — never the fixed `LIVE_PEAKS_SPB`, which
            // is only the *starting* resolution.
            spb,
            buckets,
        })
    }

    fn emit_record_if_changed(&mut self) {
        let state = self.record_state();
        if self.last_record.as_ref() != Some(&state) {
            self.last_record = Some(state.clone());
            (self.events)(EngineEvent::Record(state));
        }
    }

    /// Arms/disarms the input (opens/closes the input stream). Locked on while recording.
    pub(crate) fn set_armed(&mut self, armed: bool) -> RecordState {
        if armed || self.recording.is_none() {
            self.armed = armed;
            if armed {
                self.open_input();
            } else {
                self.close_input(true);
            }
            self.update_monitor();
            self.emit_record_if_changed();
        }
        self.record_state()
    }

    pub(crate) fn set_monitor_mode(&mut self, mode: MonitorMode) -> RecordState {
        self.monitor_mode = mode;
        self.relink_monitor();
        self.update_monitor();
        self.latency_dirty = true;
        self.update_latency_readout(self.now());
        self.emit_record_if_changed();
        self.record_state()
    }

    /// The rate the input opens at next time (the default recording format, SPEC-002 §2.2).
    pub(crate) fn set_record_rate(&mut self, rate_hz: u32) {
        self.record_rate_hz = rate_hz.max(1);
    }

    /// Starts capturing a take into `capture` (see [`crate::record`]). Opens (arms) the input if
    /// needed; the take then begins with its first captured sample, otherwise with the first
    /// sample captured at or after now (SPEC-002 §2.2 "Start").
    pub(crate) fn record_start(
        &mut self,
        doc_rate_hz: u32,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        self.start_capture(doc_rate_hz, capture, done, None)
    }

    /// A calibration run is in progress (a finished one waiting to be polled doesn't count).
    fn calibrating(&self) -> bool {
        self.calib.as_ref().is_some_and(|c| c.result.is_none())
    }

    /// [`Self::record_start`]; `op`: the record operation the take belongs to (T-304: its writer
    /// finishes only once the operation is sealed).
    fn start_capture(
        &mut self,
        doc_rate_hz: u32,
        capture: TakeCapture,
        done: RecordDone,
        op: Option<Arc<OpShared>>,
    ) -> Result<RecordState, RecordError> {
        if self.recording.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        if self.calibrating() {
            return Err(RecordError::Calibrating);
        }
        if self.prefs.input_device.is_none() {
            return Err(RecordError::NoInputDevice);
        }
        let was_open = self.input.is_some();
        if !was_open {
            self.armed = true;
            self.open_input();
        }
        let now = self.now();
        let Some(inp) = self.input.as_mut() else {
            self.emit_record_if_changed();
            return Err(RecordError::InputNotOpen);
        };
        let in_rate_hz = inp.rate_hz;
        // H-06 (SPEC-002 §2.2): the input can run at a different rate than the document — the
        // capture-writer resamples, never the RT input callback. A resampler that fails to build
        // (an unsupported rate combination) is the only remaining rejection.
        let resampler = if in_rate_hz != doc_rate_hz {
            match CaptureResampler::new(in_rate_hz, doc_rate_hz) {
                Ok(r) => Some(r),
                Err(_) => {
                    return Err(RecordError::RateMismatch {
                        input_hz: in_rate_hz,
                        doc_hz: doc_rate_hz,
                    });
                }
            }
        } else {
            None
        };
        let Some(mut rx) = inp
            .capture_rx
            .take()
            .or_else(|| take_home(&inp.capture_home))
        else {
            return Err(RecordError::AlreadyRecording);
        };
        // Drop anything a previous (forced) take left behind.
        let stale = rx.slots();
        if stale > 0
            && let Ok(chunk) = rx.read_chunk(stale)
        {
            chunk.commit_all();
        }
        // H-10 item 4 / H-11 item 5: taken for every take, resampled or not — `CaptureWriter`
        // splices the silence fill through the resampler too when one is active, so the marked
        // position/length still land in document-rate samples (`CaptureWriter` docs).
        let mut gap_rx = inp.gap_rx.take().or_else(|| take_gap_home(&inp.gap_home));
        if let Some(rx) = gap_rx.as_mut() {
            while rx.pop().is_ok() {} // drop stale events from a previous take
        }
        inp.shared.reset_take();
        let shared = inp.shared.clone();
        let peaks: LivePeaksHandle = Arc::new(Mutex::new(LivePeaks::default()));
        let writer = CaptureWriter::new(
            rx,
            capture,
            shared.clone(),
            inp.capture_home.clone(),
            doc_rate_hz,
            resampler,
            done,
            peaks.clone(),
            gap_rx,
            inp.gap_home.clone(),
            op,
        );
        let link = if self.threaded {
            match capture::spawn(writer) {
                Ok(join) => WriterLink::Thread(join),
                Err(e) => {
                    // The capture ring went down with the writer: rebuild the input stream.
                    self.close_input(false);
                    self.open_input();
                    return Err(RecordError::Writer(e.to_string()));
                }
            }
        } else {
            WriterLink::Inline {
                writer: Some(Box::new(writer)),
                ticks: 0,
            }
        };
        let start_ns = if was_open { now } else { 0 };
        if let Some(inp) = self.input.as_mut()
            && inp.cmds.push(InputCmd::StartCapture { start_ns }).is_err()
        {
            shared.force_finish.store(true, Ordering::Release);
        }
        self.recording = Some(Recording {
            link,
            shared,
            rate_hz: in_rate_hz,
            doc_rate_hz,
            captured: 0,
            anchor_ns: now,
            stop_at: None,
            peaks,
            op: None,
        });
        self.engine_stop();
        self.update_monitor();
        self.emit_record_if_changed();
        Ok(self.record_state())
    }

    /// Stops the take (User): the input callback ends it at the capture time of now.
    pub(crate) fn record_stop(&mut self) -> RecordState {
        self.stop_recording(StopReason::User);
        self.update_monitor();
        self.emit_record_if_changed();
        self.record_state()
    }

    fn stop_recording(&mut self, reason: StopReason) {
        let now = self.now();
        if self.recording.as_ref().is_some_and(|r| r.op.is_some()) {
            // T-304 (SPEC-022 §2.10): what a stop means depends on the operation's phase.
            let trigger = if reason == StopReason::User {
                EndTrigger::User
            } else {
                EndTrigger::Failure(reason)
            };
            self.op_end(trigger, now);
            return;
        }
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stop_at.is_some() && reason == StopReason::User {
            return;
        }
        rec.stop_at.get_or_insert(now);
        let code = match reason {
            StopReason::User => stop_code::USER,
            StopReason::InputLost => stop_code::INPUT_LOST,
            StopReason::Shutdown => stop_code::SHUTDOWN,
            StopReason::Overflow => stop_code::OVERFLOW,
            StopReason::WriteError => stop_code::WRITE_ERROR,
            StopReason::DiskFull => stop_code::DISK_FULL,
        };
        // The first reason sticks: a take the input callback already ended (its Stop time was
        // reached, or the capture ring overflowed) or that a write error stopped keeps it.
        if !rec.shared.capture_done.load(Ordering::Acquire) {
            let _ = rec.shared.stop_reason.compare_exchange(
                stop_code::USER,
                code,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        // User, write-error and disk-full stops end the take in the input callback, so it stops
        // pushing (nothing stale reaches the next take); the others have no live input to ask.
        let sent = matches!(
            reason,
            StopReason::User | StopReason::WriteError | StopReason::DiskFull
        ) && self.input.as_mut().is_some_and(|i| {
            Arc::ptr_eq(&i.shared, &rec.shared)
                && i.cmds.push(InputCmd::StopCapture { stop_ns: now }).is_ok()
        });
        if !sent {
            // No input to end the take at its stop time: finish with what was captured.
            rec.shared.force_finish.store(true, Ordering::Release);
        }
    }

    /// Stops a take that ended on its own (capture-ring overflow) or whose writer failed, drains
    /// an inline writer, forces a stop the input never completes, and reaps a finished writer.
    fn service_recording(&mut self, now: u64) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stop_at.is_none() {
            if rec.shared.capture_done.load(Ordering::Acquire) {
                // H-05: the input callback ended the take (overflow, SPEC-002 §2.4): finishing.
                rec.stop_at = Some(now);
            } else if rec.shared.writer_failed.load(Ordering::Acquire) {
                // H-05: nothing more reaches the take — stop now instead of recording into the
                // void until the user presses Stop (SPEC-002 §2.5, ADR-004 §7.4).
                self.stop_recording(StopReason::WriteError);
            } else if rec.shared.disk_full.load(Ordering::Acquire) {
                // H-11: `check_disk_floor` flagged the hard floor — stop now and keep the take
                // (SPEC-002 §2.5, AC-13), the same shape as a write error.
                self.stop_recording(StopReason::DiskFull);
            }
        }
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if let Some(t) = rec.stop_at
            && now.saturating_sub(t) >= FORCE_FINISH_NS
        {
            rec.shared.force_finish.store(true, Ordering::Release);
        }
        let finished = match &mut rec.link {
            WriterLink::Thread(join) => join.is_finished(),
            WriterLink::Inline { writer, ticks } => {
                *ticks = ticks.wrapping_add(1);
                let complete = match writer.as_mut() {
                    Some(w) => {
                        let complete = w.drain();
                        if !complete && (*ticks).is_multiple_of(INLINE_SYNC_TICKS) {
                            w.sync();
                        }
                        complete
                    }
                    None => true,
                };
                if complete && let Some(w) = writer.take() {
                    (*w).finish();
                }
                complete
            }
        };
        if finished {
            if let Some(Recording {
                link: WriterLink::Thread(join),
                ..
            }) = self.recording.take()
            {
                let _ = join.join();
            }
            self.update_monitor();
        }
    }

    // --- Record operations (T-304, SPEC-022) -----------------------------------------------

    fn stop_code_of(reason: StopReason) -> u8 {
        match reason {
            StopReason::User => stop_code::USER,
            StopReason::InputLost => stop_code::INPUT_LOST,
            StopReason::Shutdown => stop_code::SHUTDOWN,
            StopReason::Overflow => stop_code::OVERFLOW,
            StopReason::WriteError => stop_code::WRITE_ERROR,
            StopReason::DiskFull => stop_code::DISK_FULL,
        }
    }

    fn stop_reason_of(code: u8) -> StopReason {
        match code {
            stop_code::INPUT_LOST => StopReason::InputLost,
            stop_code::SHUTDOWN => StopReason::Shutdown,
            stop_code::OVERFLOW => StopReason::Overflow,
            stop_code::WRITE_ERROR => StopReason::WriteError,
            stop_code::DISK_FULL => StopReason::DiskFull,
            _ => StopReason::User,
        }
    }

    /// SPEC-022 §2.2: resolves a Record press on a document with audio. A playing transport
    /// stops first (engine-initiated, Pause semantics: the cursor becomes the heard position).
    pub(crate) fn record_prepare(
        &mut self,
        selection: Option<(u64, u64)>,
        prefs: RecordPrefs,
    ) -> Result<RecordPlan, RecordError> {
        if self.recording.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        if self.calibrating() {
            return Err(RecordError::Calibrating);
        }
        if self.prefs.input_device.is_none() {
            return Err(RecordError::NoInputDevice);
        }
        self.engine_stop();
        self.emit_state_if_changed();
        let has_output = self.can_play();
        resolve_record(
            self.transport.len(),
            self.doc_rate(),
            selection,
            self.transport.playhead(),
            has_output,
            &prefs,
        )
    }

    /// SPEC-022 §2.4–§2.7, §4.4: starts a resolved record operation into `capture`. Aligned
    /// operations start a playback run (pre-roll with silence padding, the record range muted,
    /// post-roll for a punch); free starts capture right away with `k_start = 0`.
    pub(crate) fn record_start_op(
        &mut self,
        plan: RecordPlan,
        capture: TakeCapture,
        done: RecordDone,
    ) -> Result<RecordState, RecordError> {
        if plan.kind == RecordOpKind::New {
            return self.record_start(plan.doc_rate_hz, capture, done);
        }
        if self.recording.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        let len = self.transport.len();
        if plan.at_samples > len
            || plan
                .end_samples
                .is_some_and(|e| e > len || e <= plan.at_samples)
            || plan.doc_rate_hz != self.doc_rate()
        {
            return Err(RecordError::InvalidPosition);
        }
        let mut plan = plan;
        if plan.aligned && !self.can_play() {
            if plan.kind == RecordOpKind::Punch {
                return Err(RecordError::PunchNeedsOutput);
            }
            // Cursor recordings work with only an input device, with a free start (D-020).
            plan.aligned = false;
            plan.preroll_samples = 0;
            plan.offset_ns = 0;
            plan.hear_original = false;
        }
        self.engine_stop();
        let take = capture.id().0;
        let shared = Arc::new(OpShared::new());
        if !plan.aligned {
            shared.k_start.store(0, Ordering::Release);
        }
        self.start_capture(plan.doc_rate_hz, capture, done, Some(shared.clone()))?;
        let now = self.now();
        let out_gen = self.monitor_gen;
        let mut run = None;
        if plan.aligned {
            let fade = (LISTEN_FADE_MS / 1000.0 * f64::from(plan.doc_rate_hz))
                .round()
                .max(1.0) as u64;
            let (mute_end, end) = match plan.end_samples {
                Some(e) if plan.kind == RecordOpKind::Punch => {
                    (Some(e), Some((e + plan.postroll_samples).min(len).max(e)))
                }
                _ => (None, None),
            };
            let spec = RunSpec {
                offset: plan.preroll_samples.saturating_sub(plan.at_samples),
                at: plan.at_samples,
                mute_end,
                doc_len: len,
                hear_original: plan.hear_original,
                end,
                fade,
            };
            let epoch = self.transport.next_run_epoch();
            let pos = spec.start_v(plan.preroll_samples);
            self.anchor = None;
            self.reader_send(ReaderCmd::StartRun {
                epoch,
                pos,
                run: spec,
            });
            self.audio_cmd(AudioCmd::Play {
                epoch,
                pos,
                reset: true,
            });
            run = Some((epoch, spec));
        }
        let Some(rec) = self.recording.as_mut() else {
            return Ok(self.record_state());
        };
        let (phase, doc_pos) = if plan.aligned {
            (
                RecordPhase::PreRoll,
                plan.at_samples.saturating_sub(plan.preroll_samples),
            )
        } else {
            (RecordPhase::Recording, plan.at_samples)
        };
        let op = OpState {
            plan,
            take,
            phase,
            shared,
            run,
            out_gen,
            run_ended: false,
            output_lost: false,
            t_at: None,
            anchor: None,
            blocks: VecDeque::new(),
            in_rate: rec.rate_hz,
            k_start: (!plan.aligned).then_some(0),
            end: None,
        };
        let info = op.phase_info(phase, doc_pos, now);
        rec.op = Some(op);
        (self.events)(EngineEvent::RecordPhase(info));
        self.update_monitor();
        self.emit_record_if_changed();
        Ok(self.record_state())
    }

    /// SPEC-022 §2.10: ends a record operation — decides by phase whether it is cancelled or
    /// where its window ends, stops its playback run, stops the capture just past the window's
    /// last sample (or forces it when the input is gone), then seals it when possible.
    fn op_end(&mut self, trigger: EndTrigger, now: u64) {
        let forced = matches!(
            trigger,
            EndTrigger::Failure(
                StopReason::InputLost | StopReason::Shutdown | StopReason::Overflow
            )
        );
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        let Some(op) = rec.op.as_mut() else {
            return;
        };
        if op.end.is_some() {
            if forced {
                rec.shared.force_finish.store(true, Ordering::Release);
                self.op_try_seal(now, true);
            }
            return;
        }
        let at = op.plan.at_samples;
        let e = op.plan.end_samples;
        let end = match (op.phase, trigger) {
            (RecordPhase::PreRoll, EndTrigger::User) => OpEnd::Cancel(CancelReason::User),
            (RecordPhase::PreRoll, EndTrigger::Failure(StopReason::InputLost)) => {
                OpEnd::Cancel(CancelReason::InputLost)
            }
            (RecordPhase::PreRoll, EndTrigger::Failure(_)) => OpEnd::Cancel(CancelReason::Aborted),
            (RecordPhase::PreRoll, EndTrigger::OutputLost | EndTrigger::RunOver) => {
                OpEnd::Cancel(CancelReason::OutputLost)
            }
            (RecordPhase::Recording, EndTrigger::User) if op.plan.aligned => {
                // `p`: the heard position at the Stop command, clamped to `[at, E]`.
                let q = op.heard_q(now).unwrap_or(i128::from(at));
                let q = e.map_or(q, |e| q.min(i128::from(e)));
                if q <= i128::from(at) {
                    OpEnd::Cancel(CancelReason::User)
                } else {
                    OpEnd::At(Some(q as u64))
                }
            }
            (RecordPhase::Recording, EndTrigger::User | EndTrigger::Failure(_)) => OpEnd::At(None),
            // D-017: the take is unaffected; a punch then ends at `E` (RunOver).
            (RecordPhase::Recording, EndTrigger::OutputLost) => return,
            (RecordPhase::Recording, EndTrigger::RunOver) => OpEnd::At(e),
            (RecordPhase::PostRoll | RecordPhase::Committing, _) => OpEnd::At(e),
        };
        op.end = Some(end);
        let stop_ns = match end {
            OpEnd::At(Some(q)) => op
                .heard_time(q)
                .map(|t| {
                    let t = t + i128::from(op.plan.offset_ns) + i128::from(OP_STOP_MARGIN_NS);
                    u64::try_from(t.max(i128::from(now))).unwrap_or(now)
                })
                .unwrap_or(now),
            _ => now,
        };
        let stop_run = op.run.is_some() && !op.run_ended && !op.output_lost;
        rec.stop_at.get_or_insert(stop_ns);
        let code = match trigger {
            EndTrigger::Failure(reason) => Self::stop_code_of(reason),
            _ => stop_code::USER,
        };
        if !rec.shared.capture_done.load(Ordering::Acquire) {
            let _ = rec.shared.stop_reason.compare_exchange(
                stop_code::USER,
                code,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        let shared = rec.shared.clone();
        if stop_run {
            self.audio_cmd(AudioCmd::Stop);
            self.reader_send(ReaderCmd::Stop);
        }
        let sent = !forced
            && self.input.as_mut().is_some_and(|i| {
                Arc::ptr_eq(&i.shared, &shared)
                    && i.cmds.push(InputCmd::StopCapture { stop_ns }).is_ok()
            });
        if !sent {
            shared.force_finish.store(true, Ordering::Release);
        }
        self.op_try_seal(now, forced);
        self.update_monitor();
        self.emit_record_if_changed();
    }

    /// Seals a decided operation once its window start is known, so the capture-writer may finish
    /// the take with the window (T-304). `force`: nothing more will arrive — an unknown window
    /// start cancels.
    fn op_try_seal(&mut self, now: u64, force: bool) {
        let mut events = Vec::new();
        {
            let Some(rec) = self.recording.as_mut() else {
                return;
            };
            let complete = force
                || rec.shared.capture_done.load(Ordering::Acquire)
                || rec.shared.force_finish.load(Ordering::Acquire);
            let Some(op) = rec.op.as_mut() else {
                return;
            };
            let Some(end) = op.end else {
                return;
            };
            if op.shared.is_sealed() {
                return;
            }
            if let Some(k) = op.resolve_k_start() {
                op.shared.k_start.store(k, Ordering::Release);
                events.push(EngineEvent::RecordWindow {
                    take: op.take,
                    k_start: k,
                });
            }
            let at = op.plan.at_samples;
            let result = match end {
                OpEnd::Cancel(reason) => Some(OpResult {
                    plan: op.plan,
                    window: None,
                    cancelled: Some(reason),
                }),
                OpEnd::At(q) => match op.k_start {
                    Some(k) => Some(OpResult {
                        plan: op.plan,
                        window: Some((k, q.map_or(u64::MAX, |q| k + (q - at)))),
                        cancelled: None,
                    }),
                    None if complete => Some(OpResult {
                        plan: op.plan,
                        window: None,
                        cancelled: Some(CancelReason::Aborted),
                    }),
                    None => None,
                },
            };
            let Some(result) = result else {
                return;
            };
            op.shared.seal(result);
            op.phase = RecordPhase::Committing;
            if result.cancelled.is_none() {
                let doc_pos = match end {
                    OpEnd::At(Some(q)) => q,
                    _ => op
                        .heard_q(now)
                        .map_or(at, |q| u64::try_from(q.max(i128::from(at))).unwrap_or(at)),
                };
                events.push(EngineEvent::RecordPhase(op.phase_info(
                    RecordPhase::Committing,
                    doc_pos,
                    now,
                )));
            }
        }
        for e in events {
            (self.events)(e);
        }
    }

    /// Each tick (T-304, SPEC-022 §2.7, §2.10, §4.3): resolves the window start, advances the
    /// phases on the heard clock, and ends the operation when its run is over, its output is gone
    /// or its take ended by itself.
    fn service_op(&mut self, now: u64) {
        let out_alive = self.output.is_some();
        let cur_gen = self.monitor_gen;
        let mut events = Vec::new();
        let mut trigger = None;
        {
            let Some(rec) = self.recording.as_mut() else {
                return;
            };
            let capture_done = rec.shared.capture_done.load(Ordering::Acquire);
            let reason = rec.shared.stop_reason.load(Ordering::Relaxed);
            let Some(op) = rec.op.as_mut() else {
                return;
            };
            if let Some(k) = op.resolve_k_start() {
                op.shared.k_start.store(k, Ordering::Release);
                events.push(EngineEvent::RecordWindow {
                    take: op.take,
                    k_start: k,
                });
            }
            if op.run.is_some() && !op.output_lost && (!out_alive || cur_gen != op.out_gen) {
                op.output_lost = true;
            }
            if op.end.is_none() {
                let at = op.plan.at_samples;
                let q_now = op.heard_q(now);
                if let (Some(q), Some(t_at)) = (q_now, op.t_at) {
                    if op.phase == RecordPhase::PreRoll && q >= i128::from(at) {
                        op.phase = RecordPhase::Recording;
                        let t = u64::try_from(t_at.max(0)).unwrap_or(0);
                        events.push(EngineEvent::RecordPhase(op.phase_info(
                            RecordPhase::Recording,
                            at,
                            t,
                        )));
                    }
                    // A-016/H-23: no PostRoll phase when the output was already lost — the run
                    // that would have played it is gone, so the op goes straight from Recording
                    // to Committing once `past_end` ends it below (`EndTrigger::RunOver`).
                    if op.phase == RecordPhase::Recording
                        && op.plan.kind == RecordOpKind::Punch
                        && !op.output_lost
                        && let Some(e) = op.plan.end_samples
                        && q >= i128::from(e)
                    {
                        op.phase = RecordPhase::PostRoll;
                        let t = op
                            .heard_time(e)
                            .map_or(now, |t| u64::try_from(t.max(0)).unwrap_or(now));
                        events.push(EngineEvent::RecordPhase(op.phase_info(
                            RecordPhase::PostRoll,
                            e,
                            t,
                        )));
                    }
                }
                let past_end = op.plan.kind == RecordOpKind::Punch
                    && q_now
                        .zip(op.plan.end_samples)
                        .is_some_and(|(q, e)| q >= i128::from(e));
                trigger = if capture_done {
                    Some(EndTrigger::Failure(Self::stop_reason_of(reason)))
                } else if op.run_ended {
                    Some(EndTrigger::RunOver)
                } else if op.output_lost {
                    match op.phase {
                        RecordPhase::PreRoll => Some(EndTrigger::OutputLost),
                        RecordPhase::PostRoll => Some(EndTrigger::RunOver),
                        RecordPhase::Recording if past_end => Some(EndTrigger::RunOver),
                        _ => None,
                    }
                } else {
                    None
                };
            }
        }
        for e in events {
            (self.events)(e);
        }
        if let Some(t) = trigger {
            self.op_end(t, now);
        }
        self.op_try_seal(now, false);
    }

    // --- Latency calibration (T-304, SPEC-022 §2.14, §4.7) ----------------------------------

    /// Starts a loopback calibration run: monitoring forced Off, the input armed, the capture
    /// ring taken for the run, the 5 sweeps started after the rack.
    pub(crate) fn calibration_start(&mut self, offset_ns: i64) -> Result<(), CalibrationError> {
        if self.recording.is_some() {
            return Err(CalibrationError::Recording);
        }
        if self.calibrating() {
            return Err(CalibrationError::Busy);
        }
        self.calib = None;
        let Some(out_rate) = self.output.as_ref().map(|o| o.rate_hz) else {
            return Err(CalibrationError::NoOutput);
        };
        if self.prefs.input_device.is_none() {
            return Err(CalibrationError::NoInput);
        }
        self.engine_stop();
        let restore_armed = self.armed;
        let restore_monitor = self.monitor_mode;
        if self.input.is_none() {
            self.armed = true;
            self.open_input();
        }
        let now = self.now();
        let prepared = match self.input.as_mut() {
            None => Err(CalibrationError::NoInput),
            Some(inp) if inp.rate_hz != out_rate => Err(CalibrationError::RateMismatch {
                input_hz: inp.rate_hz,
                output_hz: out_rate,
            }),
            Some(inp) => match inp
                .capture_rx
                .take()
                .or_else(|| take_home(&inp.capture_home))
            {
                None => Err(CalibrationError::Busy),
                Some(mut rx) => {
                    let stale = rx.slots();
                    if stale > 0
                        && let Ok(chunk) = rx.read_chunk(stale)
                    {
                        chunk.commit_all();
                    }
                    inp.shared.reset_take();
                    let _ = inp.cmds.push(InputCmd::StartCapture { start_ns: now });
                    Ok((rx, inp.rate_hz, inp.shared.clone()))
                }
            },
        };
        let (rx, rate_hz, in_shared) = match prepared {
            Ok(p) => p,
            Err(e) => {
                self.calib_restore(restore_monitor, restore_armed);
                return Err(e);
            }
        };
        self.monitor_mode = MonitorMode::Off;
        self.update_monitor();
        self.audio_cmd(AudioCmd::Calibrate { start: true });
        self.calib = Some(CalibRun {
            offset_ns,
            rx: Some(rx),
            in_shared,
            recording: Vec::with_capacity(rate_hz as usize * 12),
            blocks: VecDeque::new(),
            rep_times: Vec::new(),
            started_ns: now,
            rate_hz,
            out_gen: self.monitor_gen,
            restore_monitor,
            restore_armed,
            result: None,
        });
        self.emit_record_if_changed();
        Ok(())
    }

    /// Restores monitoring and the armed state after a calibration run.
    fn calib_restore(&mut self, monitor: MonitorMode, armed: bool) {
        self.monitor_mode = monitor;
        if !armed && self.recording.is_none() && self.armed {
            self.armed = false;
            self.close_input(true);
        }
        self.update_monitor();
        self.latency_dirty = true;
        self.emit_record_if_changed();
    }

    /// Ends the run with `result` (kept until polled) and gives the capture ring back.
    fn calib_finish(&mut self, result: Result<CalibrationCapture, CalibrationError>) {
        let Some(mut c) = self.calib.take() else {
            return;
        };
        let now = self.now();
        if let Some(inp) = self.input.as_mut()
            && Arc::ptr_eq(&inp.shared, &c.in_shared)
        {
            let _ = inp.cmds.push(InputCmd::StopCapture { stop_ns: now });
            if let Some(rx) = c.rx.take()
                && inp.capture_rx.is_none()
            {
                inp.capture_rx = Some(rx);
            }
        }
        self.audio_cmd(AudioCmd::Calibrate { start: false });
        let (monitor, armed) = (c.restore_monitor, c.restore_armed);
        c.result = Some(result);
        c.rx = None;
        c.recording = Vec::new();
        c.blocks.clear();
        self.calib = Some(c);
        self.calib_restore(monitor, armed);
    }

    /// Each tick: collects the capture; once the blocks reach past the last repetition's sweep
    /// plus the lag window, maps each repetition's heard time (+ δ) to its recording index.
    fn service_calibration(&mut self, now: u64) {
        let out_alive = self.output.is_some();
        let cur_gen = self.monitor_gen;
        let in_alive = self.input.is_some();
        let Some(c) = self.calib.as_mut() else {
            return;
        };
        if c.result.is_some() {
            return;
        }
        if let Some(rx) = c.rx.as_mut() {
            let n = rx.slots();
            if n > 0
                && let Ok(chunk) = rx.read_chunk(n)
            {
                let (a, b) = chunk.as_slices();
                c.recording.extend_from_slice(a);
                c.recording.extend_from_slice(b);
                chunk.commit_all();
            }
        }
        let failed = !out_alive
            || cur_gen != c.out_gen
            || !in_alive
            || now.saturating_sub(c.started_ns) > CALIB_TIMEOUT_NS;
        let mut done = None;
        if failed {
            done = Some(Err(CalibrationError::Interrupted));
        } else if c.rep_times.len() >= vox_dsp::calibration::REPS {
            let tail_ns = (vox_dsp::calibration::SWEEP_SECONDS * 1e9) as i128
                + (vox_dsp::calibration::LAG_MAX_MS * 1e6) as i128
                + 20_000_000;
            let last = c.rep_times.last().copied().unwrap_or(0);
            let need = i128::from(last) + i128::from(c.offset_ns.max(0)) + tail_ns;
            let reached = c.blocks.back().is_some_and(|b| {
                i128::from(b.start_ns) + i128::from(frames_to_ns(u64::from(b.frames), c.rate_hz))
                    > need
            });
            if reached {
                let rep_starts = c
                    .rep_times
                    .iter()
                    .map(|&t| {
                        take_index_at(
                            &c.blocks,
                            i128::from(t) + i128::from(c.offset_ns),
                            c.rate_hz,
                        )
                        .map_or(i64::MIN / 2, |k| i64::try_from(k).unwrap_or(i64::MIN / 2))
                    })
                    .collect();
                done = Some(Ok(CalibrationCapture {
                    rate_hz: c.rate_hz,
                    sweep: vox_dsp::calibration::sweep(c.rate_hz),
                    recording: std::mem::take(&mut c.recording),
                    rep_starts,
                    offset_ns: c.offset_ns,
                }));
            }
        }
        if let Some(result) = done {
            self.calib_finish(result);
        }
    }

    /// The run's progress, or its result once (then idle).
    pub(crate) fn calibration_poll(&mut self) -> CalibrationStatus {
        let now = self.now();
        let Some(c) = self.calib.as_mut() else {
            return CalibrationStatus::Idle;
        };
        match c.result.take() {
            Some(result) => {
                self.calib = None;
                CalibrationStatus::Done(result)
            }
            None => {
                let total = vox_dsp::calibration::REPS as f64
                    * vox_dsp::calibration::SPACING_SECONDS
                    + vox_dsp::calibration::SWEEP_SECONDS;
                let elapsed = now.saturating_sub(c.started_ns) as f64 / 1e9;
                CalibrationStatus::Running {
                    progress: (elapsed / total).clamp(0.0, 0.99) as f32,
                }
            }
        }
    }

    /// Aborts a running calibration (its poll then reports `Cancelled`).
    pub(crate) fn calibration_cancel(&mut self) {
        if self.calibrating() {
            self.calib_finish(Err(CalibrationError::Cancelled));
        }
    }

    // --- Monitoring (S1-04 Off/Dry; T-107 Through rack, drift servo, latency readout) ---------

    /// Monitoring off on both streams (the next [`Self::update_monitor`] recomputes it). The
    /// input stops pushing, so the output's fade-out fits what the ring still holds (`ending`).
    fn monitor_reset(&mut self) {
        self.monitor_active = false;
        self.monitor_feeding = false;
        if let Some(i) = self.input.as_mut() {
            let _ = i.cmds.push(InputCmd::Monitor(false));
        }
        let (in_rate_hz, target_frames) = self.monitor_sent.map_or((0, 0), |s| (s.1, s.2));
        self.audio_cmd(AudioCmd::Monitor {
            tap: MonitorTap::Off,
            in_rate_hz,
            target_frames,
            ending: true,
        });
        self.monitor_sent = None;
        self.latency_dirty = true;
    }

    /// The linked (input rate, output rate): both streams open, the input feeding the current
    /// output's monitor ring, a rate pair the monitor resampler supports (ADR-002 §6: any
    /// nominal mismatch, e.g. 48 kHz in → 44.1 kHz out).
    fn monitor_link(&self) -> Option<(u32, u32)> {
        match (&self.input, &self.output) {
            (Some(i), Some(o))
                if i.monitor_gen == Some(o.monitor_gen)
                    && MonitorResampler::supports(i.rate_hz, o.rate_hz) =>
            {
                Some((i.rate_hz, o.rate_hz))
            }
            _ => None,
        }
    }

    /// T-107 (SPEC-002 §2.7): while linked, the input feeds the monitor ring whenever it is
    /// armed or recording — whatever the mode, so a mode switch is only a ≤ 10 ms fade on the
    /// output side — and the output hears it through the mode's tap with the servo target F*
    /// (max input period + max output period + 1 ms). Idempotent: sends only changes.
    fn update_monitor(&mut self) {
        let Some((in_rate, out_rate)) = self.monitor_link() else {
            if self.monitor_feeding || self.monitor_sent.is_some() {
                self.monitor_reset();
            }
            return;
        };
        let feeding = self.armed || self.recording.is_some();
        let tap = match self.monitor_mode {
            _ if !feeding => MonitorTap::Off,
            MonitorMode::Off => MonitorTap::Off,
            MonitorMode::Dry => MonitorTap::Dry,
            MonitorMode::ThroughRack => MonitorTap::Rack,
        };
        if feeding != self.monitor_feeding {
            self.monitor_feeding = feeding;
            if let Some(i) = self.input.as_mut() {
                let _ = i.cmds.push(InputCmd::Monitor(feeding));
            }
        }
        self.monitor_active = tap != MonitorTap::Off;
        let target = monitor::target_frames(&self.in_lat, in_rate, &self.out_lat, out_rate);
        let cmd = (tap, in_rate, target);
        if self.monitor_sent != Some(cmd) {
            self.monitor_sent = Some(cmd);
            self.latency_dirty = true;
            self.audio_cmd(AudioCmd::Monitor {
                tap,
                in_rate_hz: in_rate,
                target_frames: target,
                ending: !feeding,
            });
        }
    }

    /// T-107 (SPEC-002 §2.7, §4.4): recomputes the latency readout at most every 0.5 s, or at
    /// once after a monitoring change; rounded to 0.1 ms so it settles.
    fn update_latency_readout(&mut self, now: u64) {
        let due = self.latency_dirty
            || self
                .latency_checked_ns
                .is_none_or(|t| now.saturating_sub(t) >= LATENCY_READOUT_INTERVAL_NS);
        if !due {
            return;
        }
        self.latency_checked_ns = Some(now);
        self.latency_dirty = false;
        self.monitor_latency_us = self.latency_readout_us();
    }

    /// Input latency + F* + rack latency (through-rack only; bypassed modules keep theirs) +
    /// output latency, in µs rounded to 100 µs. `None` while the mode is Off, unlinked or not
    /// measured yet.
    fn latency_readout_us(&self) -> Option<u32> {
        if self.monitor_mode == MonitorMode::Off {
            return None;
        }
        let (in_rate, out_rate) = self.monitor_link()?;
        if !self.in_lat.measured() || !self.out_lat.measured() {
            return None;
        }
        let rack = match (self.monitor_mode, self.output.as_ref()) {
            (MonitorMode::ThroughRack, Some(o)) => o.rack.total_latency_samples(),
            _ => 0,
        };
        let ns = monitor::readout_ns(&self.in_lat, in_rate, &self.out_lat, out_rate, rack);
        Some(u32::try_from((ns + 50_000) / 100_000 * 100).unwrap_or(u32::MAX))
    }

    /// After the output was rebuilt, an armed (not recording) input feeds a stale monitor ring:
    /// reopen it so it takes the new ring's producer.
    fn relink_monitor(&mut self) {
        if self.monitor_mode == MonitorMode::Off || self.recording.is_some() || !self.armed {
            return;
        }
        let stale = match (&self.input, &self.output) {
            (Some(i), Some(o)) => {
                i.monitor_gen != Some(o.monitor_gen)
                    && self
                        .monitor_tx
                        .as_ref()
                        .is_some_and(|m| m.generation == o.monitor_gen)
            }
            _ => false,
        };
        if stale {
            self.close_input(false);
            self.open_input();
        }
    }

    // --- Tick ------------------------------------------------------------------------------

    pub(crate) fn tick(&mut self) {
        let now = self.now();
        if let ReaderLink::Inline(r) = &mut self.reader {
            r.fill();
        }
        self.drain_rt();
        self.drain_input();
        self.service_calibration(now);
        self.poll_disk(now);
        self.check_disk_floor();
        self.service_op(now);
        self.service_recording(now);
        let notices = self.output.as_mut().map(|out| out.rack.tick());
        if let Some(notices) = notices {
            self.handle_rack_notices(notices);
        }
        self.check_stream(now);
        self.relink_monitor();
        self.update_latency_readout(now);
        // H-16: VXTM/VXMT/VXSA all honour `Settings.telemetry_rate_hz` through one shared gate,
        // decoupled from the fixed 60 Hz control tick above.
        if self.rate_gate.due() {
            self.emit_telemetry(now);
            self.module_telemetry
                .publish(self.output.as_ref().map(|out| &out.rack), now);
            self.analyzer.publish(now);
        }
        self.emit_state_if_changed();
        self.emit_record_if_changed();
    }

    fn drain_rt(&mut self) {
        let events: Vec<RtEvent> = {
            let Some(out) = self.output.as_mut() else {
                return;
            };
            let underruns = out.counters.underruns.load(Ordering::Relaxed);
            if underruns != self.last_underruns {
                self.last_underruns = underruns;
                self.xrun = true;
            }
            std::iter::from_fn(|| out.events.pop().ok()).collect()
        };
        let mut period_grew = false;
        let doc_rate = u64::from(self.doc_rate().max(1));
        let out_rate = self
            .output
            .as_ref()
            .map_or(1, |o| u64::from(o.rate_hz.max(1)));
        for e in events {
            match e {
                RtEvent::Block {
                    epoch,
                    heard_pos,
                    heard_time_ns,
                    latency_ns,
                    frames,
                    peak,
                    sum_sq,
                } => {
                    period_grew |= self.out_lat.observe(frames, latency_ns);
                    self.meter.add(peak, sum_sq, frames);
                    if let Some(pos) = heard_pos
                        && self.transport.playing()
                        && epoch == self.transport.epoch()
                    {
                        self.anchor = Some(Anchor {
                            epoch,
                            pos,
                            time_ns: heard_time_ns,
                        });
                    }
                    // T-304 (SPEC-022 §4.3 `WindowHeard`): the heard time of `at`, from the block
                    // whose heard range contains it (or the first past it).
                    if let Some(pos_v) = heard_pos
                        && let Some(op) = self.recording.as_mut().and_then(|r| r.op.as_mut())
                        && let Some((run_epoch, spec)) = op.run
                        && epoch == run_epoch
                    {
                        op.anchor = Some((pos_v.saturating_sub(spec.offset), heard_time_ns));
                        if op.t_at.is_none() {
                            let at_v = spec.at + spec.offset;
                            let span = (u64::from(frames) * doc_rate / out_rate).max(1);
                            if at_v < pos_v + span {
                                op.t_at = Some(
                                    i128::from(heard_time_ns)
                                        + (i128::from(at_v) - i128::from(pos_v)) * 1_000_000_000
                                            / i128::from(doc_rate),
                                );
                            }
                        }
                    }
                }
                RtEvent::Stopped { epoch, pos } => self.transport.on_stopped(epoch, pos),
                RtEvent::Ended { epoch, pos } => {
                    if let Some(op) = self.recording.as_mut().and_then(|r| r.op.as_mut())
                        && op.run.is_some_and(|(e, _)| e == epoch)
                    {
                        op.run_ended = true;
                    }
                    if self.transport.on_ended(epoch, pos) {
                        self.reader_send(ReaderCmd::Stop);
                    }
                }
                RtEvent::CalibRep { heard_time_ns, .. } => {
                    if let Some(c) = self.calib.as_mut()
                        && c.result.is_none()
                    {
                        c.rep_times.push(heard_time_ns);
                    }
                }
                RtEvent::CalibDone => {}
            }
        }
        if period_grew {
            // T-107: a longer output period raises F* (ADR-002 §6: max observed period).
            self.update_monitor();
        }
    }

    /// Stream flags and the stall detectors (T-102 handoff): a stall counts as device loss.
    fn check_stream(&mut self, now: u64) {
        let out_bits = self.output.as_mut().map_or(0, |out| {
            let status = out.handle.status().clone();
            let mut bits = status.take();
            if out.stall.check(status.callback_count(), now) {
                bits |= flags::DEVICE_LOST;
            }
            bits
        });
        if out_bits != 0 {
            self.device_event(DeviceEvent::Flags {
                dir: Direction::Output,
                bits: out_bits,
            });
        }
        let in_bits = self.input.as_mut().map_or(0, |inp| {
            let status = inp.handle.status().clone();
            let mut bits = status.take();
            if inp.stall.check(status.callback_count(), now) {
                bits |= flags::DEVICE_LOST;
            }
            bits
        });
        if in_bits != 0 {
            self.device_event(DeviceEvent::Flags {
                dir: Direction::Input,
                bits: in_bits,
            });
        }
    }

    /// Telemetry anchor: the heard position while playing, the take position while recording.
    fn anchor_now(&self, now: u64) -> (u64, u64, f64) {
        // T-304: during a record operation the playhead is the heard document position (pre-roll
        // and window alike, frozen once it stops), or `at` + the take for a free start.
        if let Some(rec) = self.recording.as_ref()
            && let Some(op) = rec.op.as_ref()
        {
            let rate = f64::from(op.plan.doc_rate_hz);
            let t = rec.stop_at.map_or(now, |s| s.min(now));
            let moving = if rec.stop_at.is_none() { rate } else { 0.0 };
            if let Some(q) = op.heard_q(t) {
                return (u64::try_from(q.max(0)).unwrap_or(0), t, moving);
            }
            if let Some((pos, time_ns)) = op.anchor {
                return (pos, time_ns, moving);
            }
            if !op.plan.aligned {
                let doc =
                    rec.captured * u64::from(op.plan.doc_rate_hz) / u64::from(rec.rate_hz.max(1));
                return (op.plan.at_samples + doc, rec.anchor_ns, moving);
            }
            return (
                op.plan.at_samples.saturating_sub(op.plan.preroll_samples),
                now,
                0.0,
            );
        }
        if let Some(rec) = self.recording.as_ref() {
            let rate = if rec.stop_at.is_none() {
                f64::from(rec.rate_hz)
            } else {
                0.0
            };
            return (rec.captured, rec.anchor_ns, rate);
        }
        if !self.transport.playing() {
            return (self.transport.playhead(), now, 0.0);
        }
        match self.anchor {
            Some(a) if a.epoch == self.transport.epoch() => {
                (a.pos, a.time_ns, f64::from(self.doc_rate()))
            }
            _ => (self.transport.display_pos(), now, 0.0),
        }
    }

    fn emit_telemetry(&mut self, now: u64) {
        let (peak, peak_db, rms_db) = self.meter.take();
        let (in_peak_db, in_rms_db, in_clip) = if self.input.is_some() {
            self.in_meter.take()
        } else {
            (f32::NEG_INFINITY, f32::NEG_INFINITY, false)
        };
        if self.telemetry.is_none() {
            self.xrun = false;
            return;
        }
        let (pos, time_ns, rate) = self.anchor_now(now);
        let mut bits = 0;
        if self.transport.playing() {
            bits |= vxtm_flags::PLAYING;
        }
        if self.capturing() {
            bits |= vxtm_flags::RECORDING;
        }
        if self.monitor_active {
            bits |= vxtm_flags::MONITORING;
        }
        if self.xrun {
            bits |= vxtm_flags::XRUN;
        }
        if peak >= 1.0 {
            bits |= vxtm_flags::OUT_CLIP;
        }
        if in_clip {
            bits |= vxtm_flags::IN_CLIP;
        }
        let dropped = self.output.as_ref().map_or(0, |o| {
            let rack = u32::try_from(o.rack.dropped_rt_events()).unwrap_or(u32::MAX);
            o.counters
                .dropped_events
                .load(Ordering::Relaxed)
                .saturating_add(rack)
        });
        let dropped = dropped.saturating_add(
            self.input
                .as_ref()
                .map_or(0, |i| i.shared.dropped_events.load(Ordering::Relaxed)),
        );
        let frame = TelemetryFrame {
            seq: self.seq,
            flags: bits,
            playhead_sample: pos,
            playhead_time_ns: time_ns,
            rate,
            out_peak_dbfs: peak_db,
            out_rms_dbfs: rms_db,
            in_peak_dbfs: in_peak_db,
            in_rms_dbfs: in_rms_db,
            audio_rev: self.doc.as_ref().map_or(0, |d| d.snapshot.audio_rev),
            dropped_rt_events: dropped,
        };
        self.seq = self.seq.wrapping_add(1);
        self.xrun = false;
        if let Some(sink) = self.telemetry.as_mut() {
            sink(&frame);
        }
    }

    pub(crate) fn set_telemetry_sink(&mut self, sink: Option<TelemetrySink>) {
        self.telemetry = sink;
        self.seq = 0;
    }

    pub(crate) fn set_module_telemetry_sink(&mut self, sink: Option<ModuleTelemetrySink>) {
        self.module_telemetry.set_sink(sink);
    }

    /// H-16 (SPEC-003 §3, ADR-009): applies a new telemetry rate (60 or 30 Hz) to the
    /// `VXTM`/`VXMT`/`VXSA` publish cadence and to the analyzer's own EMA time constants (which
    /// assume frames arrive at this rate), effective from the next control tick.
    pub(crate) fn set_telemetry_rate_hz(&mut self, rate_hz: u32) {
        self.rate_gate.set_rate_hz(rate_hz);
        self.analyzer
            .set_rate_hz(f64::from(effective_telemetry_rate_hz(rate_hz)));
    }

    /// Registers a new live-analyzer subscriber (`VXSA`, T-208); returns its id.
    pub(crate) fn analyzer_subscribe(
        &mut self,
        sink: AnalyzerSink,
        response: AnalyzerResponse,
    ) -> u32 {
        self.analyzer.subscribe(sink, response)
    }

    /// Changes one subscriber's averaging response.
    pub(crate) fn analyzer_set_response(&mut self, id: u32, response: AnalyzerResponse) {
        self.analyzer.set_response(id, response);
    }

    /// Removes a subscriber; the tap turns off once none remain (`ANALYZER_ON` clear).
    pub(crate) fn analyzer_unsubscribe(&mut self, id: u32) {
        self.analyzer.unsubscribe(id);
    }
}

/// The control thread's loop: messages as they come, a tick every [`TICK`] (checked after every
/// message too, so a steady stream of messages cannot starve the ticks).
pub(crate) fn run(control: &mut Control, rx: &Receiver<ControlMsg>) {
    let mut next = Instant::now() + TICK;
    loop {
        match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(ControlMsg::Call(f)) => f(control),
            Ok(ControlMsg::Poll(ev)) => control.on_poll(ev),
            Ok(ControlMsg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= next {
            control.tick();
            next += TICK;
            let now = Instant::now();
            if next < now {
                next = now + TICK;
            }
        }
    }
}
