//! SPEC-012 §4.3 zipper / click measurement (normative for SPEC-012 AC-3/AC-4/AC-5 and
//! SPEC-002 AC-9).
//!
//! - [`zipper_signal`]: 997 Hz at −20 dBFS peak, 48 kHz, 3.0 s, phase 0.
//! - [`analyze_zipper`]: STFT (4-term Blackman-Harris, N = 8192, hop 1024) of an output, judged
//!   against its own steady-state spectrum: every transition frame, every bin with f ≤ 20 kHz
//!   and |f − 997 Hz| ≥ B_ex = max(10 / T_s, 1000 Hz) must stay at or below
//!   max(R(k) + 3 dB, −90 dBFS).
//! - [`ZipperTest`]: drives a module through the §4.3 parameter change (step up, step down, drag)
//!   in realtime (random blocks 1…1024, seeded) and offline mode and analyzes the output.
//!
//! Crossfades (bypass, swap, A/B) are analyzed with [`analyze_zipper`] directly, with
//! `t_s_ms = 15` and the sample at which the toggle takes effect.

use std::fmt;

use super::TestRng;
use super::fft::fft_in_place;
use crate::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, Module, OutputEvents, ParamEvent,
    ParamId, ProcessContext, ProcessMode, Transport, prepare_state,
};

/// Sample rate of the §4.3 test.
pub const ZIPPER_SAMPLE_RATE: f64 = 48_000.0;
/// Tone frequency.
pub const ZIPPER_FREQ_HZ: f64 = 997.0;
/// Tone peak level.
pub const ZIPPER_LEVEL_DBFS: f64 = -20.0;
/// Signal length in samples (3.0 s at 48 kHz).
pub const ZIPPER_LEN: usize = 144_000;
/// Sample of the change (deliberately not block-aligned).
pub const ZIPPER_CHANGE_AT: usize = 72_013;
/// STFT size.
pub const ZIPPER_FFT_SIZE: usize = 8192;
/// STFT hop.
pub const ZIPPER_HOP: usize = 1024;
/// Drag variant: number of events.
pub const ZIPPER_DRAG_EVENTS: usize = 60;
/// Drag variant: event spacing.
pub const ZIPPER_DRAG_INTERVAL_MS: f64 = 16.667;
/// Steady frames on each side of the transition.
const STEADY_FRAMES: usize = 4;
/// Guard between the steady frames and the transition.
const GUARD_MS: f64 = 50.0;
/// Level floor of the pass criterion.
const FLOOR_DBFS: f64 = -90.0;
/// Allowed rise over the steady-state spectrum.
const RISE_DB: f64 = 3.0;
const MAX_JUDGED_HZ: f64 = 20_000.0;

/// The §4.3 test signal: 997 Hz sine, −20 dBFS peak, 48 kHz, 3.0 s, starting phase 0.
pub fn zipper_signal() -> Vec<f32> {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let w = std::f64::consts::TAU * ZIPPER_FREQ_HZ / ZIPPER_SAMPLE_RATE;
    (0..ZIPPER_LEN)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect()
}

/// Where the change under test happens, in **input** time (before the latency shift).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZipperWindow {
    /// S: first sample of the change (the event, or the sample the toggle takes effect).
    pub start: usize,
    /// Last event sample (drag variant); equal to `start` for a single change.
    pub end: usize,
    /// Declared `smoothing_ms` (T_s = max(it, 1 ms)); 15 for crossfades.
    pub t_s_ms: f64,
}

impl ZipperWindow {
    /// A single change at `start`.
    pub fn step(start: usize, t_s_ms: f64) -> Self {
        Self {
            start,
            end: start,
            t_s_ms,
        }
    }
}

/// Result of [`analyze_zipper`].
#[derive(Clone, Debug, PartialEq)]
pub struct ZipperReport {
    /// Every judged bin of every transition frame was within its limit.
    pub pass: bool,
    /// Worst `L − max(R + 3, −90)` in dB over the judged bins (≤ 0 passes).
    pub worst_excess_db: f64,
    /// Level of the worst bin (dBFS).
    pub worst_level_dbfs: f64,
    /// Frequency of the worst bin.
    pub worst_freq_hz: f64,
    /// First sample (shifted output) of the frame holding the worst bin.
    pub worst_frame_start: usize,
    /// Worst transition-frame level with |f − 997 Hz| in 1–2 kHz, 2–5 kHz and 5 kHz…(f ≤ 20 kHz),
    /// regardless of B_ex (the §4.3 calibration table's columns).
    pub band_levels_dbfs: [f64; 3],
    /// B_ex used.
    pub b_ex_hz: f64,
    /// Number of transition frames judged.
    pub transition_frames: usize,
}

impl fmt::Display for ZipperReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} worst excess {:+.1} dB ({:.1} dBFS at {:.0} Hz, frame @{}); bands 1-2k {:.1} / \
             2-5k {:.1} / 5-20k {:.1} dBFS; B_ex {:.0} Hz; {} transition frames",
            if self.pass { "PASS" } else { "FAIL" },
            self.worst_excess_db,
            self.worst_level_dbfs,
            self.worst_freq_hz,
            self.worst_frame_start,
            self.band_levels_dbfs[0],
            self.band_levels_dbfs[1],
            self.band_levels_dbfs[2],
            self.b_ex_hz,
            self.transition_frames
        )
    }
}

/// 4-term Blackman-Harris window (periodic), N = [`ZIPPER_FFT_SIZE`].
fn window() -> Vec<f64> {
    let (a0, a1, a2, a3) = (0.35875, 0.48829, 0.14128, 0.01168);
    let n = ZIPPER_FFT_SIZE as f64;
    (0..ZIPPER_FFT_SIZE)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n;
            a0 - a1 * x.cos() + a2 * (2.0 * x).cos() - a3 * (3.0 * x).cos()
        })
        .collect()
}

/// Bin levels (dBFS, full-scale sine peak bin = 0 dBFS) of the frame starting at `start`.
fn frame_levels(x: &[f32], start: usize, w: &[f64], norm: f64) -> Vec<f64> {
    let mut re: Vec<f64> = x[start..start + ZIPPER_FFT_SIZE]
        .iter()
        .zip(w)
        .map(|(&s, &w)| f64::from(s) * w)
        .collect();
    let mut im = vec![0.0; ZIPPER_FFT_SIZE];
    fft_in_place(&mut re, &mut im);
    (0..=ZIPPER_FFT_SIZE / 2)
        .map(|k| {
            let mag = re[k].hypot(im[k]) / norm;
            if mag > 0.0 {
                20.0 * mag.log10()
            } else {
                -400.0
            }
        })
        .collect()
}

/// Analyzes `output` (the module or chain output for the §4.3 input) per SPEC-012 §4.3.
/// `latency` samples are dropped from the front first (the output is shifted by the latency).
///
/// # Errors
/// If the signal is too short for the steady frames on either side of `change`.
pub fn analyze_zipper(
    output: &[f32],
    latency: usize,
    sample_rate: f64,
    change: ZipperWindow,
) -> Result<ZipperReport, String> {
    let x = output
        .get(latency..)
        .ok_or_else(|| format!("output shorter than the latency {latency}"))?;
    let t_s = change.t_s_ms.max(1.0) / 1000.0;
    let t_s_samples = (t_s * sample_rate).round() as usize;
    let guard = (GUARD_MS / 1000.0 * sample_rate).round() as usize;
    let b_ex = (10.0 / t_s).max(1000.0);
    let (n, hop) = (ZIPPER_FFT_SIZE, ZIPPER_HOP);
    let frames = if x.len() >= n {
        (x.len() - n) / hop + 1
    } else {
        0
    };

    // Steady frames: the last 4 ending before S − 50 ms, the first 4 starting after
    // end + T_s + 50 ms.
    let before_limit = change
        .start
        .checked_sub(guard)
        .ok_or("change too early for steady frames")?;
    let before: Vec<usize> = (0..frames)
        .filter(|f| f * hop + n <= before_limit)
        .collect();
    let after_from = change.end + t_s_samples + guard;
    let after: Vec<usize> = (0..frames).filter(|f| f * hop >= after_from).collect();
    if before.len() < STEADY_FRAMES || after.len() < STEADY_FRAMES {
        return Err(format!(
            "not enough steady frames ({} before, {} after) for a change at {}..{} in {} samples",
            before.len(),
            after.len(),
            change.start,
            change.end,
            x.len()
        ));
    }
    let steady: Vec<usize> = before[before.len() - STEADY_FRAMES..]
        .iter()
        .chain(&after[..STEADY_FRAMES])
        .copied()
        .collect();
    // Transition frames: every frame whose window overlaps [S, end + T_s).
    let transition: Vec<usize> = (0..frames)
        .filter(|f| f * hop < change.end + t_s_samples && f * hop + n > change.start)
        .collect();

    let w = window();
    let norm = w.iter().sum::<f64>() / 2.0;
    let bins = n / 2 + 1;
    let freq = |k: usize| k as f64 * sample_rate / n as f64;
    let mut reference = vec![f64::NEG_INFINITY; bins];
    for &f in &steady {
        for (r, l) in reference.iter_mut().zip(frame_levels(x, f * hop, &w, norm)) {
            *r = r.max(l);
        }
    }
    let mut report = ZipperReport {
        pass: true,
        worst_excess_db: f64::NEG_INFINITY,
        worst_level_dbfs: f64::NEG_INFINITY,
        worst_freq_hz: 0.0,
        worst_frame_start: 0,
        band_levels_dbfs: [f64::NEG_INFINITY; 3],
        b_ex_hz: b_ex,
        transition_frames: transition.len(),
    };
    for &f in &transition {
        let levels = frame_levels(x, f * hop, &w, norm);
        for (k, &l) in levels.iter().enumerate() {
            let hz = freq(k);
            if hz > MAX_JUDGED_HZ {
                break;
            }
            let d = (hz - ZIPPER_FREQ_HZ).abs();
            let band = match d {
                d if d < 1000.0 => None,
                d if d < 2000.0 => Some(0),
                d if d < 5000.0 => Some(1),
                _ => Some(2),
            };
            if let Some(b) = band {
                report.band_levels_dbfs[b] = report.band_levels_dbfs[b].max(l);
            }
            if d < b_ex {
                continue;
            }
            let excess = l - (reference[k] + RISE_DB).max(FLOOR_DBFS);
            if excess > report.worst_excess_db {
                report.worst_excess_db = excess;
                report.worst_level_dbfs = l;
                report.worst_freq_hz = hz;
                report.worst_frame_start = f * hop;
            }
        }
    }
    report.pass = report.worst_excess_db <= 0.0;
    Ok(report)
}

/// The parameter move of a §4.3 run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipperMove {
    /// One event at S: `from_normalized(0.25)` → `from_normalized(0.75)`.
    Up,
    /// One event at S: `from_normalized(0.75)` → `from_normalized(0.25)`.
    Down,
    /// 60 events 16.667 ms apart from S, linear in normalized position 0.25 → 0.75.
    Drag,
}

/// How the module is driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipperMode {
    /// Realtime mode, `max_block` 1024, seeded random block sizes 1…1024.
    Realtime {
        /// Block-size seed.
        seed: u64,
    },
    /// Offline mode, 4096-frame blocks.
    Offline,
}

/// Drives one module parameter through the §4.3 runs.
pub struct ZipperTest {
    make: Box<dyn Fn() -> Box<dyn Module>>,
    param: ParamId,
}

impl ZipperTest {
    /// Tests parameter `param` of modules built by `make`.
    pub fn new(make: impl Fn() -> Box<dyn Module> + 'static, param: ParamId) -> Self {
        Self {
            make: Box::new(make),
            param,
        }
    }

    /// Renders one run. Returns the output, the change window and the module latency.
    ///
    /// # Errors
    /// Unknown parameter, state or activation failure.
    pub fn render(
        &self,
        mv: ZipperMove,
        mode: ZipperMode,
    ) -> Result<(Vec<f32>, ZipperWindow, usize), String> {
        let mut m = (self.make)();
        let p = m
            .params()
            .iter()
            .find(|p| p.id == self.param)
            .cloned()
            .ok_or_else(|| format!("no parameter {:?}", self.param))?;
        let (from, to) = match mv {
            ZipperMove::Up | ZipperMove::Drag => (0.25, 0.75),
            ZipperMove::Down => (0.75, 0.25),
        };
        let mut state = m.save_state().map_err(|e| e.to_string())?;
        state.params.insert(p.key.clone(), p.from_normalized(from));
        let state = prepare_state(&*m, state).map_err(|e| e.to_string())?;
        m.load_state(&state).map_err(|e| e.to_string())?;
        let (process_mode, max_block) = match mode {
            ZipperMode::Realtime { .. } => (ProcessMode::Realtime, 1024u32),
            ZipperMode::Offline => (ProcessMode::Offline, 4096),
        };
        m.activate(&ActivateConfig {
            sample_rate: ZIPPER_SAMPLE_RATE,
            max_block,
            mode: process_mode,
            layout: ChannelLayout::MONO,
        })
        .map_err(|e| e.to_string())?;
        let events: Vec<(usize, f64)> = match mv {
            ZipperMove::Up | ZipperMove::Down => {
                vec![(ZIPPER_CHANGE_AT, p.from_normalized(to))]
            }
            ZipperMove::Drag => (0..ZIPPER_DRAG_EVENTS)
                .map(|i| {
                    let pos = ZIPPER_CHANGE_AT
                        + (i as f64 * ZIPPER_DRAG_INTERVAL_MS / 1000.0 * ZIPPER_SAMPLE_RATE).round()
                            as usize;
                    let t = 0.25 + 0.5 * i as f64 / (ZIPPER_DRAG_EVENTS - 1) as f64;
                    (pos, p.from_normalized(t))
                })
                .collect(),
        };
        let input = zipper_signal();
        let mut output = vec![0.0f32; input.len()];
        let mut rng = TestRng::new(match mode {
            ZipperMode::Realtime { seed } => seed,
            ZipperMode::Offline => 0,
        });
        let mut out_events = OutputEvents::with_capacity(DEFAULT_EVENT_CAPACITY);
        let mut block_events = Vec::with_capacity(ZIPPER_DRAG_EVENTS);
        let (mut pos, mut next_event) = (0usize, 0usize);
        while pos < input.len() {
            let n = match mode {
                ZipperMode::Realtime { .. } => rng.range_u32(1, 1024) as usize,
                ZipperMode::Offline => 4096,
            }
            .min(input.len() - pos);
            block_events.clear();
            while let Some(&(at, value)) = events.get(next_event)
                && at < pos + n
            {
                block_events.push(ParamEvent {
                    offset: (at - pos) as u32,
                    id: p.id,
                    value,
                });
                next_event += 1;
            }
            out_events.clear();
            let mut ctx = ProcessContext::new(
                n as u32,
                pos as u64,
                Transport {
                    playing: true,
                    position_samples: Some(pos as u64),
                },
                &block_events,
                &mut out_events,
            );
            m.process(
                &mut ctx,
                &[&input[pos..pos + n]],
                &mut [&mut output[pos..pos + n]],
            );
            pos += n;
        }
        let latency = m.latency_samples() as usize;
        m.deactivate();
        let window = ZipperWindow {
            start: ZIPPER_CHANGE_AT,
            end: events.last().map_or(ZIPPER_CHANGE_AT, |e| e.0),
            t_s_ms: f64::from(p.smoothing_ms),
        };
        Ok((output, window, latency))
    }

    /// Renders and analyzes one run.
    ///
    /// # Errors
    /// See [`render`](Self::render) and [`analyze_zipper`].
    pub fn run(&self, mv: ZipperMove, mode: ZipperMode) -> Result<ZipperReport, String> {
        let (out, window, latency) = self.render(mv, mode)?;
        analyze_zipper(&out, latency, ZIPPER_SAMPLE_RATE, window)
    }

    /// Every run SPEC-012 AC-5 requires: up, down and drag, each in realtime (`seed`) and
    /// offline mode. Returns `(label, report)` pairs.
    ///
    /// # Errors
    /// See [`run`](Self::run).
    pub fn run_all(&self, seed: u64) -> Result<Vec<(String, ZipperReport)>, String> {
        let mut out = Vec::new();
        for mode in [ZipperMode::Realtime { seed }, ZipperMode::Offline] {
            for mv in [ZipperMove::Up, ZipperMove::Down, ZipperMove::Drag] {
                out.push((format!("{mv:?} {mode:?}"), self.run(mv, mode)?));
            }
        }
        Ok(out)
    }

    /// [`run_all`](Self::run_all), panicking unless every run passes.
    ///
    /// # Panics
    /// On an error or a failing run.
    pub fn assert_passes(&self, seed: u64) -> Vec<(String, ZipperReport)> {
        let reports = self.run_all(seed).unwrap_or_else(|e| panic!("zipper: {e}"));
        let failed: Vec<String> = reports
            .iter()
            .filter(|(_, r)| !r.pass)
            .map(|(l, r)| format!("{l}: {r}"))
            .collect();
        assert!(failed.is_empty(), "§4.3 failures:\n{}", failed.join("\n"));
        reports
    }
}
