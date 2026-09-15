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
//! H-37 (SPEC-003 §2.1, ADR-002 §8 and Amendment 3): a loop wrap ([`packet_flags::LOOP_WRAP`])
//! is seamless — no fade, no rack reset (the rack's tails continue across the seam). A small
//! preallocated history of where the rack input started or jumped (`(rack frame, document
//! position)`) maps the heard position back into the loop end for one rack latency after a wrap,
//! instead of clamping it at the play start. Loop turned off mid-pass ends playback at the next
//! wrap (the old loop end).
//!
//! H-46 (SPEC-003 Amendment 2, SPEC-012 §2.5.2, ADR-002 Amendment 4): **rack pre-roll.** A start
//! (Play, seek, a record run) does not add the rack's latency L: once ready, the callback feeds
//! the rack, off the device timeline and with its output discarded, the warm-up the reader put
//! before the play position ([`packet_flags::PREROLL`], L samples; only when the rack is reset)
//! and then L frames of the stream from the play position (the look-ahead). The next sample out
//! of the rack is the play position: it is heard in the same callback, faded in after the rack.
//! The burst is bounded per callback ([`PREROLL_SPEED`]) and waits for the reader when the ring
//! runs dry (it never substitutes silence for a stream sample); meanwhile the device gets silence
//! and the rack is not advanced. A start while audible fades the heard output out after the
//! rack, at once, and resets the rack (its old contents are dropped under the fade) instead of
//! waiting for the rack to drain. Rack frames (`State::frame`) count the burst, so the H-37
//! heard-position history stays exact. Through-rack monitoring (T-107) keeps the old start: no
//! reset, no pre-roll, a fade before the rack.
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
    PREBUFFER_MS, PREROLL_SPEED, Packet, RtCounters, RtEvent, packet_flags,
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
    /// Waiting for the prebuffer of `epoch` and, before a pre-roll, for the rack to drain.
    Waiting,
    /// H-46: feeding the rack its pre-roll (warm-up, then the look-ahead) off the device
    /// timeline; the device gets silence meanwhile.
    Prerolling,
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
    /// H-46: warming up, and the next packet is the play position's (not consumed).
    WarmDone,
}

/// Frames of `epoch` queued at the head of the ring (peek only; warm-up frames counted when
/// `warm_up`), and whether the stream end follows them.
fn queued(packets: &mut Consumer<Packet>, epoch: u32, warm_up: bool) -> (u32, bool) {
    let n = packets.slots();
    let mut frames: u32 = 0;
    if let Ok(chunk) = packets.read_chunk(n) {
        let (a, b) = chunk.as_slices();
        for p in a.iter().chain(b.iter()) {
            if p.epoch != epoch {
                break;
            }
            if p.flags & packet_flags::END != 0 {
                return (frames, true);
            }
            if warm_up || p.flags & packet_flags::PREROLL == 0 {
                frames += u32::from(p.len);
            }
        }
        // Dropped without `commit`: nothing is consumed (peek only).
    }
    (frames, false)
}

/// Marks kept for the heard-position mapping (at least one rack latency of loop passes; a loop
/// is ≥ 10 ms, so 128 marks cover ≥ 1.28 s of latency).
const HISTORY: usize = 128;

/// Where the rack input (re)started (`start`: a play/seek start) or jumped (a loop wrap):
/// rack input frame `frame` carries document position `doc`.
#[derive(Clone, Copy, Debug, Default)]
struct Mark {
    frame: u64,
    doc: u64,
    start: bool,
}

/// A fixed ring of the latest [`Mark`]s (no allocation after construction).
struct History {
    marks: [Mark; HISTORY],
    /// Index of the next write.
    head: usize,
    len: usize,
}

impl History {
    fn new() -> Self {
        Self {
            marks: [Mark::default(); HISTORY],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, m: Mark) {
        self.marks[self.head] = m;
        self.head = (self.head + 1) % HISTORY;
        self.len = (self.len + 1).min(HISTORY);
    }

    /// The document position heard when rack input frame `h` (= first frame − rack latency)
    /// leaves the rack. `now_pos` is the position entering the rack at the first frame and
    /// `latency_doc` the rack latency in document samples (ADR-002 §8's `p_in − L_rack`, which
    /// holds inside the newest segment). An older segment maps exactly; a frame before the
    /// current play start clamps to it ("at least the play start").
    fn heard(&self, h: i128, now_pos: u64, latency_doc: u64, doc_rate: u64, dev_rate: u64) -> u64 {
        for i in 0..self.len {
            let m = self.marks[(self.head + HISTORY - 1 - i) % HISTORY];
            if i128::from(m.frame) <= h {
                if i == 0 {
                    return now_pos.saturating_sub(latency_doc).max(m.doc);
                }
                let into = u64::try_from(h - i128::from(m.frame)).unwrap_or(0);
                return m.doc + into * doc_rate / dev_rate.max(1);
            }
            if m.start {
                return m.doc;
            }
        }
        now_pos.saturating_sub(latency_doc)
    }
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
    /// H-37: rack input frames so far (the index of the frame being produced).
    frame: u64,
    /// H-37: recent starts and loop wraps of the rack input, for the heard position.
    history: History,
    /// H-37: loop was turned off mid-pass — the next [`packet_flags::LOOP_WRAP`] ends playback.
    end_at_wrap: bool,
    /// H-46: pre-rolling the warm-up ([`packet_flags::PREROLL`] packets).
    warming: bool,
    /// H-46: look-ahead frames still to feed the rack off the device timeline.
    burst_left: u64,
    /// H-46: gain after the rack (a pre-rolled start fades in there, a seek fades out there).
    out_gain: f32,
    /// H-46: the current fade-out is after the rack; `post_left` frames of it remain.
    post_fade: bool,
    post_left: u32,
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
        self.warming = false;
        self.burst_left = 0;
        self.mode = Mode::Waiting;
    }

    /// Play / seek: while audible, after a fade-out (a start never cuts the signal); else now.
    /// H-46: unless the live input flows through the rack, that fade-out is after the rack and
    /// starts at once — what the rack still holds of the old position is dropped under it, so the
    /// new position is not delayed by the rack's latency.
    fn start(&mut self, epoch: u32, pos: u64, reset: bool, feeds_rack: bool) {
        self.cmd_epoch = epoch;
        match self.mode {
            Mode::Playing | Mode::FadingOut { .. } => {
                self.mode = Mode::FadingOut {
                    then: AfterFade::Start { epoch, pos },
                };
                if !feeds_rack && !self.post_fade {
                    self.post_fade = true;
                    self.post_left = (self.out_gain / self.fade_step).round() as u32;
                }
            }
            Mode::Waiting | Mode::Idle | Mode::Prerolling => self.begin_wait(epoch, pos, reset),
        }
    }

    /// H-46: the rack is being pre-rolled (fed off the device timeline).
    fn bursting(&self) -> bool {
        self.mode == Mode::Prerolling || self.burst_left > 0
    }

    /// H-46: rack input frame `frame` carries the play position (the start's history mark).
    fn push_start(&mut self) {
        self.history.push(Mark {
            frame: self.frame,
            doc: self.start_pos,
            start: true,
        });
    }

    /// H-46: a transport command interrupted a pre-roll. The rack holds only audio nobody heard
    /// yet: it is reset (never under through-rack monitoring, T-107), and the next start
    /// pre-rolls a clean rack.
    fn abandon_preroll(&mut self, parts: &mut OutputParts) {
        if !parts.monitor.feeds_rack() {
            parts.live.reset();
            self.quiet = u64::MAX;
        }
        self.reset_pending = true;
        self.warming = false;
        self.burst_left = 0;
        self.out_gain = 1.0;
        if self.mode == Mode::Prerolling {
            self.mode = Mode::Waiting;
        }
    }

    /// H-46: the gain after the rack for the next output frame (a seek's fade-out to 0, a
    /// pre-rolled start's fade-in from 0).
    fn next_out_gain(&mut self) -> f32 {
        if self.post_fade {
            self.post_left = self.post_left.saturating_sub(1);
            self.out_gain = self.post_left as f32 * self.fade_step;
            self.out_gain
        } else if self.out_gain < 1.0 {
            let g = self.out_gain;
            self.out_gain = (g + self.fade_step).min(1.0);
            g
        } else {
            1.0
        }
    }

    fn command(&mut self, cmd: AudioCmd, parts: &mut OutputParts) {
        if matches!(
            cmd,
            AudioCmd::Play { .. } | AudioCmd::Seek { .. } | AudioCmd::Stop
        ) {
            // The end of the previous pass comes first, whatever the rack still holds.
            self.flush_end(parts, 0, true);
            self.end_at_wrap = false;
            if self.bursting() {
                self.abandon_preroll(parts);
            }
        }
        let feeds_rack = parts.monitor.feeds_rack();
        match cmd {
            AudioCmd::Play { epoch, pos, reset } => self.start(epoch, pos, reset, feeds_rack),
            AudioCmd::Seek { epoch, pos } => self.start(epoch, pos, true, feeds_rack),
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
                // A pre-roll was abandoned above (the rack reset): nothing was heard yet.
                Mode::Waiting | Mode::Prerolling => {
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
            AudioCmd::SetLoop { finish_pass } => self.end_at_wrap = finish_pass,
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
    /// end) of the current epoch is queued. H-46: unless the live input flows through the rack
    /// (T-107: then the start is as before — no reset, no pre-roll, a fade-in before the rack), a
    /// stop's tail still in the rack plays out first (until `latency` samples of silence were fed
    /// after it: nothing heard is cut), the rack is reset if asked, and the pre-roll begins.
    fn check_ready(&mut self, parts: &mut OutputParts, latency: u64) {
        let feeds_rack = parts.monitor.feeds_rack();
        // The warm-up goes to a rack reset for this start; any other start drops it.
        let warm = self.reset_pending && !feeds_rack;
        for _ in 0..PLAYBACK_RING_PACKETS {
            let stale = matches!(
                parts.packets.peek(),
                Ok(p) if p.epoch != self.epoch || (!warm && p.flags & packet_flags::PREROLL != 0)
            );
            if !stale {
                break;
            }
            let _ = parts.packets.pop();
        }
        let (frames, end) = queued(&mut parts.packets, self.epoch, true);
        if frames < self.prebuffer_frames && !end {
            return;
        }
        if feeds_rack {
            // T-107: never reset (or pre-roll) a rack the live monitor signal flows through (it
            // would cut the talent's voice); the playback fade-out/in already keeps the start
            // clean.
            self.reset_pending = false;
            self.mode = Mode::Playing;
            self.gain = 0.0;
            self.push_start();
            return;
        }
        if self.quiet < latency {
            return;
        }
        if self.reset_pending {
            parts.live.reset();
            self.reset_pending = false;
        }
        self.mode = Mode::Prerolling;
        self.gain = 1.0;
        self.out_gain = 0.0;
        self.warming = warm;
        if !warm {
            self.burst_left = latency;
            self.push_start();
        }
    }

    /// H-46 (SPEC-003 Amendment 2): feeds the rack its pre-roll with the output discarded — the
    /// warm-up packets, then `latency` frames of the stream from the play position (the
    /// look-ahead), so that the play position is the next sample out of the rack — at most
    /// `budget` frames. Stops early when the ring runs dry (the reader catches up by a later
    /// callback): no stream sample is ever replaced by silence. A document end inside the
    /// look-ahead is completed with silence (the end is then heard at once, and reported once
    /// heard, T-401). Once done, playback starts when the prebuffer is queued again.
    fn burst(&mut self, parts: &mut OutputParts, latency: u64, budget: &mut u64) {
        while *budget > 0 && self.bursting() {
            if self.mode == Mode::Prerolling && parts.monitor.feeds_rack() {
                // T-107: the live input joined the rack mid-pre-roll — start from here as is.
                if self.warming {
                    self.warming = false;
                    self.push_start();
                }
                self.burst_left = 0;
                self.mode = Mode::Playing;
                return;
            }
            if self.mode == Mode::Prerolling && !self.warming && self.burst_left == 0 {
                break;
            }
            let mut cap = self
                .rack_in
                .len()
                .min(usize::try_from(*budget).unwrap_or(usize::MAX));
            if !self.warming {
                cap = cap.min(usize::try_from(self.burst_left).unwrap_or(usize::MAX));
            }
            let playing = self.mode == Mode::Prerolling;
            let transport = Transport {
                playing,
                position_samples: playing.then_some(self.next_pos),
            };
            let mut n = 0;
            let mut stalled = false;
            while n < cap {
                let x = if self.mode == Mode::Idle {
                    // The document ended inside the look-ahead: silence completes it.
                    self.quiet = self.quiet.saturating_add(1);
                    0.0
                } else {
                    match self.next(parts) {
                        Next::Sample(x) => {
                            self.quiet = 0;
                            x
                        }
                        Next::WarmDone => {
                            // The play position enters the rack next: the look-ahead begins.
                            self.warming = false;
                            self.burst_left = latency;
                            self.push_start();
                            break;
                        }
                        Next::End => {
                            self.quiet = 0;
                            self.end_reached();
                            0.0
                        }
                        Next::Empty => {
                            stalled = true;
                            break;
                        }
                    }
                };
                self.rack_in[n] = x;
                n += 1;
                self.frame += 1;
                if !self.warming {
                    self.burst_left -= 1;
                }
            }
            if n > 0 {
                parts
                    .live
                    .process(transport, &self.rack_in[..n], &mut self.rack_out[..n]);
                *budget -= n as u64;
            }
            if stalled {
                break;
            }
        }
        if self.mode == Mode::Prerolling && !self.warming && self.burst_left == 0 {
            let in_cur = if self.has_cur {
                usize::from(self.cur.len)
                    .min(PACKET_FRAMES)
                    .saturating_sub(self.cur_off)
            } else {
                0
            };
            let (frames, end) = queued(&mut parts.packets, self.epoch, false);
            if u64::from(frames) + in_cur as u64 >= u64::from(self.prebuffer_frames) || end {
                self.mode = Mode::Playing;
            }
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
            // H-46: the warm-up comes first in an epoch. While warming, the first other packet
            // (the play position's) ends it; otherwise warm-up packets are dropped.
            if (flags & packet_flags::PREROLL != 0) != self.warming {
                if self.warming {
                    return Next::WarmDone;
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
                Ok(_) if flags & packet_flags::LOOP_WRAP != 0 && self.end_at_wrap => {
                    // SPEC-003 §2.1: loop off — the pass just played was the last; `next_pos`
                    // is the old loop end.
                    self.has_cur = false;
                    return Next::End;
                }
                Ok(p) => {
                    if flags & packet_flags::LOOP_WRAP != 0 {
                        self.history.push(Mark {
                            frame: self.frame,
                            doc: p.doc_pos,
                            start: false,
                        });
                    }
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
        if self.post_fade {
            self.post_fade = false;
            self.post_left = 0;
            // H-46: the heard output faded out after the rack; the old audio still in the rack is
            // dropped with a reset (not under through-rack monitoring, T-107: the output then
            // fades back in over what the rack holds).
            if !parts.monitor.feeds_rack() {
                parts.live.reset();
                self.quiet = u64::MAX;
                self.out_gain = 1.0;
            }
            self.reset_pending = true;
        }
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
            // H-46: never fed to the rack sample by sample (see `burst`).
            Mode::Prerolling => 0.0,
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
                    Next::Empty | Next::WarmDone => {
                        self.underrun = true;
                        0.0
                    }
                }
            }
            Mode::FadingOut { then } => {
                self.quiet = 0;
                if self.post_fade {
                    // H-46: the fade is after the rack; the rack keeps getting the old stream
                    // (silence past its end) until the fade is over, then it is reset.
                    return match self.next(parts) {
                        Next::Sample(x) => x * self.gain,
                        Next::End | Next::Empty | Next::WarmDone => 0.0,
                    };
                }
                let y = match self.next(parts) {
                    Next::Sample(x) => x * self.gain,
                    Next::End => {
                        self.finish_fade(then, parts);
                        return 0.0;
                    }
                    Next::Empty | Next::WarmDone => 0.0,
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
                frame: 0,
                history: History::new(),
                end_at_wrap: false,
                warming: false,
                burst_left: 0,
                out_gain: 1.0,
                post_fade: false,
                post_left: 0,
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
        let heard_time_ns = ts.to_app_ns(ts.playback_ns);
        let dev_rate = u32::try_from(st.dev_rate).unwrap_or(u32::MAX);
        let mut first_pos: Option<u64> = None;
        let mut first_frame = 0u64;
        let mut first_off = 0usize;
        // H-46: pre-roll frames this callback may still feed the rack.
        let mut budget = (frames as u64).saturating_mul(PREROLL_SPEED);
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut done = 0;
        // T-107: priming, overrun handling and the drift servo, once per callback.
        parts.monitor.begin(ts.now_ns, frames);
        while done < frames {
            if let Mode::FadingOut { then } = st.mode
                && st.post_fade
                && st.post_left == 0
            {
                st.finish_fade(then, parts);
            }
            let mut n = (frames - done).min(st.rack_in.len());
            if st.mode == Mode::Waiting {
                st.check_ready(parts, latency);
                if st.mode == Mode::Waiting && st.quiet < latency && !parts.monitor.feeds_rack() {
                    // Draining: end the sub-block where the rack is empty, so the pre-roll
                    // follows at once.
                    n = n.min((latency - st.quiet) as usize);
                }
            }
            if st.bursting() {
                st.burst(parts, latency, &mut budget);
            }
            // H-46: still pre-rolling (budget spent, or the reader behind; also a look-ahead the
            // document end cut short): the device gets silence, and the rack waits — it is never
            // fed silence inside the stream.
            let prerolling = st.bursting();
            if st.post_fade {
                n = n.min(st.post_left as usize);
            }
            let playing = matches!(st.mode, Mode::Playing | Mode::FadingOut { .. });
            let sub_pos = playing.then_some(st.next_pos);
            if first_pos.is_none() {
                first_pos = sub_pos;
                first_frame = st.frame;
                first_off = done;
            }
            if prerolling {
                parts.monitor.render(n);
                st.rack_out[..n].fill(0.0);
            } else {
                for i in 0..n {
                    st.rack_in[i] = st.sample(parts);
                    st.frame += 1;
                }
                // T-107 (SPEC-002 §2.7): the Through-rack tap joins the playback at the rack
                // input.
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
            }
            for i in 0..n {
                // H-46: the fades after the rack (a seek's fade-out, a pre-rolled start's
                // fade-in); the meters see what is heard, never the pre-roll.
                let s = if prerolling {
                    0.0
                } else {
                    st.rack_out[i] * st.next_out_gain()
                };
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
        // ADR-002 §8: `heard_pos = p_in − L_rack`, mapped through the recent starts and loop
        // wraps (H-37) so a position heard just after a wrap is the loop end, not the play start.
        // H-46: a start mid-callback is heard `first_off` frames after the callback's first frame.
        let heard_at_ns = if first_pos.is_some() {
            heard_time_ns + frames_to_ns(first_off as u64, dev_rate)
        } else {
            heard_time_ns
        };
        let heard_pos = first_pos.map(|p| {
            if block_epoch == st.epoch {
                let h = i128::from(first_frame) - i128::from(latency);
                st.history
                    .heard(h, p, latency_doc, st.doc_rate, st.dev_rate)
            } else {
                p.saturating_sub(latency_doc)
            }
        });
        emit(
            parts,
            RtEvent::Block {
                epoch: block_epoch,
                heard_pos,
                heard_time_ns: heard_at_ns,
                latency_ns: u32::try_from(heard_time_ns.saturating_sub(ts.now_ns))
                    .unwrap_or(u32::MAX),
                frames: frames as u32,
                peak,
                sum_sq,
            },
        );
    }
}
