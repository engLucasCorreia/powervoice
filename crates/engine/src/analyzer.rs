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

    pub(crate) fn unsubscribe(&mut self, id: u32) {
        self.subs.remove(&id);
        self.update_gate();
    }

    fn update_gate(&self) {
        self.on.store(!self.subs.is_empty(), Ordering::Relaxed);
    }

    /// Drains the ring, runs one FFT + band reduction + EMA update, and sends every subscriber
    /// its own frame — only while at least one subscriber exists (SPEC-007 §4.8 step 2).
    pub(crate) fn publish(&mut self, now_ns: u64) {
        if self.subs.is_empty() {
            return;
        }
        let Some(consumer) = self.consumer.as_mut() else {
            return;
        };
        let avail = consumer.slots();
        if avail > 0 {
            self.scratch.clear();
            if let Ok(chunk) = consumer.read_chunk(avail) {
                let (a, b) = chunk.as_slices();
                self.scratch.extend_from_slice(a);
                self.scratch.extend_from_slice(b);
                chunk.commit(avail);
            }
            self.history.push_slice(&self.scratch);
        }
        self.history.linearize(&mut self.window);
        let silent = self.analyzer.process(&self.window);

        let dropped_now = self.dropped.load(Ordering::Relaxed);
        let dropped = dropped_now != self.dropped_seen;
        self.dropped_seen = dropped_now;

        let reset = self.pending_reset;
        self.pending_reset = false;
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
