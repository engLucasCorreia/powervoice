//! The control thread (ADR-002 §1): the only consumer of the engine inbox and the only producer
//! of every queue into the audio thread. Owns the device configuration and the device-lost state
//! machine, the output stream (with its rack host), the transport, the playback document and the
//! telemetry sink. Ticks every ~16.7 ms (60 Hz telemetry): drains the RT events, ticks the rack
//! host, checks stream flags and the stall detector, sends a telemetry frame.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use vox_rack::{
    ActivateConfig, ChannelLayout, MAX_BLOCK, ProcessMode, RackHost, RackModel, RackOptions,
    Registry,
};

use crate::backend::{
    Backend, BackendError, BufferRequest, Direction, Enumerate, HostId, StallDetector,
    StreamHandle, StreamRequest, choose_default_host, flags,
};
use crate::device_state::{Activity, DeviceAction, DeviceEvent, DeviceStateMachine, LinkState};
use crate::devices::{
    DEVICE_POLL_INTERVAL, DeviceList, DeviceNotice, DevicePollThread, DeviceWatcher, PollEvent,
    resolve_devices, resolve_host,
};
use crate::engine::{Clock, DevicesView, EngineConfig, EngineEvent, EventSink, PlaybackDoc};
use crate::output::{OutputCb, OutputParts, PartsSlot, take_parts};
use crate::prefs::DevicePrefs;
use crate::reader::{self, Reader, ReaderCmd};
use crate::rt::{
    AUDIO_CMD_CAPACITY, AudioCmd, PLAYBACK_RING_PACKETS, RT_EVENT_CAPACITY, RtCounters, RtEvent,
};
use crate::telemetry::{Meter, TelemetryFrame, TelemetrySink, vxtm_flags};
use crate::transport::{Action, Transport, TransportCommand, TransportState};

/// Control tick = telemetry period (60 Hz, ADR-009).
pub(crate) const TICK: Duration = Duration::from_nanos(16_666_667);

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
    last_state: Option<TransportState>,
}

impl Control {
    /// `inbox`: `Some` = threaded (reader and device-poll threads, poll events arrive through the
    /// inbox); `None` = manual (reader inline, devices polled by [`Self::poll_devices`]).
    pub(crate) fn new(config: EngineConfig, inbox: Option<mpsc::Sender<ControlMsg>>) -> Self {
        let EngineConfig {
            backend,
            registry,
            rack,
            prefs,
            events,
            clock,
        } = config;
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
            last_state: None,
        }
    }

    /// Closes the stream and stops the helper threads.
    pub(crate) fn shutdown(&mut self) {
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
        self.transport.len() > 0 && self.output.is_some()
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

    pub(crate) fn transport_command(&mut self, cmd: TransportCommand) -> TransportState {
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
        }
    }

    /// Applies new device preferences (Settings → Audio Devices). The input choice is only
    /// stored until recording exists (S1-04).
    pub(crate) fn select_devices(&mut self, prefs: DevicePrefs) -> DevicesView {
        self.engine_stop();
        let new_host = if self.hosts.is_empty() {
            prefs.host.or(self.host)
        } else {
            resolve_host(prefs.host, &self.hosts, choose_default_host(&self.hosts)).map(|(h, _)| h)
        };
        self.prefs = prefs;
        self.out_device_id = None;
        if new_host.is_some() && new_host != self.host {
            self.switch_host(new_host);
            self.force_configure = true;
        } else {
            self.open_output(true);
        }
        let view = self.devices_view();
        (self.events)(EngineEvent::Devices(view.clone()));
        self.emit_state_if_changed();
        view
    }

    fn switch_host(&mut self, host: Option<HostId>) {
        self.close_output();
        self.device_event(DeviceEvent::Configured {
            dir: Direction::Output,
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
                (self.events)(EngineEvent::Devices(self.devices_view()));
                self.emit_state_if_changed();
            }
            PollEvent::Error { .. } => {}
        }
    }

    /// Is the configured output device listed? Prefers its backend id (twin devices).
    fn output_present(&self) -> Option<bool> {
        let name = self.devices.device(Direction::Output)?;
        Some(match &self.out_device_id {
            Some(id) => self
                .list
                .snapshot
                .devices_for(Direction::Output)
                .any(|d| &d.id == id),
            None => self.list.is_present(name, Direction::Output),
        })
    }

    fn device_event(&mut self, ev: DeviceEvent) {
        let act = Activity {
            playing: self.transport.playing(),
            recording: false,
            monitoring: false,
        };
        for action in self.devices.handle(ev, act) {
            match action {
                DeviceAction::StopPlayback => self.engine_stop(),
                DeviceAction::CloseStream(Direction::Output) => self.close_output(),
                DeviceAction::ReopenStream(Direction::Output) => self.open_output(false),
                DeviceAction::Notice(n) => self.notify(n),
                // No input stream, recording or monitoring yet (S1-04).
                _ => {}
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
            for n in &res.notices {
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
                self.device_event(DeviceEvent::Opened {
                    dir: Direction::Output,
                    fallback,
                });
            }
            Err(_) => self.device_event(DeviceEvent::OpenFailed {
                dir: Direction::Output,
            }),
        }
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
        let counters = Arc::new(RtCounters::default());
        let slot: PartsSlot = Arc::new(Mutex::new(None));
        let parts = OutputParts {
            live,
            packets: pkt_rx,
            cmds: cmd_rx,
            events: ev_tx,
            counters: counters.clone(),
        };
        let cb = OutputCb::new(parts, slot.clone(), rate, doc_rate);
        match self.backend.open_output(req, Box::new(cb)) {
            Ok(handle) => {
                let stall = StallDetector::new(handle.info(), self.now());
                let buffer = handle.info().buffer;
                self.reader_send(ReaderCmd::Attach {
                    producer: pkt_tx,
                    dev_rate_hz: rate,
                });
                self.last_underruns = 0;
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
    /// tears the rack down here, on the control thread.
    fn close_output(&mut self) {
        let Some(out) = self.output.take() else {
            return;
        };
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
    }

    // --- Tick ------------------------------------------------------------------------------

    pub(crate) fn tick(&mut self) {
        let now = self.now();
        if let ReaderLink::Inline(r) = &mut self.reader {
            r.fill();
        }
        self.drain_rt();
        if let Some(out) = self.output.as_mut() {
            let _ = out.rack.tick();
        }
        self.check_stream(now);
        self.emit_telemetry(now);
        self.emit_state_if_changed();
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
        for e in events {
            match e {
                RtEvent::Block {
                    epoch,
                    heard_pos,
                    heard_time_ns,
                    frames,
                    peak,
                    sum_sq,
                } => {
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
                }
                RtEvent::Stopped { epoch, pos } => self.transport.on_stopped(epoch, pos),
                RtEvent::Ended { epoch, pos } => {
                    if self.transport.on_ended(epoch, pos) {
                        self.reader_send(ReaderCmd::Stop);
                    }
                }
            }
        }
    }

    /// Stream flags and the stall detector (T-102 handoff): a stall counts as device loss.
    fn check_stream(&mut self, now: u64) {
        let bits = {
            let Some(out) = self.output.as_mut() else {
                return;
            };
            let status = out.handle.status().clone();
            let mut bits = status.take();
            if out.stall.check(status.callback_count(), now) {
                bits |= flags::DEVICE_LOST;
            }
            bits
        };
        if bits != 0 {
            self.device_event(DeviceEvent::Flags {
                dir: Direction::Output,
                bits,
            });
        }
    }

    fn anchor_now(&self, now: u64) -> (u64, u64, f64) {
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
        if self.telemetry.is_none() {
            self.xrun = false;
            return;
        }
        let (pos, time_ns, rate) = self.anchor_now(now);
        let mut bits = 0;
        if self.transport.playing() {
            bits |= vxtm_flags::PLAYING;
        }
        if self.xrun {
            bits |= vxtm_flags::XRUN;
        }
        if peak >= 1.0 {
            bits |= vxtm_flags::OUT_CLIP;
        }
        let dropped = self.output.as_ref().map_or(0, |o| {
            let rack = u32::try_from(o.rack.dropped_rt_events()).unwrap_or(u32::MAX);
            o.counters
                .dropped_events
                .load(Ordering::Relaxed)
                .saturating_add(rack)
        });
        let frame = TelemetryFrame {
            seq: self.seq,
            flags: bits,
            playhead_sample: pos,
            playhead_time_ns: time_ns,
            rate,
            out_peak_dbfs: peak_db,
            out_rms_dbfs: rms_db,
            in_peak_dbfs: f32::NEG_INFINITY,
            in_rms_dbfs: f32::NEG_INFINITY,
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
}

/// The control thread's loop: messages as they come, a tick every [`TICK`].
pub(crate) fn run(control: &mut Control, rx: &Receiver<ControlMsg>) {
    let mut next = Instant::now() + TICK;
    loop {
        match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(ControlMsg::Call(f)) => f(control),
            Ok(ControlMsg::Poll(ev)) => control.on_poll(ev),
            Ok(ControlMsg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                control.tick();
                next += TICK;
                let now = Instant::now();
                if next < now {
                    next = now + TICK;
                }
            }
        }
    }
}
