//! The output callback (ADR-002 §4), the transport clock. Per device buffer it drains control
//! commands, pulls current-epoch packets from the playback ring (≈5 ms fades on start, stop and
//! seek; a start while audible waits for a fade-out, and a rack reset waits until the rack's
//! latency has drained that fade-out), runs the live rack in sub-blocks of at most `MAX_BLOCK`,
//! mixes the drift-corrected monitor signal in (T-107, SPEC-002 §2.7, ADR-002 §4 step 3–4:
//! Through rack into the rack input with the playback, Dry after the rack; see `monitor`), writes
//! the mono result to every device channel, meters the rack output and reports one
//! [`RtEvent::Block`] (heard position, heard time, output latency, peak, energy) to the control
//! thread. T-401 (SPEC-012 §2.5, SPEC-003 §2.2): the document end ([`RtEvent::Ended`]) is
//! reported once the rack has drained its latency after the last sample, so the transport stops
//! when that sample is heard, not when it enters the rack. While the monitor feeds the rack, a transport restart skips the rack reset: the live
//! input keeps flowing through it, so a reset would cut the talent's monitored voice.
//!
//! T-304 (SPEC-022 §2.14, §4.7, §4.10): during a calibration run the preallocated sweep is mixed
//! in after the rack (5 repetitions, 1.6 s apart); the heard time of each repetition's first
//! sample leaves through the RT event ring. Its state is a read index and a flag.
//!
//! RT contract: no allocation, locks, I/O or panics; the FTZ/DAZ guard is entered first. The
//! callback owns the `LiveRack` and the ring ends. When the stream ends the backend drops the
//! callback (cpal: on its own thread) and `Drop` hands those parts back through a slot, so the
//! control thread tears the rack down with `RackHost::teardown` (T-102/T-103 handoff).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use rtrb::{Consumer, Producer};
use vox_dsp::fp::DenormalGuard;
use vox_rack::{LiveRack, MAX_BLOCK, Transport};

use crate::analyzer::AnalyzerTap;
use crate::backend::{OutputCallback, OutputTimestamp, frames_to_ns};
use crate::monitor::MonitorOut;
use crate::rt::{
    AudioCmd, FADE_MS, MAX_AUDIO_CMDS_PER_CALLBACK, PACKET_FRAMES, PLAYBACK_RING_PACKETS,
    PREBUFFER_MS, Packet, RtCounters, RtEvent, packet_flags,
};

/// What the callback owns that must be torn down on the control thread.
pub(crate) struct OutputParts {
    pub(crate) live: LiveRack,
    pub(crate) packets: Consumer<Packet>,
    pub(crate) cmds: Consumer<AudioCmd>,
    pub(crate) events: Producer<RtEvent>,
    pub(crate) counters: Arc<RtCounters>,
    /// The output side of this stream's monitor ring (the input callback produces).
    pub(crate) monitor: MonitorOut,
    /// T-208: the live analyzer's RT tap (post-rack + dry monitor, SPEC-007 §4.8 step 1).
    pub(crate) analyzer_tap: AnalyzerTap,
    /// T-304 (SPEC-022 §4.7): the calibration sweep at the stream rate, built with the stream.
    pub(crate) calib_sweep: Vec<f32>,
}

/// Where a dropped callback leaves its [`OutputParts`].
pub(crate) type PartsSlot = Arc<Mutex<Option<OutputParts>>>;

/// \[control thread\] Takes the parts a dropped callback left in `slot`.
pub(crate) fn take_parts(slot: &PartsSlot) -> Option<OutputParts> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Silence into the rack.
    Idle,
    /// Waiting for the prebuffer of `epoch` and, before a rack reset, for the rack to drain.
    Waiting,
    /// Playing (fading in while `gain < 1`).
    Playing,
    /// Fading out, then `then`.
    FadingOut { then: AfterFade },
}

/// What follows a fade-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterFade {
    /// Start `epoch` at `pos` with a rack reset (a seek, or a Play during a stop fade).
    Start { epoch: u32, pos: u64 },
    /// Go idle and report [`RtEvent::Stopped`] at `pos` (`None`: where the fade-out ended).
    Stop { pos: Option<u64> },
}

enum Next {
    Sample(f32),
    End,
    Empty,
}

struct State {
    mode: Mode,
    /// Epoch whose packets are played.
    epoch: u32,
    /// Latest commanded epoch (a seek's epoch while its fade-out still plays the old one).
    cmd_epoch: u32,
    start_pos: u64,
    reset_pending: bool,
    gain: f32,
    fade_step: f32,
    prebuffer_frames: u32,
    doc_rate: u64,
    dev_rate: u64,
    cur: Packet,
    cur_off: usize,
    has_cur: bool,
    /// Document position of the next sample to play.
    next_pos: u64,
    /// Silent samples fed to the rack since the last audio. A rack reset waits until they cover
    /// the rack's latency, so the delayed end of a fade-out is heard, not cut.
    quiet: u64,
    /// T-401: the document end `(epoch, pos)`, reported once `quiet` covers the rack's latency
    /// (or at once when a transport command supersedes the drain).
    pending_end: Option<(u32, u64)>,
    underrun: bool,
    rack_in: Vec<f32>,
    rack_out: Vec<f32>,
    /// T-304: the calibration run's position in frames since its start (`None`: idle).
    calib_pos: Option<u64>,
    /// T-304: frames between calibration repetition starts (1.6 s).
    calib_spacing: u64,
}

fn emit(parts: &mut OutputParts, e: RtEvent) {
    if parts.events.push(e).is_err() {
        parts
            .counters
            .dropped_events
            .fetch_add(1, Ordering::Relaxed);
    }
}

impl State {
    fn pos_in_packet(&self, off: usize) -> u64 {
        self.cur.doc_pos + off as u64 * self.doc_rate / self.dev_rate
    }

    fn begin_wait(&mut self, epoch: u32, pos: u64, reset: bool) {
        self.epoch = epoch;
        self.start_pos = pos;
        self.next_pos = pos;
        self.reset_pending |= reset;
        self.has_cur = false;
        self.mode = Mode::Waiting;
    }

    /// Play / seek: while audible, after a fade-out (a start never cuts the signal); else now.
    fn start(&mut self, epoch: u32, pos: u64, reset: bool) {
        self.cmd_epoch = epoch;
        match self.mode {
            Mode::Playing | Mode::FadingOut { .. } => {
                self.mode = Mode::FadingOut {
                    then: AfterFade::Start { epoch, pos },
                };
            }
            Mode::Waiting | Mode::Idle => self.begin_wait(epoch, pos, reset),
        }
    }

    fn command(&mut self, cmd: AudioCmd, parts: &mut OutputParts) {
        if matches!(
            cmd,
            AudioCmd::Play { .. } | AudioCmd::Seek { .. } | AudioCmd::Stop
        ) {
            // The end of the previous pass comes first, whatever the rack still holds.
            self.flush_end(parts, 0, true);
        }
        match cmd {
            AudioCmd::Play { epoch, pos, reset } => self.start(epoch, pos, reset),
            AudioCmd::Seek { epoch, pos } => self.start(epoch, pos, true),
            AudioCmd::Stop => match self.mode {
                Mode::Playing => {
                    self.mode = Mode::FadingOut {
                        then: AfterFade::Stop { pos: None },
                    };
                }
                Mode::FadingOut {
                    then: AfterFade::Start { pos, .. },
                } => {
                    // The transport stops at the pending start's position. The rack still holds
                    // the old material, so the next start resets it whatever it asks for.
                    self.reset_pending = true;
                    self.mode = Mode::FadingOut {
                        then: AfterFade::Stop { pos: Some(pos) },
                    };
                }
                Mode::FadingOut {
                    then: AfterFade::Stop { .. },
                } => {}
                Mode::Waiting => {
                    self.mode = Mode::Idle;
                    emit(
                        parts,
                        RtEvent::Stopped {
                            epoch: self.cmd_epoch,
                            pos: self.start_pos,
                        },
                    );
                }
                Mode::Idle => {}
            },
            AudioCmd::Monitor {
                tap,
                in_rate_hz,
                target_frames,
                ending,
            } => parts
                .monitor
                .command(tap, in_rate_hz, target_frames, ending),
            AudioCmd::Calibrate { start } => {
                if start {
                    self.calib_pos = Some(0);
                } else if self.calib_pos.take().is_some() {
                    emit(parts, RtEvent::CalibDone);
                }
            }
        }
    }

    /// Waiting: drops stale packets at the head, then starts once the prebuffer (or the document
    /// end) of the current epoch is queued and, when the rack must be reset, once the rack
    /// (`latency` samples) has drained the previous fade-out.
    fn check_ready(&mut self, parts: &mut OutputParts, latency: u64) {
        for _ in 0..PLAYBACK_RING_PACKETS {
            let stale = matches!(parts.packets.peek(), Ok(p) if p.epoch != self.epoch);
            if !stale {
                break;
            }
            let _ = parts.packets.pop();
        }
        let n = parts.packets.slots();
        let mut frames: u32 = 0;
        let mut end = false;
        if let Ok(chunk) = parts.packets.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            for p in a.iter().chain(b.iter()) {
                if p.epoch != self.epoch {
                    break;
                }
                frames += u32::from(p.len);
                if p.flags & packet_flags::END != 0 {
                    end = true;
                    break;
                }
            }
            // Dropped without `commit`: nothing is consumed (peek only).
        }
        if frames >= self.prebuffer_frames || end {
            if self.reset_pending {
                // T-107: never reset a rack the live monitor signal flows through (it would cut
                // the talent's voice); the playback fade-out/in already keeps the start clean.
                if !parts.monitor.feeds_rack() {
                    if self.quiet < latency {
                        return;
                    }
                    parts.live.reset();
                }
                self.reset_pending = false;
            }
            self.mode = Mode::Playing;
            self.gain = 0.0;
        }
    }

    fn next(&mut self, parts: &mut OutputParts) -> Next {
        for _ in 0..=PLAYBACK_RING_PACKETS {
            let len = usize::from(self.cur.len).min(PACKET_FRAMES);
            if self.has_cur && self.cur_off < len {
                let x = self.cur.samples[self.cur_off];
                self.cur_off += 1;
                self.next_pos = self.pos_in_packet(self.cur_off);
                return Next::Sample(x);
            }
            let (epoch, flags) = match parts.packets.peek() {
                Ok(p) => (p.epoch, p.flags),
                Err(_) => return Next::Empty,
            };
            if epoch != self.epoch {
                if epoch == self.cmd_epoch {
                    // The next epoch's packets wait until the fade-out is over.
                    return Next::Empty;
                }
                let _ = parts.packets.pop();
                continue;
            }
            match parts.packets.pop() {
                Ok(p) if flags & packet_flags::END != 0 => {
                    self.has_cur = false;
                    self.next_pos = p.doc_pos;
                    return Next::End;
                }
                Ok(p) => {
                    self.cur = p;
                    self.cur_off = 0;
                    self.has_cur = true;
                }
                Err(_) => return Next::Empty,
            }
        }
        Next::Empty
    }

    /// The document end entered the rack: go idle; [`flush_end`](Self::flush_end) reports it
    /// once it has been heard.
    fn end_reached(&mut self) {
        self.mode = Mode::Idle;
        self.has_cur = false;
        self.pending_end = Some((self.epoch, self.next_pos));
    }

    /// Reports a pending document end once the silence fed after it covers the rack's `latency`
    /// (its last sample has left the rack), or at once when `force`d.
    fn flush_end(&mut self, parts: &mut OutputParts, latency: u64, force: bool) {
        if let Some((epoch, pos)) = self.pending_end
            && (force || self.quiet >= latency)
        {
            self.pending_end = None;
            emit(parts, RtEvent::Ended { epoch, pos });
        }
    }

    /// The fade-out is over, or the document ended during it (`next_pos` = the end then).
    fn finish_fade(&mut self, then: AfterFade, parts: &mut OutputParts) {
        self.gain = 0.0;
        match then {
            AfterFade::Start { epoch, pos } => self.begin_wait(epoch, pos, true),
            AfterFade::Stop { pos } => {
                self.mode = Mode::Idle;
                self.has_cur = false;
                emit(
                    parts,
                    RtEvent::Stopped {
                        epoch: self.cmd_epoch,
                        pos: pos.unwrap_or(self.next_pos),
                    },
                );
            }
        }
    }

    fn sample(&mut self, parts: &mut OutputParts) -> f32 {
        match self.mode {
            Mode::Idle | Mode::Waiting => {
                self.quiet = self.quiet.saturating_add(1);
                0.0
            }
            Mode::Playing => {
                self.quiet = 0;
                match self.next(parts) {
                    Next::Sample(x) => {
                        let y = x * self.gain;
                        if self.gain < 1.0 {
                            self.gain = (self.gain + self.fade_step).min(1.0);
                        }
                        y
                    }
                    Next::End => {
                        self.end_reached();
                        0.0
                    }
                    Next::Empty => {
                        self.underrun = true;
                        0.0
                    }
                }
            }
            Mode::FadingOut { then } => {
                self.quiet = 0;
                let y = match self.next(parts) {
                    Next::Sample(x) => x * self.gain,
                    Next::End => {
                        self.finish_fade(then, parts);
                        return 0.0;
                    }
                    Next::Empty => 0.0,
                };
                self.gain -= self.fade_step;
                if self.gain <= 0.0 {
                    self.finish_fade(then, parts);
                }
                y
            }
        }
    }

    /// T-304: the calibration sweep sample for the frame heard at `heard_ns` (0 when idle);
    /// reports each repetition's start and the end of the run. Bounded, allocation-free.
    fn calib_sample(&mut self, parts: &mut OutputParts, heard_ns: u64) -> f32 {
        let Some(pos) = self.calib_pos else {
            return 0.0;
        };
        let spacing = self.calib_spacing.max(1);
        let rep = pos / spacing;
        if rep >= vox_dsp::calibration::REPS as u64 {
            self.calib_pos = None;
            emit(parts, RtEvent::CalibDone);
            return 0.0;
        }
        let off = (pos % spacing) as usize;
        if off == 0 {
            emit(
                parts,
                RtEvent::CalibRep {
                    rep: rep as u32,
                    heard_time_ns: heard_ns,
                },
            );
        }
        self.calib_pos = Some(pos + 1);
        parts.calib_sweep.get(off).copied().unwrap_or(0.0)
    }
}

/// The output stream's callback.
pub(crate) struct OutputCb {
    st: State,
    parts: Option<OutputParts>,
    slot: PartsSlot,
}

impl OutputCb {
    /// A callback for a stream at `dev_rate_hz` playing a document at `doc_rate_hz`.
    pub(crate) fn new(
        parts: OutputParts,
        slot: PartsSlot,
        dev_rate_hz: u32,
        doc_rate_hz: u32,
    ) -> Self {
        let rate = f64::from(dev_rate_hz.max(1));
        let fade_len = (rate * FADE_MS / 1000.0).round().max(1.0);
        let max_block = MAX_BLOCK as usize;
        Self {
            st: State {
                mode: Mode::Idle,
                epoch: 0,
                cmd_epoch: 0,
                start_pos: 0,
                reset_pending: false,
                gain: 0.0,
                fade_step: (1.0 / fade_len) as f32,
                prebuffer_frames: (rate * PREBUFFER_MS / 1000.0).round() as u32,
                doc_rate: u64::from(doc_rate_hz.max(1)),
                dev_rate: u64::from(dev_rate_hz.max(1)),
                cur: Packet::new(0),
                cur_off: 0,
                has_cur: false,
                next_pos: 0,
                quiet: u64::MAX,
                pending_end: None,
                underrun: false,
                rack_in: vec![0.0; max_block],
                rack_out: vec![0.0; max_block],
                calib_pos: None,
                calib_spacing: vox_dsp::calibration::spacing_frames(dev_rate_hz.max(1)),
            },
            parts: Some(parts),
            slot,
        }
    }
}

impl Drop for OutputCb {
    fn drop(&mut self) {
        if let Some(parts) = self.parts.take() {
            *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(parts);
        }
    }
}

impl OutputCallback for OutputCb {
    fn process(&mut self, data: &mut [f32], channels: usize, ts: OutputTimestamp) {
        let _fp = DenormalGuard::new();
        let OutputCb { st, parts, .. } = self;
        let Some(parts) = parts.as_mut() else {
            data.fill(0.0);
            return;
        };
        for _ in 0..MAX_AUDIO_CMDS_PER_CALLBACK {
            match parts.cmds.pop() {
                Ok(cmd) => st.command(cmd, parts),
                Err(_) => break,
            }
        }
        let channels = channels.max(1);
        let frames = data.len() / channels;
        let latency = u64::from(parts.live.latency_samples());
        let latency_doc = (latency * st.doc_rate + st.dev_rate / 2) / st.dev_rate;
        let block_epoch = st.epoch;
        let block_start = st.start_pos;
        let heard_time_ns = ts.to_app_ns(ts.playback_ns);
        let dev_rate = u32::try_from(st.dev_rate).unwrap_or(u32::MAX);
        let mut first_pos: Option<u64> = None;
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut done = 0;
        // T-107: priming, overrun handling and the drift servo, once per callback.
        parts.monitor.begin(ts.now_ns, frames);
        while done < frames {
            let mut n = (frames - done).min(st.rack_in.len());
            if st.mode == Mode::Waiting {
                st.check_ready(parts, latency);
                if st.mode == Mode::Waiting && st.reset_pending && st.quiet < latency {
                    // Draining: end the sub-block where the rack is empty, so the reset and the
                    // fade-in follow at once.
                    n = n.min((latency - st.quiet) as usize);
                }
            }
            let playing = matches!(st.mode, Mode::Playing | Mode::FadingOut { .. });
            let sub_pos = playing.then_some(st.next_pos);
            if first_pos.is_none() {
                first_pos = sub_pos;
            }
            for i in 0..n {
                st.rack_in[i] = st.sample(parts);
            }
            // T-107 (SPEC-002 §2.7): the Through-rack tap joins the playback at the rack input.
            parts.monitor.render(n);
            for (x, &w) in st.rack_in[..n].iter_mut().zip(parts.monitor.wet()) {
                *x += w;
            }
            let transport = Transport {
                playing,
                position_samples: sub_pos,
            };
            parts
                .live
                .process(transport, &st.rack_in[..n], &mut st.rack_out[..n]);
            for i in 0..n {
                let s = st.rack_out[i];
                peak = peak.max(s.abs());
                sum_sq += f64::from(s) * f64::from(s);
                // Dry monitoring: after the rack, unity gain (never recorded, never metered).
                let mut y = s + parts.monitor.dry()[i];
                // T-304 (SPEC-022 §2.14): the calibration sweep, after the rack.
                if st.calib_pos.is_some() {
                    let heard = heard_time_ns + frames_to_ns((done + i) as u64, dev_rate);
                    y += st.calib_sample(parts, heard);
                }
                // T-208: stage the analyzer tap's signal in `rack_out` (no longer needed as `s`
                // past this point) rather than a second scratch buffer.
                st.rack_out[i] = y;
                let base = (done + i) * channels;
                data[base..base + channels].fill(y);
            }
            // SPEC-007 §4.8 step 1: the same `out` signal as the output meter (rack + dry
            // monitor), one bounded push per sub-block. A no-op unless a subscriber exists.
            parts.analyzer_tap.push(&st.rack_out[..n]);
            done += n;
        }
        st.flush_end(parts, latency, false);
        if st.underrun {
            st.underrun = false;
            parts.counters.underruns.fetch_add(1, Ordering::Relaxed);
        }
        let heard_pos = first_pos.map(|p| {
            let start = if block_epoch == st.epoch {
                block_start
            } else {
                0
            };
            p.saturating_sub(latency_doc).max(start)
        });
        emit(
            parts,
            RtEvent::Block {
                epoch: block_epoch,
                heard_pos,
                heard_time_ns,
                latency_ns: u32::try_from(heard_time_ns.saturating_sub(ts.now_ns))
                    .unwrap_or(u32::MAX),
                frames: frames as u32,
                peak,
                sum_sq,
            },
        );
    }
}
