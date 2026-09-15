//! Telemetry frames (ADR-003 §2 `VXTM`): playhead anchor + output meter, built by the control
//! thread at the telemetry rate (60 Hz, ADR-009). The engine hands [`TelemetryFrame`]s to a sink;
//! the app encodes them with [`TelemetryFrame::encode`] and sends them over a Tauri `Channel`.

/// Encoded `VXTM` frame length (header only, version 1).
pub const VXTM_LEN: usize = 72;

/// `VXTM` flag bits (ADR-003 §2).
pub mod vxtm_flags {
    /// Playback is running.
    pub const PLAYING: u32 = 1 << 0;
    /// Recording.
    pub const RECORDING: u32 = 1 << 1;
    /// Monitoring.
    pub const MONITORING: u32 = 1 << 2;
    /// Loop playback.
    pub const LOOPING: u32 = 1 << 3;
    /// An underrun/xrun since the previous frame.
    pub const XRUN: u32 = 1 << 4;
    /// Output peak ≥ 0 dBFS since the previous frame.
    pub const OUT_CLIP: u32 = 1 << 5;
    /// Input peak ≥ 0 dBFS since the previous frame.
    pub const IN_CLIP: u32 = 1 << 6;
}

/// One telemetry frame. Meter values are max-hold (peak) / energy average (RMS) since the
/// previous frame; `-inf` = digital silence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TelemetryFrame {
    /// Frame counter.
    pub seq: u32,
    /// [`vxtm_flags`] bits.
    pub flags: u32,
    /// Heard position (document samples) at `playhead_time_ns`.
    pub playhead_sample: u64,
    /// App-clock time of `playhead_sample`.
    pub playhead_time_ns: u64,
    /// Document samples per second while playing, 0 when stopped.
    pub rate: f64,
    /// Output peak since the previous frame (post-rack).
    pub out_peak_dbfs: f32,
    /// Output RMS over the audio since the previous frame.
    pub out_rms_dbfs: f32,
    /// Input peak (no input yet: `-inf`).
    pub in_peak_dbfs: f32,
    /// Input RMS (no input yet: `-inf`).
    pub in_rms_dbfs: f32,
    /// `audio_rev` of the playing document (0: none).
    pub audio_rev: u64,
    /// RT events dropped so far (full rings).
    pub dropped_rt_events: u32,
}

impl TelemetryFrame {
    /// Little-endian `VXTM` v1 encoding (ADR-003 §2).
    pub fn encode(&self) -> [u8; VXTM_LEN] {
        let mut b = [0u8; VXTM_LEN];
        b[0..4].copy_from_slice(b"VXTM");
        b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[6..8].copy_from_slice(&(VXTM_LEN as u16).to_le_bytes());
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b[12..16].copy_from_slice(&self.flags.to_le_bytes());
        b[16..24].copy_from_slice(&self.playhead_sample.to_le_bytes());
        b[24..32].copy_from_slice(&self.playhead_time_ns.to_le_bytes());
        b[32..40].copy_from_slice(&self.rate.to_le_bytes());
        b[40..44].copy_from_slice(&self.out_peak_dbfs.to_le_bytes());
        b[44..48].copy_from_slice(&self.out_rms_dbfs.to_le_bytes());
        b[48..52].copy_from_slice(&self.in_peak_dbfs.to_le_bytes());
        b[52..56].copy_from_slice(&self.in_rms_dbfs.to_le_bytes());
        b[56..64].copy_from_slice(&self.audio_rev.to_le_bytes());
        b[64..68].copy_from_slice(&self.dropped_rt_events.to_le_bytes());
        b
    }
}

/// Receives telemetry frames on the control thread (must not block for long).
pub type TelemetrySink = Box<dyn FnMut(&TelemetryFrame) + Send>;

/// Clamps an arbitrary `Settings.telemetry_rate_hz` to the two valid values (H-16, SPEC-003 §3 /
/// ADR-009: only 60 and 30 Hz are settable), falling back to the nearer one rather than panicking
/// on bad input. Used both for the tick divisor below and for the rate actually handed to the
/// analyzer's EMA time constants ([`crate::analyzer::AnalyzerPublisher::set_rate_hz`]), so the two
/// always agree on what rate is really in effect.
pub(crate) fn effective_telemetry_rate_hz(rate_hz: u32) -> u32 {
    if rate_hz <= 30 { 30 } else { 60 }
}

/// Control-tick divisor for `Settings.telemetry_rate_hz`: only 60 (every control tick) and 30
/// (every other tick) Hz are valid settings, so a plain tick counter is enough — no need for a
/// wall-clock accumulator.
fn rate_divisor(rate_hz: u32) -> u32 {
    60 / effective_telemetry_rate_hz(rate_hz)
}

/// Publish-cadence gate shared by the `VXTM`/`VXMT`/`VXSA` publishers (H-16): the control tick
/// itself always runs at a fixed 60 Hz (`control::TICK`), but the telemetry rate setting can halve
/// how often frames actually go out without touching that tick. Call [`Self::due`] once per
/// control tick; it returns whether this tick should publish.
pub(crate) struct TelemetryRateGate {
    /// Ticks between publishes (1 at 60 Hz, 2 at 30 Hz).
    divisor: u32,
    counter: u32,
}

impl TelemetryRateGate {
    pub(crate) fn new(rate_hz: u32) -> Self {
        Self {
            divisor: rate_divisor(rate_hz),
            counter: 0,
        }
    }

    /// Changes the configured rate, effective immediately: the phase resets so the new rate
    /// starts publishing right away instead of finishing out a stale offset.
    pub(crate) fn set_rate_hz(&mut self, rate_hz: u32) {
        self.divisor = rate_divisor(rate_hz);
        self.counter = 0;
    }

    /// Call once per control tick; `true` on ticks that should publish.
    pub(crate) fn due(&mut self) -> bool {
        self.counter += 1;
        if self.counter >= self.divisor {
            self.counter = 0;
            true
        } else {
            false
        }
    }
}

/// Linear amplitude → dBFS (`-inf` for 0).
pub fn to_dbfs(x: f64) -> f32 {
    if x > 0.0 {
        (20.0 * x.log10()) as f32
    } else {
        f32::NEG_INFINITY
    }
}

/// RMS window shared by every meter (SPEC-002 §3 `meter_rms_window_ms`; ADR-003's `VXTM` table
/// says the output meter's RMS is a "window per meter spec" without giving its own number, so
/// this reuses the input meter's fixed 300 ms convention — H-41).
pub(crate) const METER_RMS_WINDOW_MS: u64 = 300;

/// Backwards-compatible alias (input meter call sites predate the output meter reusing the same
/// window).
pub(crate) const INPUT_RMS_WINDOW_MS: u64 = METER_RMS_WINDOW_MS;

/// Output meter aggregation between two frames (H-41: peak is a true max-hold since the last
/// `take()`, reset on every call; RMS is a proper sliding rectangular window over
/// [`METER_RMS_WINDOW_MS`] of audio — *not* reset on `take()`, or it would only ever average the
/// ~17/33 ms between two 60/30 Hz telemetry frames, which is short enough to read as noise/
/// flicker rather than a level. The window itself persists across `take()` calls exactly like
/// [`InputMeter`]'s).
#[derive(Debug, Default)]
pub(crate) struct Meter {
    peak: f32,
    /// Rolling window of `(frames, sum_sq)` chunks, oldest first (mirrors [`InputMeter::window`]).
    window: std::collections::VecDeque<(u32, f64)>,
    frames: u64,
    sum_sq: f64,
    window_frames: u64,
}

impl Meter {
    /// Clears the meter for a stream at `rate_hz` (0: no stream — the window never accepts data
    /// until a real rate is set, see `add`'s doc comment).
    pub(crate) fn reset(&mut self, rate_hz: u32) {
        *self = Self {
            window_frames: u64::from(rate_hz) * METER_RMS_WINDOW_MS / 1000,
            ..Self::default()
        };
    }

    /// Folds one output block's `(peak, sum_sq, frames)` into the meter. Before the first
    /// [`Self::reset`] (`window_frames == 0`) every chunk is evicted as soon as it's added, so the
    /// RMS reads silence rather than accumulating against an unknown window size.
    pub(crate) fn add(&mut self, peak: f32, sum_sq: f64, frames: u32) {
        self.peak = self.peak.max(peak);
        self.window.push_back((frames, sum_sq));
        self.frames += u64::from(frames);
        self.sum_sq += sum_sq;
        while let Some(&(f, s)) = self.window.front() {
            if self.frames - u64::from(f) < self.window_frames {
                break;
            }
            self.window.pop_front();
            self.frames -= u64::from(f);
            self.sum_sq = (self.sum_sq - s).max(0.0);
        }
    }

    /// `(peak, peak dBFS, windowed RMS dBFS)` — `peak` is the max-hold since the last call and
    /// resets here; the RMS window is unaffected by `take()` and keeps sliding.
    pub(crate) fn take(&mut self) -> (f32, f32, f32) {
        let peak = self.peak;
        self.peak = 0.0;
        let rms = if self.frames > 0 {
            (self.sum_sq / self.frames as f64).sqrt()
        } else {
            0.0
        };
        (peak, to_dbfs(f64::from(peak)), to_dbfs(rms))
    }
}

/// Input meter (SPEC-002 §2.1): peak max-hold since the previous frame, unweighted RMS over a
/// sliding rectangular window of ≥ 300 ms (block granularity), and the clip flag.
#[derive(Debug, Default)]
pub(crate) struct InputMeter {
    peak: f32,
    /// A block arrived since the last frame (otherwise the last frame's peak is held: input
    /// blocks can be longer than a telemetry period).
    fresh: bool,
    last_peak: f32,
    clip: bool,
    window: std::collections::VecDeque<(u32, f64)>,
    frames: u64,
    sum_sq: f64,
    window_frames: u64,
}

impl InputMeter {
    /// Clears the meter for a stream at `rate_hz`.
    pub(crate) fn reset(&mut self, rate_hz: u32) {
        *self = Self {
            window_frames: u64::from(rate_hz) * INPUT_RMS_WINDOW_MS / 1000,
            ..Self::default()
        };
    }

    pub(crate) fn add(&mut self, frames: u32, peak: f32, sum_sq: f64, clipped: bool) {
        self.peak = self.peak.max(peak);
        self.fresh = true;
        self.clip |= clipped;
        self.window.push_back((frames, sum_sq));
        self.frames += u64::from(frames);
        self.sum_sq += sum_sq;
        while let Some(&(f, s)) = self.window.front() {
            if self.frames - u64::from(f) < self.window_frames {
                break;
            }
            self.window.pop_front();
            self.frames -= u64::from(f);
            self.sum_sq = (self.sum_sq - s).max(0.0);
        }
    }

    /// `(peak dBFS since the last call — held when no block arrived —, window RMS dBFS, clipped
    /// since the last call)`.
    pub(crate) fn take(&mut self) -> (f32, f32, bool) {
        let rms = if self.frames > 0 {
            (self.sum_sq / self.frames as f64).sqrt()
        } else {
            0.0
        };
        if self.fresh {
            self.last_peak = self.peak;
        }
        let out = (to_dbfs(f64::from(self.last_peak)), to_dbfs(rms), self.clip);
        self.peak = 0.0;
        self.fresh = false;
        self.clip = false;
        out
    }
}

/// Header length of a `VXMT` frame (SPEC-016 §4.12, version 1).
pub const VXMT_HEADER_LEN: usize = 32;

/// One slot's values in a [`ModuleTelemetryFrame`].
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleTelemetryRecord {
    /// The slot's stable id (the rack DTO's `uid`; encoded as `u32`).
    pub slot_uid: u64,
    /// One value per telemetry channel, in the slot's channel order (Min/Max channels: the
    /// extreme since the previous frame, e.g. the deepest gain reduction).
    pub values: Vec<f32>,
}

/// Module telemetry (`VXMT`, SPEC-016 §4.12): the values of every rack slot with a `Telemetry`
/// extension (e.g. the true-peak limiter's gain reduction), built by the control thread at the
/// telemetry rate.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleTelemetryFrame {
    /// Frame counter (restarts at 0 for a new subscriber).
    pub seq: u32,
    /// App-clock time of the read.
    pub frame_time_ns: u64,
    /// One record per slot with telemetry, rack order.
    pub records: Vec<ModuleTelemetryRecord>,
}

impl ModuleTelemetryFrame {
    /// Little-endian `VXMT` v1 encoding: a 32-byte header (`"VXMT"`, version 1, header_len 32,
    /// seq, flags 0, frame_time_ns, record count, reserved 0), then per record
    /// `{u32 slot_uid, u16 count, u16 reserved, f32[count]}`.
    pub fn encode(&self) -> Vec<u8> {
        let body: usize = self.records.iter().map(|r| 8 + 4 * r.values.len()).sum();
        let mut b = Vec::with_capacity(VXMT_HEADER_LEN + body);
        b.extend_from_slice(b"VXMT");
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&(VXMT_HEADER_LEN as u16).to_le_bytes());
        b.extend_from_slice(&self.seq.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&self.frame_time_ns.to_le_bytes());
        b.extend_from_slice(
            &u32::try_from(self.records.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        b.extend_from_slice(&0u32.to_le_bytes());
        for r in &self.records {
            let count = u16::try_from(r.values.len()).unwrap_or(u16::MAX);
            b.extend_from_slice(&u32::try_from(r.slot_uid).unwrap_or(u32::MAX).to_le_bytes());
            b.extend_from_slice(&count.to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
            for v in r.values.iter().take(usize::from(count)) {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b
    }
}

/// Receives module-telemetry frames on the control thread (must not block for long).
pub type ModuleTelemetrySink = Box<dyn FnMut(&ModuleTelemetryFrame) + Send>;

/// Publishes module telemetry at the telemetry rate: the single reader of every slot's
/// `Telemetry` handle (ADR-005 §11). Nothing is read while nobody subscribes.
#[derive(Default)]
pub(crate) struct ModuleTelemetryPublisher {
    sink: Option<ModuleTelemetrySink>,
    seq: u32,
}

impl ModuleTelemetryPublisher {
    pub(crate) fn set_sink(&mut self, sink: Option<ModuleTelemetrySink>) {
        self.sink = sink;
        self.seq = 0;
    }

    /// One frame from `rack`'s slots, only while a subscriber exists and at least one slot has
    /// the extension (SPEC-016 §4.12).
    pub(crate) fn publish(&mut self, rack: Option<&vox_rack::RackHost>, now_ns: u64) {
        let (Some(sink), Some(rack)) = (self.sink.as_mut(), rack) else {
            return;
        };
        let records: Vec<ModuleTelemetryRecord> = rack
            .read_telemetry()
            .into_iter()
            .map(|s| ModuleTelemetryRecord {
                slot_uid: s.uid.0,
                values: s.values,
            })
            .collect();
        if records.is_empty() {
            return;
        }
        let frame = ModuleTelemetryFrame {
            seq: self.seq,
            frame_time_ns: now_ns,
            records,
        };
        self.seq = self.seq.wrapping_add(1);
        sink(&frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vxmt_layout() {
        let f = ModuleTelemetryFrame {
            seq: 9,
            frame_time_ns: 123_456_789_000,
            records: vec![
                ModuleTelemetryRecord {
                    slot_uid: 3,
                    values: vec![-3.5],
                },
                ModuleTelemetryRecord {
                    slot_uid: 7,
                    values: vec![0.0, -12.25],
                },
            ],
        };
        let b = f.encode();
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f32_at = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(b.len(), 32 + (8 + 4) + (8 + 8));
        assert_eq!(&b[0..4], b"VXMT");
        assert_eq!((u16_at(4), u16_at(6)), (1, 32));
        assert_eq!((u32_at(8), u32_at(12)), (9, 0));
        assert_eq!(
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
            123_456_789_000
        );
        assert_eq!((u32_at(24), u32_at(28)), (2, 0));
        assert_eq!((u32_at(32), u16_at(36), u16_at(38)), (3, 1, 0));
        assert_eq!(f32_at(40).to_bits(), (-3.5f32).to_bits());
        assert_eq!((u32_at(44), u16_at(48)), (7, 2));
        assert_eq!(f32_at(52).to_bits(), 0.0f32.to_bits());
        assert_eq!(f32_at(56).to_bits(), (-12.25f32).to_bits());
    }

    /// A sine with peak −20 dBFS reads −23.0 dB RMS (SPEC-002 §2.1); the window slides.
    #[test]
    fn input_meter_rms_window_and_peak_hold() {
        let rate = 48_000u32;
        let mut m = InputMeter::default();
        m.reset(rate);
        let amp = 0.1f64;
        let block: Vec<f64> = (0..480)
            .map(|i| amp * (std::f64::consts::TAU * 1000.0 * f64::from(i) / f64::from(rate)).sin())
            .collect();
        let sum: f64 = block.iter().map(|x| x * x).sum();
        for _ in 0..100 {
            m.add(480, amp as f32, sum, false);
        }
        let (peak_db, rms_db, clip) = m.take();
        assert!((peak_db + 20.0).abs() < 1e-3);
        assert!((rms_db + 23.0103).abs() < 0.01, "{rms_db}");
        assert!(!clip);
        // Silence for > 300 ms empties the window; peak resets per frame.
        for _ in 0..31 {
            m.add(480, 0.0, 0.0, false);
        }
        let (peak_db, rms_db, _) = m.take();
        assert!(peak_db.is_infinite() && rms_db.is_infinite(), "{rms_db}");
        m.add(480, 0.5, 0.0, false);
        let _ = m.take();
        assert!(
            (m.take().0 + 6.0206).abs() < 1e-3,
            "a frame without a new block holds the peak"
        );
        m.add(10, 1.0, 1.0, true);
        assert!(m.take().2, "clip flag");
        assert!(!m.take().2, "clip flag resets");
    }

    #[test]
    fn vxtm_layout() {
        let f = TelemetryFrame {
            seq: 7,
            flags: vxtm_flags::PLAYING | vxtm_flags::XRUN,
            playhead_sample: 48_000,
            playhead_time_ns: 1_000_000_000,
            rate: 44_100.0,
            out_peak_dbfs: -6.0,
            out_rms_dbfs: f32::NEG_INFINITY,
            in_peak_dbfs: f32::NEG_INFINITY,
            in_rms_dbfs: f32::NEG_INFINITY,
            audio_rev: 3,
            dropped_rt_events: 2,
        };
        let b = f.encode();
        assert_eq!(&b[0..4], b"VXTM");
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 72);
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 0b1_0001);
        assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), 48_000);
        assert_eq!(
            f64::from_le_bytes(b[32..40].try_into().unwrap()).to_bits(),
            44_100.0f64.to_bits()
        );
        assert!(f32::from_le_bytes(b[44..48].try_into().unwrap()).is_infinite());
        assert_eq!(u64::from_le_bytes(b[56..64].try_into().unwrap()), 3);
        assert_eq!(&b[68..72], &[0, 0, 0, 0]);
    }

    // --- H-16: telemetry rate gate ---------------------------------------------------------

    #[test]
    fn rate_gate_fires_every_tick_at_60hz() {
        let mut gate = TelemetryRateGate::new(60);
        for _ in 0..10 {
            assert!(gate.due());
        }
    }

    #[test]
    fn rate_gate_fires_every_other_tick_at_30hz() {
        let mut gate = TelemetryRateGate::new(30);
        let fires: Vec<bool> = (0..6).map(|_| gate.due()).collect();
        assert_eq!(fires, vec![false, true, false, true, false, true]);
    }

    #[test]
    fn rate_gate_treats_out_of_range_rates_as_the_nearer_valid_one() {
        assert_eq!(rate_divisor(0), 2);
        assert_eq!(rate_divisor(1), 2);
        assert_eq!(rate_divisor(45), 1);
        assert_eq!(rate_divisor(60), 1);
        assert_eq!(rate_divisor(120), 1);
    }

    /// The rate the gate's divisor implies and the rate handed to the analyzer's EMA constants
    /// must always agree, or halving the tick rate would silently change the Fast/Medium/Slow
    /// time constants without the analyzer knowing (SPEC-007 §4.8 step 4).
    #[test]
    fn effective_rate_matches_the_gates_divisor() {
        for rate_hz in [0, 1, 29, 30, 31, 45, 59, 60, 61, 120] {
            let effective = effective_telemetry_rate_hz(rate_hz);
            assert_eq!(60 / rate_divisor(rate_hz), effective, "rate_hz={rate_hz}");
        }
    }

    #[test]
    fn rate_gate_resets_phase_on_rate_change() {
        let mut gate = TelemetryRateGate::new(30);
        assert!(!gate.due(), "counter is now 1 of 2");
        gate.set_rate_hz(60);
        assert!(
            gate.due(),
            "changing rate resets phase so it fires right away"
        );
    }

    #[test]
    fn meter_peak_and_rms() {
        let mut m = Meter::default();
        m.reset(48_000);
        m.add(0.5, 0.25 * 100.0, 100);
        m.add(0.25, 0.0, 100);
        let (peak, peak_db, rms_db) = m.take();
        assert_eq!(peak.to_bits(), 0.5f32.to_bits());
        assert!((peak_db + 6.0206).abs() < 1e-3);
        assert!((rms_db - to_dbfs(0.125f64.sqrt())).abs() < 1e-4);
        assert!(m.take().1.is_infinite(), "peak resets after take");
        // H-41: unlike peak, the RMS window is *not* cleared by take() — a telemetry frame with
        // no new block still reports the level of the audio still inside the window, not silence.
        assert!(m.take().2.is_finite(), "the RMS window outlives take()");
    }

    /// H-41 (the ticket's own acceptance numbers): a full-scale-relative sine at −6 dBFS peak
    /// gives peak −6.0 dBFS and RMS −9.0 dBFS (a sine's RMS is 3.0103 dB under its peak).
    #[test]
    fn meter_sine_peak_and_rms() {
        let rate = 48_000u32;
        let mut m = Meter::default();
        m.reset(rate);
        let amp = 10f64.powf(-6.0 / 20.0);
        let block_len = 480usize;
        let block: Vec<f64> = (0..block_len)
            .map(|i| amp * (std::f64::consts::TAU * 1000.0 * i as f64 / f64::from(rate)).sin())
            .collect();
        let sum_sq: f64 = block.iter().map(|x| x * x).sum();
        let peak = block.iter().cloned().fold(0.0, f64::max) as f32;
        // Fill the full 300 ms window with identical blocks so the RMS has converged.
        let blocks_per_window = (u64::from(rate) * METER_RMS_WINDOW_MS / 1000) / block_len as u64;
        for _ in 0..(blocks_per_window * 2) {
            m.add(peak, sum_sq, block_len as u32);
        }
        let (_, peak_db, rms_db) = m.take();
        assert!((peak_db + 6.0).abs() < 0.05, "peak_db={peak_db}");
        assert!((rms_db + 9.0).abs() < 0.05, "rms_db={rms_db}");
    }

    /// H-41: the RMS window is a proper sliding 300 ms rectangular window (SPEC-002 §3's fixed
    /// convention, reused for the output meter per ADR-003's `VXTM` table) — *not* just whatever
    /// arrived since the previous telemetry frame (which at 60 Hz would be ~17 ms of audio, far
    /// too little to read as a stable level rather than noise).
    #[test]
    fn meter_rms_is_a_sliding_window_not_a_per_frame_reset() {
        let rate = 48_000u32;
        let mut m = Meter::default();
        m.reset(rate);
        // One block of full-scale audio, then many frames' worth of silence: a per-frame-reset
        // RMS would read silence on the very next `take()`; a real 300 ms window keeps the loud
        // block's energy audible until it actually slides out.
        m.add(1.0, 480.0, 480);
        let (_, _, rms_after_block) = m.take();
        assert!(rms_after_block.is_finite());
        m.add(0.0, 0.0, 480);
        let (_, _, rms_next_frame) = m.take();
        assert!(
            rms_next_frame.is_finite() && rms_next_frame < 0.0,
            "the loud block is still inside the 300 ms window: {rms_next_frame}"
        );
        // Push enough silence to slide the whole window past the loud block (300 ms of it).
        let window_frames = u64::from(rate) * METER_RMS_WINDOW_MS / 1000;
        let mut pushed = 480u64; // already added above
        while pushed < window_frames + 480 {
            m.add(0.0, 0.0, 480);
            pushed += 480;
        }
        let (_, _, rms_after_window_slides) = m.take();
        assert!(
            rms_after_window_slides.is_infinite(),
            "the loud block has slid out of the window: {rms_after_window_slides}"
        );
    }

    /// H-41: before the meter's first `reset()` (no stream open yet, or a fresh `Meter::default()`
    /// as some other caller might construct), `add` must not silently accumulate against an
    /// unknown window size — it should read as silence rather than a meaningless (rate-less) RMS.
    #[test]
    fn meter_before_reset_reads_silent() {
        let mut m = Meter::default();
        m.add(1.0, 480.0, 480);
        let (_, _, rms_db) = m.take();
        assert!(rms_db.is_infinite());
    }
}
