//! The output callback (ADR-002 §4), the transport clock. Per device buffer it drains control
//! commands, pulls current-epoch packets from the playback ring (≈5 ms fades on start, stop and
//! seek), runs the live rack in sub-blocks of at most `MAX_BLOCK`, writes the mono result to
//! every device channel, meters it and reports one [`RtEvent::Block`] (heard position, heard
//! time, peak, energy) to the control thread.
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

use crate::backend::{OutputCallback, OutputTimestamp};
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
    /// Waiting for the prebuffer of `epoch`.
    Waiting,
    /// Playing (fading in while `gain < 1`).
    Playing,
    /// Fading out; then wait for `next` (a seek) or go idle.
    FadingOut { next: Option<(u32, u64)> },
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
    underrun: bool,
    rack_in: Vec<f32>,
    rack_out: Vec<f32>,
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

    fn command(&mut self, cmd: AudioCmd, parts: &mut OutputParts) {
        match cmd {
            AudioCmd::Play { epoch, pos, reset } => {
                self.cmd_epoch = epoch;
                self.begin_wait(epoch, pos, reset);
            }
            AudioCmd::Seek { epoch, pos } => {
                self.cmd_epoch = epoch;
                match self.mode {
                    Mode::Playing | Mode::FadingOut { .. } => {
                        self.mode = Mode::FadingOut {
                            next: Some((epoch, pos)),
                        };
                    }
                    Mode::Waiting | Mode::Idle => self.begin_wait(epoch, pos, true),
                }
            }
            AudioCmd::Stop => match self.mode {
                Mode::Playing | Mode::FadingOut { .. } => {
                    self.mode = Mode::FadingOut { next: None }
                }
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
        }
    }

    /// Waiting: drops stale packets at the head, then starts once the prebuffer (or the document
    /// end) of the current epoch is queued.
    fn check_ready(&mut self, parts: &mut OutputParts) {
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
                parts.live.reset();
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
                Ok(_) if flags & packet_flags::END != 0 => {
                    self.has_cur = false;
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

    fn end_reached(&mut self, parts: &mut OutputParts) {
        self.mode = Mode::Idle;
        self.has_cur = false;
        emit(
            parts,
            RtEvent::Ended {
                epoch: self.epoch,
                pos: self.next_pos,
            },
        );
    }

    fn sample(&mut self, parts: &mut OutputParts) -> f32 {
        match self.mode {
            Mode::Idle | Mode::Waiting => 0.0,
            Mode::Playing => match self.next(parts) {
                Next::Sample(x) => {
                    let y = x * self.gain;
                    if self.gain < 1.0 {
                        self.gain = (self.gain + self.fade_step).min(1.0);
                    }
                    y
                }
                Next::End => {
                    self.end_reached(parts);
                    0.0
                }
                Next::Empty => {
                    self.underrun = true;
                    0.0
                }
            },
            Mode::FadingOut { next } => {
                let y = match self.next(parts) {
                    Next::Sample(x) => x * self.gain,
                    Next::End => {
                        self.end_reached(parts);
                        return 0.0;
                    }
                    Next::Empty => 0.0,
                };
                self.gain -= self.fade_step;
                if self.gain <= 0.0 {
                    self.gain = 0.0;
                    match next {
                        Some((epoch, pos)) => self.begin_wait(epoch, pos, true),
                        None => {
                            self.mode = Mode::Idle;
                            self.has_cur = false;
                            emit(
                                parts,
                                RtEvent::Stopped {
                                    epoch: self.cmd_epoch,
                                    pos: self.next_pos,
                                },
                            );
                        }
                    }
                }
                y
            }
        }
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
                underrun: false,
                rack_in: vec![0.0; max_block],
                rack_out: vec![0.0; max_block],
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
        let mut first_pos: Option<u64> = None;
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(st.rack_in.len());
            if st.mode == Mode::Waiting {
                st.check_ready(parts);
            }
            let playing = matches!(st.mode, Mode::Playing | Mode::FadingOut { .. });
            let sub_pos = playing.then_some(st.next_pos);
            if first_pos.is_none() {
                first_pos = sub_pos;
            }
            for i in 0..n {
                st.rack_in[i] = st.sample(parts);
            }
            let transport = Transport {
                playing,
                position_samples: sub_pos,
            };
            parts
                .live
                .process(transport, &st.rack_in[..n], &mut st.rack_out[..n]);
            for (i, &s) in st.rack_out[..n].iter().enumerate() {
                peak = peak.max(s.abs());
                sum_sq += f64::from(s) * f64::from(s);
                let base = (done + i) * channels;
                data[base..base + channels].fill(s);
            }
            done += n;
        }
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
                heard_time_ns: ts.to_app_ns(ts.playback_ns),
                frames: frames as u32,
                peak,
                sum_sq,
            },
        );
    }
}
