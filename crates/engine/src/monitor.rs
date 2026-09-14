//! Monitoring (T-107, SPEC-002 §2.7, §4.4, ADR-002 §4, §6): the input → output monitor path.
//!
//! - The input callback pushes the selected channel (input-device rate) into the **monitor
//!   ring** and then publishes a [`MonitorLink`] stamp: its total pushed count and its callback's
//!   app time, packed into one atomic (no seqlock, no lock).
//! - The output callback owns a [`MonitorOut`]: it measures the ring's **phase-compensated fill**
//!   `fill = F + r_in · (t_out − t_stamp)` (F = ring frames at the last stamp: the samples the
//!   input has captured but not yet delivered are counted as if delivery were continuous), feeds
//!   it to the PI [`DriftServo`], and pulls the ring through the adjustable-ratio
//!   [`MonitorResampler`] (rate mismatch + clock drift, ≤ ±1000 ppm). With the fill held at the
//!   target F*, every monitored sample is heard exactly
//!   `input latency + F* + output latency (+ rack latency through the rack)` after its capture —
//!   the SPEC-002 §4.4 readout — whatever the phase between the two streams' callbacks.
//! - Taps (SPEC-002 §2.7): **Dry** is added after the rack at unity gain, **Through rack** into
//!   the rack input together with any playback (one shared rack instance, ADR-002 §4; SPEC-002
//!   OD-1 default A). Mode switches, arming and disarming fade over ≤ 10 ms (smoothstep ramps).
//! - Faults (ADR-002 §6): an **underrun** fades out over the ring's last ⅓ ms, re-primes to F*
//!   and fades back in, counted in [`RtCounters::mon_underruns`] (no marker: the take never
//!   passes here). An **overrun** (fill > 4·F*) drops back to F* with a 64-sample crossfade in
//!   the ring domain, counted in [`RtCounters::mon_overruns`].
//!
//! Control side: [`StreamLatency`] tracks each stream's largest period and mean latency from the
//! RT events; [`target_frames`] and [`readout_ns`] give F* and the latency readout.
//!
//! RT contract: [`MonitorOut::command`], [`MonitorOut::begin`] and [`MonitorOut::render`] never
//! allocate, lock or block; everything is sized in [`MonitorOut::new`] on the control thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::Consumer;
use vox_dsp::async_resample::MonitorResampler;
use vox_rack::MAX_BLOCK;

use crate::drift::DriftServo;
use crate::rt::RtCounters;

/// Monitor ring capacity in input frames. ADR-002 §2 lists 8192; that cannot hold 4·F* even for
/// 1024-frame periods (F* ≈ 2096 frames at 48 kHz), so the ring is sized for the ≤ 4096-frame
/// periods the backends deliver (~1.4 s at 48 kHz, 256 KiB).
pub(crate) const MONITOR_RING_FRAMES: usize = 65_536;
/// Mode switch / arm / disarm fade (SPEC-002 §2.7: ≤ 10 ms).
pub(crate) const MONITOR_FADE_MS: f64 = 10.0;
/// Callback period assumed for F* while a stream hasn't reported one yet.
pub(crate) const UNKNOWN_PERIOD_FRAMES: u32 = 1024;
/// Safety margin in F* (ADR-002 §6: max input period + max output period + 1 ms).
const TARGET_MARGIN_S: f64 = 1e-3;
/// Underrun fade-out, taken from the ring's reserve (it must fit into F*'s 1 ms margin).
const FAULT_FADE_MS: f64 = 1.0 / 3.0;
/// Overrun crossfade length (ADR-002 §6).
const SKIP_XFADE: usize = 64;
/// Input frames of slack when checking the ring can supply a fade.
const INTERP_SLACK: usize = 2;

const TIME_BITS: u32 = 40;
const TIME_MASK: u64 = (1 << TIME_BITS) - 1;
const PUSHED_BITS: u32 = 24;
const PUSHED_MASK: u64 = (1 << PUSHED_BITS) - 1;

/// Where the monitor signal enters the output (SPEC-002 §2.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MonitorTap {
    /// Not heard.
    #[default]
    Off,
    /// After the rack, unity gain.
    Dry,
    /// Into the rack input, summed with playback.
    Rack,
}

/// The input → output stamp (see the module docs). Shared by one monitor ring's producer and
/// consumer.
#[derive(Debug, Default)]
pub(crate) struct MonitorLink {
    /// `(pushed mod 2²⁴) << 40 | (app ns of the push mod 2⁴⁰)`.
    stamp: AtomicU64,
}

impl MonitorLink {
    /// \[input callback\] Publishes the total pushed count after a push at app time `now_ns`.
    pub(crate) fn publish(&self, pushed: u64, now_ns: u64) {
        let v = ((pushed & PUSHED_MASK) << TIME_BITS) | (now_ns & TIME_MASK);
        self.stamp.store(v, Ordering::Release);
    }

    /// `(pushed mod 2²⁴, app ns mod 2⁴⁰)` of the latest push.
    fn read(&self) -> (u64, u64) {
        let v = self.stamp.load(Ordering::Acquire);
        (v >> TIME_BITS, v & TIME_MASK)
    }
}

/// Frames pushed at the stamp minus frames popped now (24-bit wrap, signed).
fn stamped_fill(pushed24: u64, popped: u64) -> i64 {
    let d = pushed24.wrapping_sub(popped) & PUSHED_MASK;
    if d >= 1 << (PUSHED_BITS - 1) {
        d as i64 - (1 << PUSHED_BITS)
    } else {
        d as i64
    }
}

/// Nanoseconds from the stamp to `now_ns` (40-bit wrap; a stamp "from the future" counts as 0).
fn stamp_age_ns(now_ns: u64, t40: u64) -> u64 {
    let d = now_ns.wrapping_sub(t40) & TIME_MASK;
    if d >= 1 << (TIME_BITS - 1) { 0 } else { d }
}

/// C¹ fade shape (smoothstep): sidebands fall off faster than a linear ramp's.
#[inline]
fn shape(p: f32) -> f32 {
    p * p * (3.0 - 2.0 * p)
}

#[inline]
fn approach(p: f32, target: f32, step: f32) -> f32 {
    if p < target {
        (p + step).min(target)
    } else {
        (p - step).max(target)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Nothing heard; the ring is trimmed to F* so a switch-on is immediate.
    Idle,
    /// Waiting for the (fresh) fill to reach F*.
    Priming,
    /// Pulling the ring through the resampler.
    Running,
}

/// The output callback's side of monitoring (see the module docs). Lives in the output
/// callback's parts, so it is dropped on the control thread.
pub(crate) struct MonitorOut {
    rx: Consumer<f32>,
    link: Arc<MonitorLink>,
    counters: Arc<RtCounters>,
    /// `None`: the resampler couldn't be built — monitoring is unavailable.
    rs: Option<MonitorResampler>,
    servo: DriftServo,
    out_rate: u32,
    /// Linked input rate (0: not linked).
    in_rate: u32,
    /// F* in input frames.
    target: f64,
    tap: MonitorTap,
    phase: Phase,
    /// Frames consumed from the ring (popped or discarded) since the ring was made.
    popped: u64,
    /// Fade-in level (linear progress, shaped on use).
    level: f32,
    level_step: f32,
    /// Tap gains (linear progress, shaped on use) and their current step.
    g_dry: f32,
    g_rack: f32,
    tap_step: f32,
    fade_step: f32,
    fade_frames: usize,
    /// Underrun fade-out length (output frames) and the input frames it needs.
    fault_frames: usize,
    reserve_in: usize,
    /// Overrun crossfade output, consumed before the ring.
    staging: [f32; SKIP_XFADE],
    staging_pos: usize,
    staging_len: usize,
    raw: Vec<f32>,
    dry: Vec<f32>,
    wet: Vec<f32>,
    /// The last measured fill (input frames), for tests.
    last_fill: f64,
}

impl MonitorOut {
    /// \[control thread\] The output side of a new monitor ring for an output at `out_rate_hz`.
    pub(crate) fn new(
        rx: Consumer<f32>,
        link: Arc<MonitorLink>,
        counters: Arc<RtCounters>,
        out_rate_hz: u32,
    ) -> Self {
        let out_rate = out_rate_hz.max(1);
        let fade_frames = (f64::from(out_rate) * MONITOR_FADE_MS / 1000.0)
            .round()
            .max(1.0) as usize;
        let fault_frames = (f64::from(out_rate) * FAULT_FADE_MS / 1000.0)
            .round()
            .max(1.0) as usize;
        let max_block = MAX_BLOCK as usize;
        Self {
            rx,
            link,
            counters,
            rs: MonitorResampler::new().ok(),
            servo: DriftServo::default(),
            out_rate,
            in_rate: 0,
            target: 1.0,
            tap: MonitorTap::Off,
            phase: Phase::Idle,
            popped: 0,
            level: 0.0,
            level_step: 1.0 / fade_frames as f32,
            g_dry: 0.0,
            g_rack: 0.0,
            tap_step: 1.0 / fade_frames as f32,
            fade_step: 1.0 / fade_frames as f32,
            fade_frames,
            fault_frames,
            reserve_in: 0,
            staging: [0.0; SKIP_XFADE],
            staging_pos: 0,
            staging_len: 0,
            raw: vec![0.0; max_block],
            dry: vec![0.0; max_block],
            wet: vec![0.0; max_block],
            last_fill: 0.0,
        }
    }

    fn staged(&self) -> usize {
        self.staging_len - self.staging_pos
    }

    /// Input frames per output frame.
    fn in_per_out(&self) -> f64 {
        f64::from(self.in_rate) / f64::from(self.out_rate)
    }

    /// Drops up to `n` frames from the head of the ring.
    fn discard(&mut self, n: usize) {
        let n = n.min(self.rx.slots());
        if n > 0
            && let Ok(chunk) = self.rx.read_chunk(n)
        {
            chunk.commit_all();
            self.popped = self.popped.wrapping_add(n as u64);
        }
    }

    /// \[output callback\] `AudioCmd::Monitor`: the tap, the linked input rate (0: unlinked),
    /// F* in input frames, and whether the input is ending (the fade then fits what is left).
    pub(crate) fn command(&mut self, tap: MonitorTap, in_rate_hz: u32, target: u32, ending: bool) {
        if in_rate_hz != self.in_rate {
            let linked = in_rate_hz > 0
                && self
                    .rs
                    .as_mut()
                    .is_some_and(|rs| rs.set_rates(in_rate_hz, self.out_rate));
            self.in_rate = if linked { in_rate_hz } else { 0 };
            self.phase = Phase::Idle;
            self.g_dry = 0.0;
            self.g_rack = 0.0;
            self.staging_len = 0;
            self.staging_pos = 0;
            self.reserve_in =
                (self.fault_frames as f64 * self.in_per_out()).ceil() as usize + INTERP_SLACK;
        }
        self.target = f64::from(target.max(1)).min((MONITOR_RING_FRAMES / 4) as f64);
        let tap = if self.in_rate == 0 {
            MonitorTap::Off
        } else {
            tap
        };
        self.tap_step = self.fade_step;
        if ending && tap == MonitorTap::Off && self.phase == Phase::Running {
            // The input stops pushing: fade out within what the ring still holds.
            let avail = (self.rx.slots() + self.staged()).saturating_sub(INTERP_SLACK);
            let frames = ((avail as f64 / self.in_per_out()) as usize).clamp(1, self.fade_frames);
            self.tap_step = 1.0 / frames as f32;
        }
        self.tap = tap;
        if tap != MonitorTap::Off && self.phase == Phase::Idle {
            self.phase = Phase::Priming;
        }
    }

    /// Whether the monitor signal currently goes into the rack (a rack reset would cut it).
    pub(crate) fn feeds_rack(&self) -> bool {
        self.phase == Phase::Running && (self.tap == MonitorTap::Rack || self.g_rack > 0.0)
    }

    /// The phase-compensated fill in input frames and whether the stamp is fresh (the input
    /// delivered within F*).
    fn fill(&self, now_ns: u64) -> (f64, bool) {
        let (pushed24, t40) = self.link.read();
        let f = stamped_fill(pushed24, self.popped) as f64;
        let age_s = stamp_age_ns(now_ns, t40) as f64 / 1e9;
        let max_age_s = self.target / f64::from(self.in_rate);
        let fresh = age_s <= max_age_s;
        // Frames the interpolator took ahead of its output are still to be heard.
        let lookahead = if self.phase == Phase::Running {
            MonitorResampler::LOOKAHEAD_FRAMES
        } else {
            0.0
        };
        let fill =
            f + f64::from(self.in_rate) * age_s.min(max_age_s) + self.staged() as f64 + lookahead;
        (fill, fresh)
    }

    /// \[output callback\] Once per callback of `frames` frames at app time `now_ns` (the
    /// output's callback time): priming, overrun handling and the servo update.
    pub(crate) fn begin(&mut self, now_ns: u64, frames: usize) {
        if self.in_rate == 0 {
            // Unlinked: nobody should push; drop anything stale.
            self.discard(self.rx.slots());
            self.phase = Phase::Idle;
            return;
        }
        let (fill, fresh) = self.fill(now_ns);
        self.last_fill = fill;
        match self.phase {
            Phase::Idle => {
                let keep = self.target.ceil() as usize;
                self.discard(self.rx.slots().saturating_sub(keep));
            }
            Phase::Priming => {
                if fresh && fill >= self.target {
                    self.discard((fill - self.target) as usize);
                    if let Some(rs) = self.rs.as_mut() {
                        rs.reset();
                    }
                    self.servo.reset();
                    self.level = 0.0;
                    self.g_dry = f32::from(self.tap == MonitorTap::Dry);
                    self.g_rack = f32::from(self.tap == MonitorTap::Rack);
                    self.phase = Phase::Running;
                    self.last_fill = self.fill(now_ns).0;
                }
            }
            Phase::Running => {
                let mut fill = fill;
                let limit = (4.0 * self.target).min(0.75 * MONITOR_RING_FRAMES as f64);
                if fill > limit && self.staged() == 0 {
                    let excess = (fill - self.target) as usize;
                    if self.skip_crossfade(excess) {
                        fill -= excess as f64;
                        self.counters.mon_overruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let err_s = (fill - self.target) / f64::from(self.in_rate);
                let dt_s = frames as f64 / f64::from(self.out_rate);
                let rel = self.servo.update(err_s, dt_s);
                if let Some(rs) = self.rs.as_mut() {
                    rs.set_correction(rel);
                }
                self.last_fill = fill;
            }
        }
    }

    /// Drops `excess` frames at the head of the ring, crossfading the first [`SKIP_XFADE`]
    /// frames of the old and the new position into the staging buffer.
    fn skip_crossfade(&mut self, excess: usize) -> bool {
        let n = excess + SKIP_XFADE;
        if excess == 0 || self.rx.slots() < n {
            return false;
        }
        let Ok(chunk) = self.rx.read_chunk(n) else {
            return false;
        };
        let (a, b) = chunk.as_slices();
        let at = |i: usize| if i < a.len() { a[i] } else { b[i - a.len()] };
        for i in 0..SKIP_XFADE {
            let w = shape((i as f32 + 0.5) / SKIP_XFADE as f32);
            self.staging[i] = at(i) * (1.0 - w) + at(excess + i) * w;
        }
        chunk.commit_all();
        self.popped = self.popped.wrapping_add(n as u64);
        self.staging_pos = 0;
        self.staging_len = SKIP_XFADE;
        true
    }

    /// Runs the resampler for `out.len()` frames (after `prepare`), feeding it from the
    /// staging buffer, then the ring.
    fn pull(&mut self, lo: usize, hi: usize) -> bool {
        let Self {
            rs,
            rx,
            staging,
            staging_pos,
            staging_len,
            popped,
            raw,
            ..
        } = self;
        let Some(rs) = rs.as_mut() else {
            return false;
        };
        rs.process(&mut raw[lo..hi], |buf| {
            let from_staging = (*staging_len - *staging_pos).min(buf.len());
            buf[..from_staging]
                .copy_from_slice(&staging[*staging_pos..*staging_pos + from_staging]);
            *staging_pos += from_staging;
            let rest = buf.len() - from_staging;
            if rest == 0 {
                return;
            }
            match rx.read_chunk(rest) {
                Ok(chunk) => {
                    let (a, b) = chunk.as_slices();
                    let k = from_staging;
                    buf[k..k + a.len()].copy_from_slice(a);
                    buf[k + a.len()..].copy_from_slice(b);
                    chunk.commit_all();
                    *popped = popped.wrapping_add(rest as u64);
                }
                Err(_) => buf[from_staging..].fill(0.0),
            }
        })
    }

    /// \[output callback\] Renders `n` (≤ `MAX_BLOCK`) monitor frames; read them with
    /// [`Self::dry`] and [`Self::wet`].
    pub(crate) fn render(&mut self, n: usize) {
        let n = n.min(self.raw.len());
        let mut i = 0;
        let mut faded_out = false;
        // Gains apply to the whole block if it started running (it may end idle or priming
        // part-way: the samples produced before that are still heard).
        let was_running = self.phase == Phase::Running;
        if was_running && self.rs.is_some() {
            while i < n {
                let piece = (n - i).min(MonitorResampler::MAX_CHUNK);
                let need = self.rs.as_mut().map_or(usize::MAX, |rs| rs.prepare(piece));
                let avail = self.rx.slots() + self.staged();
                let reserve = if self.tap == MonitorTap::Off {
                    0
                } else {
                    self.reserve_in
                };
                if avail >= need.saturating_add(reserve) {
                    self.pull(i, i + piece);
                    i += piece;
                    continue;
                }
                if self.tap == MonitorTap::Off {
                    // Fading out while the input ended: play what is left (a shorter piece),
                    // then go idle.
                    let can =
                        (avail.saturating_sub(INTERP_SLACK) as f64 / self.in_per_out()) as usize;
                    let short = can.min(piece);
                    if short > 0 {
                        let need_s = self.rs.as_mut().map_or(usize::MAX, |rs| rs.prepare(short));
                        if avail >= need_s && self.pull(i, i + short) {
                            i += short;
                            continue;
                        }
                    }
                    self.phase = Phase::Idle;
                    break;
                }
                // Underrun: fade out over what is left, then re-prime.
                let can = (avail.saturating_sub(INTERP_SLACK) as f64 / self.in_per_out()) as usize;
                let f = self.fault_frames.min(can).min(n - i);
                if f > 0 {
                    let need_f = self.rs.as_mut().map_or(usize::MAX, |rs| rs.prepare(f));
                    if avail >= need_f && self.pull(i, i + f) {
                        for (j, x) in self.raw[i..i + f].iter_mut().enumerate() {
                            *x *= shape(1.0 - (j + 1) as f32 / f as f32);
                        }
                        i += f;
                    }
                }
                self.counters.mon_underruns.fetch_add(1, Ordering::Relaxed);
                self.phase = Phase::Priming;
                faded_out = true;
                break;
            }
        }
        self.raw[i..n].fill(0.0);
        let (td, tr) = (
            f32::from(self.tap == MonitorTap::Dry),
            f32::from(self.tap == MonitorTap::Rack),
        );
        if !was_running {
            self.g_dry = td;
            self.g_rack = tr;
            self.dry[..n].fill(0.0);
            self.wet[..n].fill(0.0);
            return;
        }
        for j in 0..n {
            self.level = (self.level + self.level_step).min(1.0);
            self.g_dry = approach(self.g_dry, td, self.tap_step);
            self.g_rack = approach(self.g_rack, tr, self.tap_step);
            let x = self.raw[j] * shape(self.level);
            self.dry[j] = x * shape(self.g_dry);
            self.wet[j] = x * shape(self.g_rack);
        }
        if faded_out {
            self.level = 0.0;
        }
        if self.phase == Phase::Running
            && self.tap == MonitorTap::Off
            && self.g_dry == 0.0
            && self.g_rack == 0.0
        {
            self.phase = Phase::Idle;
        }
    }

    /// The Dry tap of the last [`Self::render`] (added after the rack).
    pub(crate) fn dry(&self) -> &[f32] {
        &self.dry
    }

    /// The Through-rack tap of the last [`Self::render`] (added to the rack input).
    pub(crate) fn wet(&self) -> &[f32] {
        &self.wet
    }

    /// The last measured fill (input frames).
    #[cfg(test)]
    fn last_fill(&self) -> f64 {
        self.last_fill
    }
}

// --- Control side -----------------------------------------------------------------------------

/// One stream direction's observed periods and latency (SPEC-002 §4.4), from the RT events.
#[derive(Clone, Debug, Default)]
pub(crate) struct StreamLatency {
    /// Largest callback period seen (frames); the nominal one until the first callback.
    max_frames: u32,
    /// Mean device latency (ns): input `callback − capture end`, output `playback − callback`.
    latency_ns: f64,
    measured: bool,
}

impl StreamLatency {
    /// A newly opened stream with its nominal period (`None`: varies/unknown).
    pub(crate) fn reset(&mut self, nominal_frames: Option<u32>) {
        *self = Self {
            max_frames: nominal_frames.unwrap_or(0),
            ..Self::default()
        };
    }

    /// One callback of `frames` frames with latency `latency_ns`. Returns whether the largest
    /// period grew (F* changes).
    pub(crate) fn observe(&mut self, frames: u32, latency_ns: u32) -> bool {
        let grew = frames > self.max_frames;
        self.max_frames = self.max_frames.max(frames);
        if self.measured {
            self.latency_ns += 0.05 * (f64::from(latency_ns) - self.latency_ns);
        } else {
            self.latency_ns = f64::from(latency_ns);
            self.measured = true;
        }
        grew
    }

    /// Whether at least one callback was observed.
    pub(crate) fn measured(&self) -> bool {
        self.measured
    }

    fn period_s(&self, rate_hz: u32) -> f64 {
        let frames = if self.max_frames == 0 {
            UNKNOWN_PERIOD_FRAMES
        } else {
            self.max_frames
        };
        f64::from(frames) / f64::from(rate_hz.max(1))
    }
}

/// F* in seconds: max input period + max output period + 1 ms (ADR-002 §6).
fn target_s(
    input: &StreamLatency,
    in_rate_hz: u32,
    output: &StreamLatency,
    out_rate_hz: u32,
) -> f64 {
    input.period_s(in_rate_hz) + output.period_s(out_rate_hz) + TARGET_MARGIN_S
}

/// F* in input frames (what `AudioCmd::Monitor` carries).
pub(crate) fn target_frames(
    input: &StreamLatency,
    in_rate_hz: u32,
    output: &StreamLatency,
    out_rate_hz: u32,
) -> u32 {
    // Rounding noise must not add a frame (256 + 256 + 48 is exactly 560).
    (target_s(input, in_rate_hz, output, out_rate_hz) * f64::from(in_rate_hz) - 1e-6).ceil() as u32
}

/// The monitoring latency readout (SPEC-002 §2.7, §4.4): input latency + F* + rack latency
/// (`rack_samples` at the output rate, through-rack only — pass 0 for Dry) + output latency, in
/// ns. The input latency is `callback − capture` of the block's **end** (the device's own
/// latency; the period is F*'s part), which is how §4.4's example counts it (256-frame periods,
/// 5 ms in, 7 ms out → 23.7 ms).
pub(crate) fn readout_ns(
    input: &StreamLatency,
    in_rate_hz: u32,
    output: &StreamLatency,
    out_rate_hz: u32,
    rack_samples: u32,
) -> u64 {
    let s = input.latency_ns / 1e9
        + target_s(input, in_rate_hz, output, out_rate_hz)
        + f64::from(rack_samples) / f64::from(out_rate_hz.max(1))
        + output.latency_ns / 1e9;
    (s * 1e9).round().max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::{Producer, RingBuffer};
    use vox_module_api::test_util::no_alloc;

    const RATE: u32 = 48_000;

    struct Rig {
        tx: Producer<f32>,
        link: Arc<MonitorLink>,
        counters: Arc<RtCounters>,
        out: MonitorOut,
        pushed: u64,
        next_in: u64,
    }

    fn rig(in_rate: u32, target: u32, tap: MonitorTap) -> Rig {
        let (tx, rx) = RingBuffer::new(MONITOR_RING_FRAMES);
        let link = Arc::new(MonitorLink::default());
        let counters = Arc::new(RtCounters::default());
        let mut out = MonitorOut::new(rx, link.clone(), counters.clone(), RATE);
        out.command(tap, in_rate, target, false);
        Rig {
            tx,
            link,
            counters,
            out,
            pushed: 0,
            next_in: 0,
        }
    }

    impl Rig {
        /// The input delivers `n` frames of `f(k)` at app time `now_ns`.
        fn push(&mut self, n: usize, now_ns: u64, f: impl Fn(u64) -> f32) {
            for _ in 0..n {
                self.tx.push(f(self.next_in)).unwrap();
                self.next_in += 1;
            }
            self.pushed += n as u64;
            self.link.publish(self.pushed, now_ns);
        }

        /// One output callback of `n` frames at `now_ns`; returns (dry, wet).
        fn pull(&mut self, n: usize, now_ns: u64) -> (Vec<f32>, Vec<f32>) {
            let out = &mut self.out;
            no_alloc(|| {
                out.begin(now_ns, n);
                out.render(n);
            })
            .expect("the monitor path allocated");
            (self.out.dry()[..n].to_vec(), self.out.wet()[..n].to_vec())
        }
    }

    const MS: u64 = 1_000_000;

    #[test]
    fn stamp_packs_and_wraps() {
        let link = MonitorLink::default();
        link.publish((1 << 24) + 5, (1 << 40) + 7);
        let (p, t) = link.read();
        assert_eq!((p, t), (5, 7));
        assert_eq!(stamped_fill(5, (1 << 24) + 2), 3);
        assert_eq!(stamped_fill(2, 5), -3);
        assert_eq!(stamp_age_ns((1 << 40) + 10, 7), 3);
        assert_eq!(
            stamp_age_ns(5, 7),
            0,
            "a stamp from the future is not negative"
        );
    }

    /// The fill counts the frames captured since the last delivery (phase compensation).
    #[test]
    fn fill_is_phase_compensated() {
        let mut r = rig(RATE, 500, MonitorTap::Dry);
        r.push(600, 100 * MS, |_| 0.0);
        let (fill, fresh) = r.out.fill(101 * MS);
        assert!(fresh);
        assert!((fill - 648.0).abs() < 1e-6, "{fill}");
        // Older than F*: stale, and the compensation is capped.
        let (fill, fresh) = r.out.fill(200 * MS);
        assert!(!fresh);
        assert!((fill - 1100.0).abs() < 1e-6, "{fill}");
    }

    /// Primes to F*, then the Dry tap carries the input at unity gain after a smooth fade-in; the
    /// Rack tap stays silent. No allocation.
    #[test]
    fn primes_then_runs_dry() {
        let mut r = rig(RATE, 560, MonitorTap::Dry);
        let dc = |_| 0.5f32;
        let mut t = 0;
        let mut dry = Vec::new();
        for _ in 0..40 {
            r.push(256, t, dc);
            let (d, w) = r.pull(256, t + 2 * MS);
            assert!(w.iter().all(|&x| x == 0.0));
            dry.extend(d);
            t += 256 * 1_000_000_000 / u64::from(RATE);
        }
        let first = dry.iter().position(|&x| x != 0.0).expect("heard");
        // A 10 ms fade-in, then unity.
        assert!(dry[first] < 0.01);
        assert!(
            dry[first + 480..].iter().all(|&x| (x - 0.5).abs() < 1e-4),
            "unity after the fade"
        );
        let max_step = dry
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.5 * 1.6 / 480.0, "fade step {max_step}");
        assert_eq!(r.counters.mon_underruns.load(Ordering::Relaxed), 0);
        // The fill is held at F*.
        assert!(
            (r.out.last_fill() - 560.0).abs() < 3.0,
            "{}",
            r.out.last_fill()
        );
    }

    /// An underrun fades out (no step larger than the ⅓ ms fade's), counts once, re-primes and
    /// fades back in.
    #[test]
    fn underrun_fades_out_counts_and_reprimes() {
        let mut r = rig(RATE, 560, MonitorTap::Dry);
        let dc = |_| 0.5f32;
        let period = 256 * 1_000_000_000 / u64::from(RATE);
        let mut t = 0;
        let mut dry = Vec::new();
        for i in 0..60 {
            // The input stalls for 10 callbacks.
            if !(20..30).contains(&i) {
                r.push(256, t, dc);
            }
            dry.extend(r.pull(256, t + 2 * MS).0);
            t += period;
        }
        assert_eq!(r.counters.mon_underruns.load(Ordering::Relaxed), 1);
        let max_step = dry
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.5 * 1.6 / 16.0, "underrun fade step {max_step}");
        // Silent gap, then heard again.
        let last_heard = dry.iter().rposition(|&x| (x - 0.5).abs() < 1e-4).unwrap();
        assert!(last_heard > 50 * 256, "monitoring resumed");
    }

    /// An overrun (fill > 4·F*) drops back to F* with a crossfade, counted once.
    #[test]
    fn overrun_skips_back_to_the_target() {
        let mut r = rig(RATE, 560, MonitorTap::Dry);
        let ramp = |k: u64| (k % 48_000) as f32 / 48_000.0;
        let period = 256 * 1_000_000_000 / u64::from(RATE);
        let mut t = 0;
        for _ in 0..20 {
            r.push(256, t, ramp);
            r.pull(256, t + 2 * MS);
            t += period;
        }
        // The output stalls while the input delivers 4000 frames more.
        r.push(4_000, t, ramp);
        let (d, _) = r.pull(256, t + 2 * MS);
        assert_eq!(r.counters.mon_overruns.load(Ordering::Relaxed), 1);
        assert!(d.iter().all(|x| x.is_finite()));
        assert!(
            (r.out.last_fill() - 560.0).abs() < 2.0,
            "{}",
            r.out.last_fill()
        );
    }

    /// Dry → Through rack crossfades over ≤ 10 ms; → Off fades out and goes idle.
    #[test]
    fn tap_switches_fade() {
        let mut r = rig(RATE, 560, MonitorTap::Dry);
        let dc = |_| 0.5f32;
        let period = 256 * 1_000_000_000 / u64::from(RATE);
        let mut t = 0;
        for _ in 0..20 {
            r.push(256, t, dc);
            r.pull(256, t + 2 * MS);
            t += period;
        }
        r.out.command(MonitorTap::Rack, RATE, 560, false);
        let (mut dry, mut wet) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            r.push(256, t, dc);
            let (d, w) = r.pull(256, t + 2 * MS);
            dry.extend(d);
            wet.extend(w);
            t += period;
        }
        assert!(r.out.feeds_rack());
        assert!(
            dry[480..].iter().all(|&x| x == 0.0),
            "dry faded out within 10 ms"
        );
        assert!(
            wet[480..].iter().all(|&x| (x - 0.5).abs() < 1e-4),
            "wet faded in"
        );
        for (d, w) in dry.iter().zip(&wet).take(480) {
            assert!(
                (d + w - 0.5).abs() < 0.26,
                "no dip deeper than the crossfade's"
            );
        }
        r.out.command(MonitorTap::Off, RATE, 560, false);
        for _ in 0..3 {
            r.push(256, t, dc);
            let (_, w) = r.pull(256, t + 2 * MS);
            wet = w;
            t += period;
        }
        assert!(wet.iter().all(|&x| x == 0.0));
        assert!(!r.out.feeds_rack());
        assert_eq!(r.out.phase, Phase::Idle);
    }

    /// Disarming (the input ends): the fade fits what the ring still holds — no underrun.
    #[test]
    fn ending_input_fades_within_the_ring() {
        let mut r = rig(RATE, 400, MonitorTap::Dry);
        let dc = |_| 0.5f32;
        let period = 128 * 1_000_000_000 / u64::from(RATE);
        let mut t = 0;
        for _ in 0..40 {
            r.push(128, t, dc);
            r.pull(128, t + MS);
            t += period;
        }
        r.out.command(MonitorTap::Off, RATE, 400, true);
        let mut dry = Vec::new();
        for _ in 0..10 {
            dry.extend(r.pull(128, t + MS).0);
            t += period;
        }
        assert_eq!(r.counters.mon_underruns.load(Ordering::Relaxed), 0);
        assert!(dry.last() == Some(&0.0));
        // The ring held ≈ 4.5 ms (F* minus one period): the fade uses all of it, never cut.
        let max_step = dry
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.5 * 1.55 / 200.0, "ending fade step {max_step}");
    }

    /// A mismatched input rate is resampled: 44.1 kHz in, 48 kHz out keeps the fill at F*
    /// (441 + 256·44.1/48 + 44.1 ≈ 721 input frames).
    #[test]
    fn rate_mismatch_holds_the_fill() {
        let mut r = rig(44_100, 721, MonitorTap::Dry);
        let tone = |k: u64| (0.3 * (k as f64 * 0.05).sin()) as f32;
        // Input: 441-frame blocks every 10 ms; output: 256-frame blocks every 5.33 ms.
        let mut t_in = 0u64;
        let mut t_out = 0u64;
        for _ in 0..2_000 {
            if t_in <= t_out {
                r.push(441, t_in, tone);
                t_in += 10 * MS;
            } else {
                r.pull(256, t_out);
                t_out += 256 * 1_000_000_000 / u64::from(RATE);
            }
        }
        assert_eq!(r.counters.mon_underruns.load(Ordering::Relaxed), 0);
        assert!(
            (r.out.last_fill() - 721.0).abs() < 5.0,
            "{}",
            r.out.last_fill()
        );
    }

    /// SPEC-002 §4.4 example: 256-frame periods at 48 kHz, 5 ms in, 7 ms out → 23.7 ms; the rack
    /// adds its latency; F* = 256 + 256 + 48 frames.
    #[test]
    fn readout_matches_the_spec_example() {
        let mut i = StreamLatency::default();
        let mut o = StreamLatency::default();
        i.reset(Some(256));
        o.reset(Some(256));
        assert!(!i.observe(256, 5_000_000));
        assert!(!o.observe(256, 7_000_000));
        assert_eq!(target_frames(&i, RATE, &o, RATE), 560);
        let ns = readout_ns(&i, RATE, &o, RATE, 0);
        assert!((ns as f64 / 1e6 - 23.6667).abs() < 0.001, "{ns}");
        let with_rack = readout_ns(&i, RATE, &o, RATE, 480);
        assert_eq!(with_rack - ns, 10_000_000);
        // A larger period raises F*.
        assert!(i.observe(512, 5_000_000));
        assert_eq!(target_frames(&i, RATE, &o, RATE), 816);
    }
}
