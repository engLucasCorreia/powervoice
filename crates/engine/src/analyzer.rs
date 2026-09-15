//! Live output spectrum analyzer (T-208, SPEC-007 §2.9/§4.8, ADR-003 `VXSA`): an RT-safe tap in
//! the output callback that copies post-rack(+dry-monitor) mono samples into an `rtrb` ring
//! ([`AnalyzerTap`]), and a control-thread publisher ([`AnalyzerPublisher`]) that drains the ring,
//! runs `vox_dsp::analyzer`'s BH4 FFT + 1/24-octave band reduction + averaging, and sends one
//! [`AnalyzerFrame`] per subscriber per telemetry tick — only while at least one subscriber
//! exists (`ANALYZER_ON`), so the analyzer costs nothing when nobody is looking at it.
//!
//! Wire format: SPEC-007 §4.9's `VXSA` layout (48-byte header, `RESET`/`DROPPED`/`SILENT` flags,
//! no peak-hold payload — peak hold is UI ballistics, §4.8 step 7). This supersedes the shorter
//! table in ADR-003 Amendment 1, which predates §4.9 and lacks the flags AC-18/AC-19 require; see
//! ADR-003 Amendment 4.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use rtrb::{Consumer, Producer};
use vox_dsp::analyzer::{Analyzer as DspAnalyzer, Response, auto_fft_size};
use vox_dsp::diagnostics::spectrum::{MAX_FFT_SIZE, is_valid_fft_size, power_db};
use vox_dsp::diagnostics::{PowerSpectrum, VoiceReport, VoiceTracker, WindowKind};

/// RT tap ring capacity (SPEC-007 §3 `an_ring_samples`, ≈ 0.68 s at 48 kHz).
pub(crate) const ANALYZER_RING_FRAMES: usize = 32_768;

/// `VXSA` v1 header length (SPEC-007 §4.9).
pub const VXSA_HEADER_LEN: usize = 48;

/// `VXSA` flag bits (SPEC-007 §4.9).
pub mod vxsa_flags {
    /// History/averaging restarted (device reopen or rate change).
    pub const RESET: u32 = 1 << 0;
    /// Tap samples dropped since the previous frame (ring overflow).
    pub const DROPPED: u32 = 1 << 1;
    /// The analysis window is digital silence.
    pub const SILENT: u32 = 1 << 2;
}

/// The live analyzer's averaging response (re-exported for `src-tauri`/tests).
pub use vox_dsp::analyzer::Response as AnalyzerResponse;

/// RT-side tap: pushes post-rack(+dry) mono samples into the analyzer ring while its shared
/// `on` gate is set (SPEC-007 §4.8 step 1). Never blocks, allocates, locks or logs. When the
/// ring lacks space, the samples that fit are written and `dropped` is bumped by the rest.
pub(crate) struct AnalyzerTap {
    producer: Producer<f32>,
    on: Arc<AtomicBool>,
    dropped: Arc<AtomicU32>,
}

impl AnalyzerTap {
    pub(crate) fn new(
        producer: Producer<f32>,
        on: Arc<AtomicBool>,
        dropped: Arc<AtomicU32>,
    ) -> Self {
        Self {
            producer,
            on,
            dropped,
        }
    }

    /// Pushes `samples` (one sub-block) into the ring; a no-op while `on` is clear.
    pub(crate) fn push(&mut self, samples: &[f32]) {
        if samples.is_empty() || !self.on.load(Ordering::Relaxed) {
            return;
        }
        let avail = self.producer.slots();
        let n = samples.len().min(avail);
        if n > 0 {
            // `n <= avail`, so this cannot fail.
            if let Ok(mut chunk) = self.producer.write_chunk(n) {
                let (a, b) = chunk.as_mut_slices();
                a.copy_from_slice(&samples[..a.len()]);
                b.copy_from_slice(&samples[a.len()..a.len() + b.len()]);
                chunk.commit(n);
            }
        }
        if n < samples.len() {
            self.dropped
                .fetch_add((samples.len() - n) as u32, Ordering::Relaxed);
        }
    }
}

/// A ring-shaped history of the latest `N` samples (the analyzer's sliding analysis window).
/// `push` is O(pushed len); [`History::linearize`] is O(N), done once per publish.
struct History {
    buf: Vec<f32>,
    pos: usize,
}

impl History {
    fn new(n: usize) -> Self {
        Self {
            buf: vec![0.0; n.max(1)],
            pos: 0,
        }
    }

    fn resize(&mut self, n: usize) {
        self.buf = vec![0.0; n.max(1)];
        self.pos = 0;
    }

    fn push_slice(&mut self, samples: &[f32]) {
        let n = self.buf.len();
        // A burst longer than the window: only its tail matters.
        let tail = if samples.len() > n {
            &samples[samples.len() - n..]
        } else {
            samples
        };
        for &s in tail {
            self.buf[self.pos] = s;
            self.pos = (self.pos + 1) % n;
        }
    }

    /// Oldest → newest, into `out` (`out.len() == self.buf.len()`).
    fn linearize(&self, out: &mut [f32]) {
        let (tail, head) = self.buf.split_at(self.pos);
        out[..head.len()].copy_from_slice(head);
        out[head.len()..].copy_from_slice(tail);
    }
}

/// One subscriber's sink + its chosen response.
pub type AnalyzerSink = Box<dyn FnMut(&AnalyzerFrame) + Send>;

/// H-42: a voice-diagnostics subscriber (SPEC-007 §8.8) — called with a fresh
/// [`VoiceReport`] about [`VOICE_REPORT_HZ`] times a second, only when it changed.
pub type VoiceSink = Box<dyn FnMut(&VoiceReport) + Send>;

/// H-42: a Spectrum Inspector subscriber (SPEC-007 §8.3) — called with one [`InspectorFrame`]
/// every [`INSPECTOR_EVERY`] publishes while there is something to show.
pub type InspectorSink = Box<dyn FnMut(&InspectorFrame) + Send>;

/// Voice reports go out at most this often (Hz).
pub const VOICE_REPORT_HZ: f64 = 10.0;
/// Inspector frames go out every this many publishes (30 Hz at the default 60 Hz rate).
pub const INSPECTOR_EVERY: u32 = 2;
/// Below this averaged power (−150 dB) over a silent window, an Inspector stream goes idle:
/// one last frame, then nothing until sound returns (the UI then has nothing to redraw).
const INSPECTOR_IDLE_POWER: f64 = 1e-15;

/// One Spectrum Inspector subscription's analysis settings (SPEC-007 §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectorConfig {
    /// A power of two, 1 024 … 32 768 (clamped into range when invalid).
    pub fft_size: u32,
    pub window: WindowKind,
    pub response: Response,
}

impl InspectorConfig {
    fn sanitized(self) -> Self {
        let fft_size = if is_valid_fft_size(self.fft_size) {
            self.fft_size
        } else {
            vox_dsp::diagnostics::spectrum::DEFAULT_FFT_SIZE
        };
        Self { fft_size, ..self }
    }
}

struct InspectorSub {
    sink: InspectorSink,
    config: InspectorConfig,
    spectrum: PowerSpectrum,
    power: Vec<f64>,
    ema: Vec<f64>,
    frames: u64,
    idle: bool,
    pending_reset: bool,
}

impl InspectorSub {
    fn new(sink: InspectorSink, config: InspectorConfig) -> Self {
        let config = config.sanitized();
        let spectrum = PowerSpectrum::new(config.fft_size as usize, config.window);
        let bins = spectrum.bins();
        Self {
            sink,
            config,
            spectrum,
            power: vec![0.0; bins],
            ema: vec![0.0; bins],
            frames: 0,
            idle: false,
            pending_reset: true,
        }
    }

    fn reconfigure(&mut self, config: InspectorConfig) {
        let config = config.sanitized();
        if config.fft_size != self.config.fft_size || config.window != self.config.window {
            self.spectrum = PowerSpectrum::new(config.fft_size as usize, config.window);
            let bins = self.spectrum.bins();
            self.power = vec![0.0; bins];
            self.ema = vec![0.0; bins];
            self.frames = 0;
            self.pending_reset = true;
        }
        self.idle = false;
        self.config = config;
    }

    fn reset(&mut self) {
        self.ema.fill(0.0);
        self.frames = 0;
        self.idle = false;
        self.pending_reset = true;
    }
}

struct Subscription {
    sink: AnalyzerSink,
    response: Response,
}

/// Publishes `VXSA` frames at the telemetry rate: drains the RT tap ring into a sliding history,
/// runs one BH4 FFT + band reduction per tick (shared across every subscriber, ADR-003: "each
/// subscriber gets its own channel from one computation"), and sends each subscriber its chosen
/// response. Nothing is computed while nobody subscribes (`ANALYZER_ON` follows subscriber count).
pub(crate) struct AnalyzerPublisher {
    consumer: Option<Consumer<f32>>,
    on: Arc<AtomicBool>,
    dropped: Arc<AtomicU32>,
    dropped_seen: u32,
    history: History,
    window: Vec<f32>,
    scratch: Vec<f32>,
    analyzer: DspAnalyzer,
    sample_rate_hz: u32,
    fft_size: u32,
    rate_hz: f64,
    subs: HashMap<u32, Subscription>,
    next_id: u32,
    seq: u32,
    pending_reset: bool,
    /// H-42: live voice statistics, only while a voice subscriber exists.
    voice: Option<VoiceTracker>,
    voice_subs: HashMap<u32, VoiceSink>,
    voice_countdown: u32,
    last_voice: Option<VoiceReport>,
    /// H-42: Spectrum Inspector subscribers and their (longer) sample history.
    inspector_subs: HashMap<u32, InspectorSub>,
    inspector_history: History,
    inspector_window: Vec<f32>,
    inspector_countdown: u32,
    inspector_seq: u32,
}

const DEFAULT_SAMPLE_RATE_HZ: u32 = 48_000;

impl Default for AnalyzerPublisher {
    fn default() -> Self {
        let fft_size = auto_fft_size(DEFAULT_SAMPLE_RATE_HZ);
        Self {
            consumer: None,
            on: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU32::new(0)),
            dropped_seen: 0,
            history: History::new(fft_size as usize),
            window: vec![0.0; fft_size as usize],
            scratch: Vec::new(),
            analyzer: DspAnalyzer::new(fft_size, DEFAULT_SAMPLE_RATE_HZ, 60.0),
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            fft_size,
            rate_hz: 60.0,
            subs: HashMap::new(),
            next_id: 1,
            seq: 0,
            pending_reset: false,
            voice: None,
            voice_subs: HashMap::new(),
            voice_countdown: 0,
            last_voice: None,
            inspector_subs: HashMap::new(),
            inspector_history: History::new(0),
            inspector_window: Vec::new(),
            inspector_countdown: 0,
            inspector_seq: 0,
        }
    }
}

impl AnalyzerPublisher {
    /// The atomic the RT tap checks before touching the ring (`ANALYZER_ON`).
    pub(crate) fn gate(&self) -> Arc<AtomicBool> {
        self.on.clone()
    }

    /// The RT tap's drop counter.
    pub(crate) fn dropped_counter(&self) -> Arc<AtomicU32> {
        self.dropped.clone()
    }

    /// A fresh ring's consumer half is attached whenever the output (re)opens — which is exactly
    /// every point SPEC-007 §4.8.6 wants treated as a reset (device reopen, rate change).
    pub(crate) fn attach_ring(&mut self, consumer: Consumer<f32>, sample_rate_hz: u32) {
        self.consumer = Some(consumer);
        self.reconfigure(sample_rate_hz);
        self.pending_reset = true;
    }

    pub(crate) fn detach_ring(&mut self) {
        self.consumer = None;
    }

    fn reconfigure(&mut self, sample_rate_hz: u32) {
        let fft_size = auto_fft_size(sample_rate_hz);
        self.sample_rate_hz = sample_rate_hz;
        self.fft_size = fft_size;
        self.history.resize(fft_size as usize);
        self.window.resize(fft_size as usize, 0.0);
        self.analyzer = DspAnalyzer::new(fft_size, sample_rate_hz, self.rate_hz);
        if !self.inspector_subs.is_empty() {
            self.inspector_history.resize(MAX_FFT_SIZE as usize);
        }
    }

    /// Recomputes averaging time constants for a new `TELEMETRY_RATE` (H-16: driven by
    /// `Control::set_telemetry_rate_hz`, so the Fast/Medium/Slow EMAs stay correct at 30 Hz too).
    pub(crate) fn set_rate_hz(&mut self, rate_hz: f64) {
        self.rate_hz = rate_hz;
        self.analyzer.set_rate_hz(rate_hz);
    }

    /// Registers a new subscriber, returning its id (used by `analyzer_set_response`/
    /// `analyzer_unsubscribe`).
    pub(crate) fn subscribe(&mut self, sink: AnalyzerSink, response: Response) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.subs.insert(id, Subscription { sink, response });
        self.update_gate();
        id
    }

    pub(crate) fn set_response(&mut self, id: u32, response: Response) {
        if let Some(s) = self.subs.get_mut(&id) {
            s.response = response;
        }
    }

    /// Removes a subscriber of any kind (`VXSA`, voice, Inspector).
    pub(crate) fn unsubscribe(&mut self, id: u32) {
        self.subs.remove(&id);
        if self.voice_subs.remove(&id).is_some() && self.voice_subs.is_empty() {
            self.voice = None;
            self.last_voice = None;
        }
        if self.inspector_subs.remove(&id).is_some() && self.inspector_subs.is_empty() {
            self.inspector_history = History::new(0);
            self.inspector_window = Vec::new();
        }
        self.update_gate();
    }

    fn take_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// H-42: registers a voice-diagnostics subscriber (SPEC-007 §8.8). The statistics start
    /// with the first subscriber and are dropped with the last.
    pub(crate) fn subscribe_voice(&mut self, sink: VoiceSink) -> u32 {
        let id = self.take_id();
        self.voice_subs.insert(id, sink);
        // The newcomer gets the next report even if nothing changed.
        self.last_voice = None;
        self.voice_countdown = 0;
        self.update_gate();
        id
    }

    /// H-42: registers a Spectrum Inspector subscriber (SPEC-007 §8.3).
    pub(crate) fn subscribe_inspector(
        &mut self,
        sink: InspectorSink,
        config: InspectorConfig,
    ) -> u32 {
        let id = self.take_id();
        if self.inspector_subs.is_empty() {
            self.inspector_history = History::new(MAX_FFT_SIZE as usize);
            self.inspector_window = vec![0.0; MAX_FFT_SIZE as usize];
        }
        self.inspector_subs
            .insert(id, InspectorSub::new(sink, config));
        self.update_gate();
        id
    }

    /// H-42: changes an Inspector subscriber's FFT size / window / response.
    pub(crate) fn configure_inspector(&mut self, id: u32, config: InspectorConfig) {
        if let Some(sub) = self.inspector_subs.get_mut(&id) {
            sub.reconfigure(config);
        }
    }

    fn has_subscribers(&self) -> bool {
        !self.subs.is_empty() || !self.voice_subs.is_empty() || !self.inspector_subs.is_empty()
    }

    fn update_gate(&self) {
        self.on.store(self.has_subscribers(), Ordering::Relaxed);
    }

    /// Drains the ring, runs one FFT + band reduction + EMA update, and sends every subscriber
    /// its own frame — only while at least one subscriber exists (SPEC-007 §4.8 step 2).
    pub(crate) fn publish(&mut self, now_ns: u64) {
        if !self.has_subscribers() {
            return;
        }
        let Some(consumer) = self.consumer.as_mut() else {
            return;
        };
        let avail = consumer.slots();
        self.scratch.clear();
        if avail > 0 {
            if let Ok(chunk) = consumer.read_chunk(avail) {
                let (a, b) = chunk.as_slices();
                self.scratch.extend_from_slice(a);
                self.scratch.extend_from_slice(b);
                chunk.commit(avail);
            }
            self.history.push_slice(&self.scratch);
        }
        let reset = self.pending_reset;
        self.pending_reset = false;
        self.publish_voice(reset);
        self.publish_inspector(reset);
        if self.subs.is_empty() {
            return;
        }

        self.history.linearize(&mut self.window);
        let silent = self.analyzer.process(&self.window);

        let dropped_now = self.dropped.load(Ordering::Relaxed);
        let dropped = dropped_now != self.dropped_seen;
        self.dropped_seen = dropped_now;

        if reset {
            self.analyzer.reset();
        }

        let band_count = self.analyzer.band_count() as usize;
        let mut fast = vec![0.0f32; band_count];
        let mut medium = vec![0.0f32; band_count];
        let mut slow = vec![0.0f32; band_count];
        let mut need = [false; 3];
        for s in self.subs.values() {
            need[s.response.code() as usize] = true;
        }
        if need[Response::Fast.code() as usize] {
            self.analyzer.levels_db(Response::Fast, &mut fast);
        }
        if need[Response::Medium.code() as usize] {
            self.analyzer.levels_db(Response::Medium, &mut medium);
        }
        if need[Response::Slow.code() as usize] {
            self.analyzer.levels_db(Response::Slow, &mut slow);
        }

        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        let sample_rate_hz = self.sample_rate_hz;
        let fft_size = self.fft_size;
        for sub in self.subs.values_mut() {
            let levels = match sub.response {
                Response::Fast => &fast,
                Response::Medium => &medium,
                Response::Slow => &slow,
            };
            let frame = AnalyzerFrame {
                seq,
                reset,
                dropped,
                silent,
                frame_time_ns: now_ns,
                sample_rate_hz,
                fft_size,
                band_count: band_count as u32,
                response: sub.response,
                levels_db: levels.clone(),
            };
            (sub.sink)(&frame);
        }
    }

    /// H-42: feeds the new tap samples to the voice tracker; about [`VOICE_REPORT_HZ`] times a
    /// second, sends its report — only when it changed (silence sends nothing new).
    fn publish_voice(&mut self, reset: bool) {
        if self.voice_subs.is_empty() {
            return;
        }
        let rate = self.sample_rate_hz;
        let tracker = self.voice.get_or_insert_with(|| VoiceTracker::new(rate));
        if reset || tracker.sample_rate_hz() != rate {
            *tracker = VoiceTracker::new(rate);
            self.last_voice = None;
        }
        tracker.push(&self.scratch);
        if self.voice_countdown > 0 {
            self.voice_countdown -= 1;
            return;
        }
        self.voice_countdown = ((self.rate_hz / VOICE_REPORT_HZ).round() as u32).max(1) - 1;
        let report = tracker.report();
        if self.last_voice.as_ref() == Some(&report) {
            return;
        }
        for sink in self.voice_subs.values_mut() {
            sink(&report);
        }
        self.last_voice = Some(report);
    }

    /// H-42: one windowed FFT per Inspector subscriber every [`INSPECTOR_EVERY`] publishes,
    /// power-averaged with the subscriber's response; idle (no frames) once a silent window has
    /// decayed below −150 dB.
    fn publish_inspector(&mut self, reset: bool) {
        if self.inspector_subs.is_empty() {
            return;
        }
        self.inspector_history.push_slice(&self.scratch);
        if reset {
            self.inspector_subs
                .values_mut()
                .for_each(InspectorSub::reset);
        }
        if self.inspector_countdown > 0 {
            self.inspector_countdown -= 1;
            return;
        }
        self.inspector_countdown = INSPECTOR_EVERY - 1;
        self.inspector_history.linearize(&mut self.inspector_window);
        self.inspector_seq = self.inspector_seq.wrapping_add(1);
        let update_rate_hz = self.rate_hz / f64::from(INSPECTOR_EVERY);
        let len = self.inspector_window.len();
        for sub in self.inspector_subs.values_mut() {
            let n = sub.spectrum.fft_size().min(len);
            let window = &self.inspector_window[len - n..];
            let silent = window.iter().all(|&s| s == 0.0);
            if silent && sub.ema.iter().all(|&p| p < INSPECTOR_IDLE_POWER) {
                if sub.idle {
                    continue;
                }
                sub.idle = true;
            } else {
                sub.idle = false;
            }
            if silent {
                sub.power.fill(0.0);
            } else {
                sub.spectrum.power(window, &mut sub.power);
            }
            let tau = sub.config.response.tau_s();
            let alpha =
                (1.0 - (-(1.0 / update_rate_hz) / tau).exp()).max(1.0 / (sub.frames + 1) as f64);
            for (e, &p) in sub.ema.iter_mut().zip(&sub.power) {
                *e += alpha * (p - *e);
            }
            sub.frames += 1;
            let frame = InspectorFrame {
                seq: self.inspector_seq,
                reset: std::mem::take(&mut sub.pending_reset),
                silent,
                sample_rate_hz: self.sample_rate_hz,
                fft_size: sub.config.fft_size,
                window: sub.config.window,
                response: sub.config.response,
                levels_db: sub.ema.iter().map(|&p| power_db(p) as f32).collect(),
            };
            (sub.sink)(&frame);
        }
    }
}

/// `VXIS` v1 header length (H-42, SPEC-007 §8.9).
pub const VXIS_HEADER_LEN: usize = 40;

/// `VXIS` flag bits (SPEC-007 §8.9).
pub mod vxis_flags {
    /// Averaging restarted (subscribe, reconfigure, device reopen or rate change).
    pub const RESET: u32 = 1 << 0;
    /// The analysis window is digital silence.
    pub const SILENT: u32 = 1 << 1;
}

/// One Spectrum Inspector frame (`VXIS`, SPEC-007 §8.9): the averaged power of every FFT bin
/// `k = 0 … fft_size/2` at `k · fs / fft_size` Hz, in sine-normalized dB.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorFrame {
    pub seq: u32,
    pub reset: bool,
    pub silent: bool,
    pub sample_rate_hz: u32,
    pub fft_size: u32,
    pub window: WindowKind,
    pub response: Response,
    /// `fft_size / 2 + 1` levels, dB (`-inf` allowed, never NaN).
    pub levels_db: Vec<f32>,
}

impl InspectorFrame {
    /// Little-endian `VXIS` v1: `"VXIS"`, u16 version 1, u16 header_len 40, u32 seq, u32 flags,
    /// u32 sample_rate_hz, u32 fft_size, u32 window, u32 bin_count, u32 response, u32 reserved,
    /// then `f32[bin_count]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(VXIS_HEADER_LEN + 4 * self.levels_db.len());
        b.extend_from_slice(b"VXIS");
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&(VXIS_HEADER_LEN as u16).to_le_bytes());
        b.extend_from_slice(&self.seq.to_le_bytes());
        let mut flags = 0u32;
        if self.reset {
            flags |= vxis_flags::RESET;
        }
        if self.silent {
            flags |= vxis_flags::SILENT;
        }
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&self.sample_rate_hz.to_le_bytes());
        b.extend_from_slice(&self.fft_size.to_le_bytes());
        b.extend_from_slice(&self.window.code().to_le_bytes());
        b.extend_from_slice(&(self.levels_db.len() as u32).to_le_bytes());
        b.extend_from_slice(&self.response.code().to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for v in &self.levels_db {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }
}

/// One `VXSA` frame (SPEC-007 §4.9). Band centre frequencies are derived deterministically from
/// `band_count`/`fft_size`/`sample_rate_hz` (`f_k = 20·2^(k/24)`); they are not transmitted.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzerFrame {
    pub seq: u32,
    /// History/averaging restarted (device reopen or rate change).
    pub reset: bool,
    /// Tap samples dropped since the previous frame.
    pub dropped: bool,
    /// The analysis window is digital silence.
    pub silent: bool,
    pub frame_time_ns: u64,
    pub sample_rate_hz: u32,
    pub fft_size: u32,
    pub band_count: u32,
    pub response: Response,
    /// `band_count` averaged band levels, dB (`-inf` allowed, never NaN).
    pub levels_db: Vec<f32>,
}

/// Analyzer band-centre base frequency, as transmitted in the `VXSA` header (SPEC-007 §4.9).
const VXSA_F0_HZ: f32 = 20.0;
/// Analyzer band resolution, as transmitted in the `VXSA` header (SPEC-007 §4.9).
const VXSA_BANDS_PER_OCTAVE: u32 = 24;

impl AnalyzerFrame {
    /// Little-endian `VXSA` v1 encoding (SPEC-007 §4.9): a 48-byte header, then `f32[band_count]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(VXSA_HEADER_LEN + 4 * self.levels_db.len());
        b.extend_from_slice(b"VXSA");
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&(VXSA_HEADER_LEN as u16).to_le_bytes());
        b.extend_from_slice(&self.seq.to_le_bytes());
        let mut flags = 0u32;
        if self.reset {
            flags |= vxsa_flags::RESET;
        }
        if self.dropped {
            flags |= vxsa_flags::DROPPED;
        }
        if self.silent {
            flags |= vxsa_flags::SILENT;
        }
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&self.frame_time_ns.to_le_bytes());
        b.extend_from_slice(&self.sample_rate_hz.to_le_bytes());
        b.extend_from_slice(&self.fft_size.to_le_bytes());
        b.extend_from_slice(&VXSA_F0_HZ.to_le_bytes());
        b.extend_from_slice(&VXSA_BANDS_PER_OCTAVE.to_le_bytes());
        b.extend_from_slice(&self.band_count.to_le_bytes());
        b.extend_from_slice(&self.response.code().to_le_bytes());
        for v in &self.levels_db {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }
}
