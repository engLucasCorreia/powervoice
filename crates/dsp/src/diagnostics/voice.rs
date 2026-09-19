//! Voice statistics (H-42, SPEC-007 §8.6–§8.8): one [`VoiceReport`] — F0, tone balance,
//! sibilance, hum, rumble, noise floor and SNR — either **live** ([`VoiceTracker`], fed the
//! analyzer tap's samples: smoothed spectral values plus statistics over the last
//! [`STATS_WINDOW_S`] s) or **offline** ([`analyze_buffer`], a whole selection/document, for the
//! long-term average mode).
//!
//! Levels (SPEC-007 §8.8):
//! - 10 ms blocks of mean square; blocks of exact digital silence are ignored;
//! - **noise floor** = the ACX definition: RMS of the quietest 500 ms window (unweighted),
//!   evaluated on the 10 ms grid ([`crate::acx`] evaluates it per sample);
//! - **active level** = power mean of the blocks at least [`ACTIVE_MARGIN_DB`] above the noise
//!   floor (ITU-T P.56's 15.9 dB margin, simplified: no envelope hangover);
//! - **SNR** = active level − noise floor.
//!
//! Spectra (SPEC-007 §8.7): Hann frames of [`diagnostics_fft_size`] (≈ 0.34 s: 16 384 points at
//! 44.1/48 kHz — 2.9 Hz bins, fine enough to separate 50 from 60 Hz hum), each gated by its
//! level: **active** frames (≥ noise floor + 15.9 dB) feed the voice spectrum that tone
//! balance, sibilance and rumble read; **quiet** frames (≤ noise floor + [`QUIET_MARGIN_DB`])
//! feed the room-tone spectrum hum is detected in — so a voice whose harmonics happen to fall
//! on 100/150/200 Hz is not mistaken for mains hum. Live, both spectra are exponential averages
//! (τ = [`SPECTRUM_TAU_S`]); offline, plain means.
//!
//! F0 (YIN, [`super::pitch`]) every 10 ms live, 20 ms offline. Statistics over the voiced
//! frames: median, 10th and 90th percentiles (the "range"), and the voiced fraction of the
//! frames with signal. Live, "current" is the median of the voiced frames of the last 0.3 s.
//! The frames are aggregated by [`super::f0_profile`] (H-91), which folds YIN's octave errors
//! onto the robust centre first — a few slipped frames would otherwise move the percentiles by
//! an octave — and reports how confident the estimate was and how much of it needed correcting.

use std::collections::VecDeque;

use super::f0_profile::{self, F0Frame};
use super::features::{self, Hum, Sibilance, SpectrumView, ToneBalance};
use super::pitch::Yin;
use super::spectrum::{PowerSpectrum, power_db};
use super::window::WindowKind;

/// Live statistics window for F0 (s).
pub const STATS_WINDOW_S: f64 = 10.0;
/// Live statistics window for the noise floor / active level / SNR (s).
pub const LEVEL_WINDOW_S: f64 = 30.0;
/// Level block (s).
pub const LEVEL_BLOCK_S: f64 = 0.010;
/// ACX noise-floor window (s).
pub const NOISE_FLOOR_WINDOW_S: f64 = 0.5;
/// Active-speech margin above the noise floor (dB, ITU-T P.56).
pub const ACTIVE_MARGIN_DB: f64 = 15.9;
/// Quiet (room-tone) frames sit within this margin above the noise floor (dB).
pub const QUIET_MARGIN_DB: f64 = 6.0;
/// YIN hop, live (s).
pub const LIVE_YIN_HOP_S: f64 = 0.010;
/// YIN hop, offline (s).
pub const OFFLINE_YIN_HOP_S: f64 = 0.020;
/// Diagnostics-spectrum hop, live (s).
pub const LIVE_SPECTRUM_HOP_S: f64 = 0.100;
/// Live spectrum averaging time constant (s).
pub const SPECTRUM_TAU_S: f64 = 3.0;
/// Live "current F0" span (s).
pub const CURRENT_F0_SPAN_S: f64 = 0.3;
/// Fewer voiced frames than this → no F0 statistics.
pub const MIN_VOICED_FRAMES: usize = 5;
/// Fewer voiced frames than this in the last [`CURRENT_F0_SPAN_S`] → no live "now" readout.
pub const MIN_CURRENT_FRAMES: usize = 3;
/// Fewer quiet frames than this → no hum verdict (not enough room tone heard yet).
pub const MIN_QUIET_FRAMES: u64 = 3;
/// Live: the room-tone average restarts when the noise floor drops by more than this (dB) —
/// before the first pause the "floor" is the quietest stretch of the voice itself.
pub const QUIET_RESTART_DB: f64 = 3.0;

/// The diagnostics spectrum's FFT size: the power of two nearest 0.34 s, clamped to
/// 8 192 … 32 768 (16 384 at 44.1/48 kHz, 32 768 at 88.2/96 kHz).
pub fn diagnostics_fft_size(sample_rate_hz: u32) -> usize {
    let target = f64::from(sample_rate_hz.max(1)) * 0.34;
    let exp = target.log2().round().clamp(13.0, 15.0) as u32;
    1usize << exp
}

/// F0 statistics (Hz).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Stats {
    /// Live only: the median of the last 0.3 s of voiced frames (`None` between phrases).
    pub current_hz: Option<f64>,
    pub median_hz: f64,
    /// 10th percentile.
    pub low_hz: f64,
    /// 90th percentile.
    pub high_hz: f64,
    /// Voiced frames / frames with signal, 0 … 1.
    pub voiced_fraction: f64,
    /// `1 − median aperiodicity` of the voiced frames, 0 … 1 (H-91): how periodic the voice
    /// actually was, so the reader can tell a firm reading from a breathy guess.
    pub confidence: f64,
    /// Share of the voiced frames that were an octave off the robust centre and were folded onto
    /// it (H-91, [`super::f0_profile`]), 0 … 1.
    pub octave_corrected: f64,
}

/// Everything the diagnostics panel shows. `None` = not measurable (silence, too little audio,
/// no room tone heard yet…).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VoiceReport {
    pub f0: Option<F0Stats>,
    pub tone: Option<ToneBalance>,
    pub sibilance: Option<Sibilance>,
    pub hum: Option<Hum>,
    pub rumble_db: Option<f64>,
    pub noise_floor_dbfs: Option<f64>,
    pub active_level_dbfs: Option<f64>,
    pub snr_db: Option<f64>,
    /// Seconds of non-silent audio the statistics cover (live: within the last
    /// [`STATS_WINDOW_S`] s) — steady during silence, so an idle report doesn't change.
    pub span_s: f64,
}

/// Noise floor and active level from 10 ms mean-square blocks (0 = digital silence, skipped).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct LevelStats {
    noise_floor_dbfs: Option<f64>,
    active_level_dbfs: Option<f64>,
}

fn level_stats<I: Iterator<Item = f64> + Clone>(blocks: I) -> LevelStats {
    let win = (NOISE_FLOOR_WINDOW_S / LEVEL_BLOCK_S).round() as usize;
    let mut window: VecDeque<f64> = VecDeque::with_capacity(win + 1);
    let mut sum = 0.0;
    let mut min_mean: Option<f64> = None;
    for ms in blocks.clone().filter(|&ms| ms > 0.0) {
        window.push_back(ms);
        sum += ms;
        if window.len() > win {
            sum -= window.pop_front().unwrap_or(0.0);
        }
        if window.len() == win {
            let mean = (sum / win as f64).max(0.0);
            min_mean = Some(min_mean.map_or(mean, |m: f64| m.min(mean)));
        }
    }
    let Some(floor_ms) = min_mean.filter(|&m| m > 0.0) else {
        return LevelStats::default();
    };
    let floor_db = power_db(floor_ms);
    let gate = 10f64.powf((floor_db + ACTIVE_MARGIN_DB) / 10.0);
    let (active_sum, active_n) = blocks
        .filter(|&ms| ms >= gate)
        .fold((0.0, 0usize), |(s, n), ms| (s + ms, n + 1));
    LevelStats {
        noise_floor_dbfs: Some(floor_db),
        active_level_dbfs: (active_n > 0).then(|| power_db(active_sum / active_n as f64)),
    }
}

/// Nearest-rank percentile of an ascending slice.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// The profile of `voiced` (every voiced frame of the statistics window) plus, live, the "now"
/// readout from `recent` (the voiced frames of the last [`CURRENT_F0_SPAN_S`]) — each of those
/// folded onto the profile's octave first, so one slipped frame can't make the readout jump.
fn f0_stats(voiced: &[F0Frame], signal_frames: usize, recent: &[F0Frame]) -> Option<F0Stats> {
    if voiced.len() < MIN_VOICED_FRAMES {
        return None;
    }
    let profile = f0_profile::profile(voiced)?;
    let current_hz = (recent.len() >= MIN_CURRENT_FRAMES).then(|| {
        let mut folded: Vec<f64> = recent
            .iter()
            .map(|f| f0_profile::fold_toward(f.f0_hz, profile.centre_log2))
            .collect();
        folded.sort_by(f64::total_cmp);
        percentile(&folded, 0.5)
    });
    Some(F0Stats {
        current_hz,
        median_hz: profile.median_hz,
        low_hz: profile.low_hz,
        high_hz: profile.high_hz,
        voiced_fraction: voiced.len() as f64 / signal_frames.max(voiced.len()) as f64,
        confidence: profile.confidence,
        octave_corrected: profile.octave_corrected,
    })
}

/// Which spectrum a diagnostics frame feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    Active,
    Quiet,
    Neither,
}

fn gate(frame_ms: f64, noise_floor_dbfs: Option<f64>) -> Gate {
    if frame_ms <= 0.0 {
        return Gate::Neither;
    }
    let db = power_db(frame_ms);
    match noise_floor_dbfs {
        None => Gate::Neither,
        Some(floor) if db >= floor + ACTIVE_MARGIN_DB => Gate::Active,
        Some(floor) if db <= floor + QUIET_MARGIN_DB => Gate::Quiet,
        Some(_) => Gate::Neither,
    }
}

fn mean_square(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    x.iter()
        .filter(|s| s.is_finite())
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / x.len() as f64
}

fn spectral_report(
    report: &mut VoiceReport,
    active: Option<SpectrumView<'_>>,
    quiet: Option<SpectrumView<'_>>,
) {
    if let Some(view) = active {
        report.tone = features::tone_balance(&view);
        report.sibilance = features::sibilance(&view);
        report.rumble_db = features::rumble_db(&view);
    }
    if let Some(view) = quiet {
        report.hum = features::detect_hum(&view);
    }
}

fn fill_levels(report: &mut VoiceReport, levels: LevelStats) {
    report.noise_floor_dbfs = levels.noise_floor_dbfs;
    report.active_level_dbfs = levels.active_level_dbfs;
    report.snr_db = match (levels.active_level_dbfs, levels.noise_floor_dbfs) {
        (Some(a), Some(n)) => Some(a - n),
        _ => None,
    };
}

/// Live voice statistics over the analyzer tap (SPEC-007 §8.8). Feed it every new sample with
/// [`VoiceTracker::push`]; read a [`VoiceReport`] whenever the panel wants one. Nothing heavy
/// happens during silence: YIN stops at its energy gate and silent spectrum frames skip the FFT.
pub struct VoiceTracker {
    sample_rate_hz: u32,
    ring: Vec<f32>,
    ring_pos: usize,
    total_samples: u64,
    scratch: Vec<f32>,
    yin: Yin,
    yin_hop: usize,
    yin_pos: usize,
    spectrum: PowerSpectrum,
    spec_hop: usize,
    spec_pos: usize,
    frame_power: Vec<f64>,
    block_len: usize,
    block_pos: usize,
    block_sum: f64,
    levels: VecDeque<f64>,
    level_cap: usize,
    /// Per YIN hop: > 0 voiced F0 (Hz), 0 unvoiced with signal, < 0 silent.
    f0s: VecDeque<F0Sample>,
    f0_cap: usize,
    active: Vec<f64>,
    active_frames: u64,
    quiet: Vec<f64>,
    quiet_frames: u64,
    /// The noise floor the room-tone average was started at (see [`QUIET_RESTART_DB`]).
    quiet_floor_db: Option<f64>,
    alpha: f64,
    f0_scratch: Vec<F0Frame>,
    recent_scratch: Vec<F0Frame>,
}

/// One YIN hop as the tracker keeps it: the estimate (with the sentinels above) and, for a
/// voiced frame, its aperiodicity — [`super::f0_profile`] needs both.
#[derive(Debug, Clone, Copy, PartialEq)]
struct F0Sample {
    f0: f32,
    aperiodicity: f32,
}

impl F0Sample {
    const UNVOICED: Self = Self {
        f0: 0.0,
        aperiodicity: 1.0,
    };
    const SILENT: Self = Self {
        f0: -1.0,
        aperiodicity: 1.0,
    };

    fn frame(self) -> F0Frame {
        F0Frame {
            f0_hz: f64::from(self.f0),
            aperiodicity: f64::from(self.aperiodicity),
        }
    }
}

impl VoiceTracker {
    pub fn new(sample_rate_hz: u32) -> Self {
        let fs = f64::from(sample_rate_hz.max(1));
        let fft = diagnostics_fft_size(sample_rate_hz);
        let yin = Yin::new(sample_rate_hz);
        let ring_len = fft.max(yin.frame_len());
        let spectrum = PowerSpectrum::new(fft, WindowKind::Hann);
        let bins = spectrum.bins();
        let yin_hop = ((LIVE_YIN_HOP_S * fs).round() as usize).max(1);
        let f0_cap = (STATS_WINDOW_S / LIVE_YIN_HOP_S).round() as usize;
        let level_cap = (LEVEL_WINDOW_S / LEVEL_BLOCK_S).round() as usize;
        Self {
            sample_rate_hz,
            ring: vec![0.0; ring_len],
            ring_pos: 0,
            total_samples: 0,
            scratch: vec![0.0; ring_len],
            yin,
            yin_hop,
            yin_pos: 0,
            spectrum,
            spec_hop: ((LIVE_SPECTRUM_HOP_S * fs).round() as usize).max(1),
            spec_pos: 0,
            frame_power: vec![0.0; bins],
            block_len: ((LEVEL_BLOCK_S * fs).round() as usize).max(1),
            block_pos: 0,
            block_sum: 0.0,
            levels: VecDeque::with_capacity(level_cap + 1),
            level_cap,
            f0s: VecDeque::with_capacity(f0_cap + 1),
            f0_cap,
            active: vec![0.0; bins],
            active_frames: 0,
            quiet: vec![0.0; bins],
            quiet_frames: 0,
            quiet_floor_db: None,
            alpha: 1.0 - (-LIVE_SPECTRUM_HOP_S / SPECTRUM_TAU_S).exp(),
            f0_scratch: Vec::with_capacity(f0_cap),
            recent_scratch: Vec::new(),
        }
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Forgets everything (the analyzer's `RESET`: device reopen or rate change).
    pub fn reset(&mut self) {
        *self = Self::new(self.sample_rate_hz);
    }

    /// Feeds new tap samples.
    pub fn push(&mut self, samples: &[f32]) {
        let mut rest = samples;
        while !rest.is_empty() {
            let until = (self.block_len - self.block_pos)
                .min(self.yin_hop - self.yin_pos)
                .min(self.spec_hop - self.spec_pos);
            let n = until.min(rest.len());
            self.ingest(&rest[..n]);
            rest = &rest[n..];
            if self.block_pos == self.block_len {
                self.end_block();
            }
            if self.yin_pos == self.yin_hop {
                self.yin_pos = 0;
                self.run_yin();
            }
            if self.spec_pos == self.spec_hop {
                self.spec_pos = 0;
                self.run_spectrum();
            }
        }
    }

    fn ingest(&mut self, x: &[f32]) {
        let len = self.ring.len();
        for &s in x {
            self.ring[self.ring_pos] = s;
            self.ring_pos = (self.ring_pos + 1) % len;
            if s.is_finite() {
                self.block_sum += f64::from(s) * f64::from(s);
            }
        }
        self.total_samples += x.len() as u64;
        self.block_pos += x.len();
        self.yin_pos += x.len();
        self.spec_pos += x.len();
    }

    fn end_block(&mut self) {
        let ms = self.block_sum / self.block_len as f64;
        self.block_sum = 0.0;
        self.block_pos = 0;
        self.levels.push_back(ms);
        if self.levels.len() > self.level_cap {
            self.levels.pop_front();
        }
    }

    /// The latest `n` samples, oldest first, into `self.scratch[..n]`.
    fn latest(&mut self, n: usize) {
        let len = self.ring.len();
        let start = (self.ring_pos + len - n) % len;
        for i in 0..n {
            self.scratch[i] = self.ring[(start + i) % len];
        }
    }

    fn run_yin(&mut self) {
        let n = self.yin.frame_len();
        let value = if self.total_samples < n as u64 {
            F0Sample::SILENT
        } else {
            self.latest(n);
            match self.yin.estimate(&self.scratch[..n]) {
                Some(p) => F0Sample {
                    f0: p.f0_hz as f32,
                    aperiodicity: p.aperiodicity as f32,
                },
                None if self.yin.last_had_signal() => F0Sample::UNVOICED,
                None => F0Sample::SILENT,
            }
        };
        self.f0s.push_back(value);
        if self.f0s.len() > self.f0_cap {
            self.f0s.pop_front();
        }
    }

    fn run_spectrum(&mut self) {
        let n = self.spectrum.fft_size();
        if self.total_samples < n as u64 {
            return;
        }
        self.latest(n);
        let ms = mean_square(&self.scratch[..n]);
        if ms <= 0.0 {
            return;
        }
        let floor = level_stats(self.levels.iter().copied()).noise_floor_dbfs;
        let verdict = gate(ms, floor);
        if let (Gate::Quiet, Some(nf)) = (verdict, floor) {
            match self.quiet_floor_db {
                Some(started) if nf >= started - QUIET_RESTART_DB => {
                    self.quiet_floor_db = Some(nf);
                }
                _ => {
                    self.quiet.fill(0.0);
                    self.quiet_frames = 0;
                    self.quiet_floor_db = Some(nf);
                }
            }
        }
        let (acc, frames) = match verdict {
            Gate::Active => (&mut self.active, &mut self.active_frames),
            Gate::Quiet => (&mut self.quiet, &mut self.quiet_frames),
            Gate::Neither => return,
        };
        self.spectrum
            .power(&self.scratch[..n], &mut self.frame_power);
        // Running mean until τ worth of frames, then the exponential average.
        let alpha = self.alpha.max(1.0 / (*frames + 1) as f64);
        for (a, &p) in acc.iter_mut().zip(&self.frame_power) {
            *a += alpha * (p - *a);
        }
        *frames += 1;
    }

    /// The current report (smoothed spectral values, statistics over the last N s).
    pub fn report(&mut self) -> VoiceReport {
        let recent_blocks = (STATS_WINDOW_S / LEVEL_BLOCK_S).round() as usize;
        let mut report = VoiceReport {
            span_s: self
                .levels
                .iter()
                .rev()
                .take(recent_blocks)
                .filter(|&&ms| ms > 0.0)
                .count() as f64
                * LEVEL_BLOCK_S,
            ..VoiceReport::default()
        };

        let recent = (CURRENT_F0_SPAN_S / LIVE_YIN_HOP_S).round() as usize;
        self.recent_scratch.clear();
        self.recent_scratch.extend(
            self.f0s
                .iter()
                .rev()
                .take(recent)
                .filter(|s| s.f0 > 0.0)
                .map(|s| s.frame()),
        );
        let signal_frames = self.f0s.iter().filter(|s| s.f0 >= 0.0).count();
        self.f0_scratch.clear();
        self.f0_scratch
            .extend(self.f0s.iter().filter(|s| s.f0 > 0.0).map(|s| s.frame()));
        report.f0 = f0_stats(&self.f0_scratch, signal_frames, &self.recent_scratch);

        fill_levels(&mut report, level_stats(self.levels.iter().copied()));

        let fft = self.spectrum.fft_size();
        let enbw = self.spectrum.enbw_bins();
        let fs = self.sample_rate_hz;
        let active =
            (self.active_frames > 0).then(|| SpectrumView::new(&self.active, fs, fft, enbw));
        let quiet = (self.quiet_frames >= MIN_QUIET_FRAMES)
            .then(|| SpectrumView::new(&self.quiet, fs, fft, enbw));
        spectral_report(&mut report, active, quiet);
        report
    }
}

/// The long-term average job's parameters (SPEC-007 §8.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OfflineConfig {
    pub sample_rate_hz: u32,
    /// The displayed spectrum's FFT size (1 024 … 32 768).
    pub fft_size: usize,
    pub window: WindowKind,
}

/// A whole selection's analysis: the displayed long-term average spectrum at the requested
/// resolution, its room-tone counterpart, and the [`VoiceReport`].
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineAnalysis {
    pub fft_size: usize,
    pub window: WindowKind,
    pub enbw_bins: f64,
    /// Mean sine-normalized power per bin over every frame (`fft_size / 2 + 1`).
    pub ltas: Vec<f64>,
    /// Mean power over the quiet (room-tone) frames; `None` when there were none.
    pub noise: Option<Vec<f64>>,
    /// Frames averaged into `ltas`.
    pub frames: u64,
    pub report: VoiceReport,
}

/// Analyses `samples` (SPEC-007 §8.5). `progress(fraction)` is called regularly and returns
/// `false` to cancel — then this returns `None`.
///
/// # Panics
/// If `config.fft_size` is not a power of two ≥ 4.
pub fn analyze_buffer(
    samples: &[f32],
    config: OfflineConfig,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<OfflineAnalysis> {
    let fs = f64::from(config.sample_rate_hz.max(1));
    let progress_every = (fs as usize).max(1) * 2;

    // 1. Levels.
    let block_len = ((LEVEL_BLOCK_S * fs).round() as usize).max(1);
    let blocks: Vec<f64> = samples.chunks(block_len).map(mean_square).collect();
    let levels = level_stats(blocks.iter().copied());
    let floor = levels.noise_floor_dbfs;
    if !progress(0.05) {
        return None;
    }

    // 2. The displayed LTAS (all frames) and its room-tone counterpart (quiet frames).
    let mut spec = PowerSpectrum::new(config.fft_size, config.window);
    let bins = spec.bins();
    let mut frame_power = vec![0.0; bins];
    let mut ltas = vec![0.0; bins];
    let mut noise = vec![0.0; bins];
    let (mut frames, mut noise_frames) = (0u64, 0u64);
    let mut padded = Vec::new();
    let n = config.fft_size;
    let frame_starts: Vec<usize> = if samples.len() >= n {
        (0..=samples.len() - n).step_by(n / 2).collect()
    } else if samples.is_empty() {
        Vec::new()
    } else {
        padded = samples.to_vec();
        padded.resize(n, 0.0);
        vec![0]
    };
    let source: &[f32] = if padded.is_empty() { samples } else { &padded };
    for (i, &start) in frame_starts.iter().enumerate() {
        let frame = &source[start..start + n];
        spec.power(frame, &mut frame_power);
        for (a, &p) in ltas.iter_mut().zip(&frame_power) {
            *a += p;
        }
        frames += 1;
        if gate(mean_square(frame), floor) == Gate::Quiet {
            for (a, &p) in noise.iter_mut().zip(&frame_power) {
                *a += p;
            }
            noise_frames += 1;
        }
        if i % (progress_every / (n / 2).max(1)).max(1) == 0
            && !progress(0.05 + 0.3 * (i as f32 / frame_starts.len().max(1) as f32))
        {
            return None;
        }
    }
    let scale = |acc: &mut Vec<f64>, count: u64| {
        if count > 0 {
            let inv = 1.0 / count as f64;
            acc.iter_mut().for_each(|a| *a *= inv);
        }
    };
    scale(&mut ltas, frames);
    scale(&mut noise, noise_frames);

    // 3. The diagnostics spectra (fixed resolution), gated.
    let dfft = diagnostics_fft_size(config.sample_rate_hz);
    let mut dspec = PowerSpectrum::new(dfft, WindowKind::Hann);
    let mut dpower = vec![0.0; dspec.bins()];
    let mut active = vec![0.0; dspec.bins()];
    let mut quiet = vec![0.0; dspec.bins()];
    let (mut active_frames, mut quiet_frames) = (0u64, 0u64);
    if samples.len() >= dfft {
        let starts: Vec<usize> = (0..=samples.len() - dfft).step_by(dfft / 2).collect();
        for (i, &start) in starts.iter().enumerate() {
            let frame = &samples[start..start + dfft];
            let (acc, count) = match gate(mean_square(frame), floor) {
                Gate::Active => (&mut active, &mut active_frames),
                Gate::Quiet => (&mut quiet, &mut quiet_frames),
                Gate::Neither => continue,
            };
            dspec.power(frame, &mut dpower);
            for (a, &p) in acc.iter_mut().zip(&dpower) {
                *a += p;
            }
            *count += 1;
            if i % (progress_every / (dfft / 2)).max(1) == 0
                && !progress(0.35 + 0.3 * (i as f32 / starts.len().max(1) as f32))
            {
                return None;
            }
        }
    }
    scale(&mut active, active_frames);
    scale(&mut quiet, quiet_frames);

    // 4. F0.
    let mut yin = Yin::new(config.sample_rate_hz);
    let hop = ((OFFLINE_YIN_HOP_S * fs).round() as usize).max(1);
    let mut voiced: Vec<F0Frame> = Vec::new();
    let mut signal_frames = 0usize;
    let mut end = yin.frame_len();
    let mut since_progress = 0usize;
    while end <= samples.len() {
        if let Some(p) = yin.estimate(&samples[..end]) {
            voiced.push(F0Frame {
                f0_hz: p.f0_hz,
                aperiodicity: p.aperiodicity,
            });
        }
        if yin.last_had_signal() {
            signal_frames += 1;
        }
        end += hop;
        since_progress += hop;
        if since_progress >= progress_every {
            since_progress = 0;
            if !progress(0.65 + 0.35 * (end as f32 / samples.len() as f32)) {
                return None;
            }
        }
    }

    let mut report = VoiceReport {
        span_s: blocks.iter().filter(|&&ms| ms > 0.0).count() as f64 * LEVEL_BLOCK_S,
        f0: f0_stats(&voiced, signal_frames, &[]),
        ..VoiceReport::default()
    };
    fill_levels(&mut report, levels);
    let enbw = dspec.enbw_bins();
    let active_view =
        (active_frames > 0).then(|| SpectrumView::new(&active, config.sample_rate_hz, dfft, enbw));
    let quiet_view = (quiet_frames >= MIN_QUIET_FRAMES)
        .then(|| SpectrumView::new(&quiet, config.sample_rate_hz, dfft, enbw));
    spectral_report(&mut report, active_view, quiet_view);
    progress(1.0);

    Some(OfflineAnalysis {
        fft_size: n,
        window: config.window,
        enbw_bins: spec.enbw_bins(),
        ltas,
        noise: (noise_frames > 0).then_some(noise),
        frames,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::super::features::tests::{mix, sine};
    use super::super::pitch::tests::voice_like;
    use super::*;

    const FS: u32 = 48_000;

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    fn rms_db(x: &[f32]) -> f64 {
        power_db(mean_square(x))
    }

    /// Phrases of a 140 Hz voice (1.2 s) separated by 0.8 s pauses; underneath everything,
    /// pink room tone at −65 dBFS RMS and 50/100/150 Hz mains hum.
    fn session(seconds: f64) -> (Vec<f32>, f64) {
        let voice = voice_like(140.0, seconds, FS);
        let phrase = (1.2 * f64::from(FS)) as usize;
        let period = (2.0 * f64::from(FS)) as usize;
        let voice: Vec<f32> = voice
            .iter()
            .enumerate()
            .map(|(i, &v)| if i % period < phrase { v } else { 0.0 })
            .collect();
        let voice_rms = rms_db(
            &voice
                .chunks(period)
                .flat_map(|c| &c[..phrase.min(c.len())])
                .copied()
                .collect::<Vec<_>>(),
        );
        let room = vox_testkit::signal::pink_noise(11, -65.0, seconds, FS).unwrap();
        let hum = mix(&[
            &sine(50.0, -72.0, seconds),
            &sine(100.0, -75.0, seconds),
            &sine(150.0, -74.0, seconds),
        ]);
        (mix(&[&voice, &room, &hum]), voice_rms)
    }

    fn check_report(r: &VoiceReport, voice_rms: f64) {
        let f0 = r.f0.expect("f0");
        assert!(
            cents(f0.median_hz, 140.0).abs() < 25.0,
            "median {}",
            f0.median_hz
        );
        assert!(f0.low_hz <= f0.median_hz && f0.median_hz <= f0.high_hz);
        assert!(f0.low_hz > 125.0 && f0.high_hz < 155.0, "{f0:?}");
        assert!(f0.voiced_fraction > 0.4, "voiced {}", f0.voiced_fraction);

        let floor = r.noise_floor_dbfs.expect("noise floor");
        // Room tone −65 dBFS RMS + hum (−75, −78, −77 RMS) ≈ −64.5 dBFS.
        assert!((floor - (-64.5)).abs() < 1.5, "floor {floor}");
        let active = r.active_level_dbfs.expect("active");
        assert!(
            (active - voice_rms).abs() < 1.5,
            "active {active} vs {voice_rms}"
        );
        assert!((r.snr_db.unwrap() - (active - floor)).abs() < 1e-9);

        // Under −65 dBFS pink room tone the −72 dBFS 50 Hz fundamental barely stands out;
        // its harmonics do.
        let hum = r.hum.expect("hum from the pauses");
        assert!((hum.mains_hz - 50.0).abs() < 1e-9, "mains {}", hum.mains_hz);
        assert!(hum.harmonics.count_ones() >= 2, "mask {:b}", hum.harmonics);
        assert!(r.tone.is_some());
        assert!(r.sibilance.is_some());
        assert!(r.rumble_db.is_some());
    }

    #[test]
    fn live_tracker_reports_voice_statistics() {
        let (x, voice_rms) = session(12.0);
        let mut t = VoiceTracker::new(FS);
        // The engine hands over one control tick's worth at a time (~800 samples at 60 Hz).
        for chunk in x.chunks(800) {
            t.push(chunk);
        }
        let r = t.report();
        check_report(&r, voice_rms);
        assert!((r.span_s - STATS_WINDOW_S).abs() < 1e-9);
        // Ends in a pause (12 s = 6 × (1.2 + 0.8)): no current F0.
        assert_eq!(r.f0.unwrap().current_hz, None);
    }

    /// Alternating the amplitude of every other pitch period doubles the waveform's period
    /// without changing the voice's pitch — the classic way a real tracker is talked into
    /// reporting F0/2 (diplophonia / creak). `strength` 0 leaves the signal alone.
    fn period_double(x: &[f32], f0: f64, strength: f64) -> Vec<f32> {
        let period = f64::from(FS) / f0;
        x.iter()
            .enumerate()
            .map(|(i, &s)| {
                let odd = ((i as f64 / period).floor() as i64).rem_euclid(2) == 1;
                if odd {
                    (f64::from(s) * (1.0 - strength)) as f32
                } else {
                    s
                }
            })
            .collect()
    }

    #[test]
    fn octave_slips_do_not_move_the_pitch_profile() {
        // 8 s of a 140 Hz voice; the middle 2 s are period-doubled, which YIN reads as 70 Hz.
        let clean = voice_like(140.0, 8.0, FS);
        let start = 3 * FS as usize;
        let end = 5 * FS as usize;
        let mut slipped = clean.clone();
        let patch = period_double(&clean[start..end], 140.0, 0.55);
        slipped[start..end].copy_from_slice(&patch);

        let config = OfflineConfig {
            sample_rate_hz: FS,
            fft_size: 4096,
            window: WindowKind::Hann,
        };
        let f0 = |x: &[f32]| {
            analyze_buffer(x, config, |_| true)
                .unwrap()
                .report
                .f0
                .unwrap()
        };
        let reference = f0(&clean);
        let guarded = f0(&slipped);

        // The patch really does slip the tracker — otherwise this test proves nothing.
        assert!(
            guarded.octave_corrected > 0.05,
            "no slip to guard against: {guarded:?}"
        );
        // …and the profile is the same voice as the clean take, percentiles included.
        for (a, b, name) in [
            (guarded.median_hz, reference.median_hz, "median"),
            (guarded.low_hz, reference.low_hz, "low"),
            (guarded.high_hz, reference.high_hz, "high"),
        ] {
            assert!(cents(a, b).abs() < 50.0, "{name}: {a} vs {b} ({guarded:?})");
        }
        assert!(cents(guarded.low_hz, 140.0).abs() < 200.0, "{guarded:?}");
    }

    #[test]
    fn live_current_f0_follows_the_last_phrase() {
        let (x, _) = session(11.0); // ends 1.0 s into a phrase
        let mut t = VoiceTracker::new(FS);
        t.push(&x);
        let current = t.report().f0.unwrap().current_hz.expect("current");
        assert!(cents(current, 140.0).abs() < 60.0, "{current}");
    }

    #[test]
    fn offline_analysis_matches_the_live_tracker() {
        let (x, voice_rms) = session(12.0);
        let config = OfflineConfig {
            sample_rate_hz: FS,
            fft_size: 8192,
            window: WindowKind::Hann,
        };
        let mut calls = 0;
        let a = analyze_buffer(&x, config, |f| {
            calls += 1;
            assert!((0.0..=1.0).contains(&f));
            true
        })
        .unwrap();
        assert!(calls > 3);
        check_report(&a.report, voice_rms);
        assert_eq!(a.report.f0.unwrap().current_hz, None);
        assert_eq!(a.ltas.len(), 4097);
        assert!(a.frames > 100);
        let noise = a.noise.expect("room-tone spectrum");
        // The room tone sits well under the voice at 1 kHz.
        let k = (1000.0 / (f64::from(FS) / 8192.0)) as usize;
        assert!(power_db(a.ltas[k]) > power_db(noise[k]) + 20.0);
    }

    #[test]
    fn offline_analysis_can_be_cancelled() {
        let (x, _) = session(4.0);
        let config = OfflineConfig {
            sample_rate_hz: FS,
            fft_size: 4096,
            window: WindowKind::BlackmanHarris,
        };
        assert!(analyze_buffer(&x, config, |f| f < 0.3).is_none());
    }

    #[test]
    fn silence_reports_nothing() {
        let x = vec![0.0f32; FS as usize * 3];
        let mut t = VoiceTracker::new(FS);
        t.push(&x);
        let r = t.report();
        assert_eq!(r.f0, None);
        assert_eq!(r.noise_floor_dbfs, None);
        assert_eq!(r.hum, None);
        assert_eq!(r.tone, None);
        let config = OfflineConfig {
            sample_rate_hz: FS,
            fft_size: 1024,
            window: WindowKind::Hann,
        };
        let a = analyze_buffer(&x, config, |_| true).unwrap();
        assert_eq!(a.report.f0, None);
        assert!(a.noise.is_none());
        assert!(a.ltas.iter().all(|&p| p == 0.0));
        // An empty selection is still an answer (all zeros), not a panic.
        let empty = analyze_buffer(&[], config, |_| true).unwrap();
        assert_eq!(empty.frames, 0);
    }

    #[test]
    fn levels_follow_the_acx_definition() {
        // 1 s tone bursts (−20 dBFS peak = −23.01 dBFS RMS) with −70 dBFS RMS noise gaps.
        let x = vox_testkit::signal::tone_bursts(1, 1000.0, -20.0, Some(-70.0), 1.0, 1.0, 10.0, FS)
            .unwrap();
        let block = (LEVEL_BLOCK_S * f64::from(FS)) as usize;
        let blocks: Vec<f64> = x.chunks(block).map(mean_square).collect();
        let s = level_stats(blocks.iter().copied());
        assert!((s.noise_floor_dbfs.unwrap() - (-70.0)).abs() < 0.5);
        assert!((s.active_level_dbfs.unwrap() - (-23.01)).abs() < 0.3);
        // Digital silence is not a noise floor; too little audio gives no floor at all.
        assert_eq!(level_stats([0.0; 100].into_iter()), LevelStats::default());
        assert_eq!(level_stats([1e-4; 20].into_iter()), LevelStats::default());
    }

    #[test]
    fn reset_forgets_everything() {
        let (x, _) = session(3.0);
        let mut t = VoiceTracker::new(FS);
        t.push(&x);
        assert!(t.report().noise_floor_dbfs.is_some());
        t.reset();
        assert_eq!(t.report(), VoiceReport::default());
    }

    #[test]
    fn diagnostics_fft_sizes() {
        assert_eq!(diagnostics_fft_size(48_000), 16_384);
        assert_eq!(diagnostics_fft_size(44_100), 16_384);
        assert_eq!(diagnostics_fft_size(96_000), 32_768);
        assert_eq!(diagnostics_fft_size(22_050), 8192);
        assert_eq!(diagnostics_fft_size(192_000), 32_768);
    }
}
