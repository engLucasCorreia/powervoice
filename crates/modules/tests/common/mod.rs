//! Shared drivers and signals for the Dynamics / Noise Gate tests (S3-02).
#![allow(dead_code)]

use std::f64::consts::{PI, TAU};

use vox_module_api::test_util::{
    TestRng, ZIPPER_CHANGE_AT, ZIPPER_DRAG_EVENTS, ZIPPER_DRAG_INTERVAL_MS, ZIPPER_SAMPLE_RATE,
    ZipperReport, ZipperWindow, analyze_zipper, no_alloc,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, Module, OutputEvents, ParamEvent,
    ParamId, ProcessContext, ProcessMode, Telemetry, Transport, prepare_state,
};

/// Builds a module.
pub type Make = fn() -> Box<dyn Module>;

/// A scheduled parameter change: (input sample, id, plain value).
pub type Change = (usize, ParamId, f64);

/// 1 − e⁻¹: the fraction of a step a one-pole covers in one time constant.
pub const TAU_FRACTION: f64 = 0.632_120_558_828_557_7;

pub fn lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

pub fn to_db(x: f64) -> f64 {
    20.0 * x.log10()
}

/// A fresh module with `settings` (key, plain value) loaded; inactive.
pub fn new_with(make: Make, settings: &[(&str, f64)]) -> Box<dyn Module> {
    let mut m = make();
    let mut state = m.save_state().expect("save_state");
    for (k, v) in settings {
        assert!(state.params.contains_key(*k), "unknown parameter key `{k}`");
        state.params.insert((*k).to_owned(), *v);
    }
    let state = prepare_state(&*m, state).expect("prepare_state");
    m.load_state(&state).expect("load_state");
    m
}

pub fn activated_with(
    make: Make,
    settings: &[(&str, f64)],
    sample_rate: f64,
    mode: ProcessMode,
    max_block: u32,
) -> Box<dyn Module> {
    let mut m = new_with(make, settings);
    m.activate(&ActivateConfig {
        sample_rate,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    })
    .expect("activate");
    m
}

/// Offline, `max_block` 4096.
pub fn activated(make: Make, settings: &[(&str, f64)], sample_rate: f64) -> Box<dyn Module> {
    activated_with(make, settings, sample_rate, ProcessMode::Offline, 4096)
}

/// Block partition of a render.
#[derive(Clone, Copy, Debug)]
pub enum Blocks {
    /// Every block `n` frames (the last one shorter).
    Fixed(usize),
    /// Seeded random sizes 1…`max`, with ~8 % zero-length parameter flushes.
    Random { seed: u64, max: usize },
}

pub fn render(m: &mut dyn Module, x: &[f32], changes: &[Change], blocks: Blocks) -> Vec<f32> {
    render_observed(m, x, changes, blocks, |_, _| {})
}

/// Renders `x` (every `process` under the allocation checker); `observe(start, len)` runs after
/// each block, e.g. to read telemetry.
pub fn render_observed(
    m: &mut dyn Module,
    x: &[f32],
    changes: &[Change],
    blocks: Blocks,
    mut observe: impl FnMut(usize, usize),
) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    let mut rng = TestRng::new(match blocks {
        Blocks::Random { seed, .. } => seed,
        Blocks::Fixed(_) => 0,
    });
    let mut out_events = OutputEvents::with_capacity(DEFAULT_EVENT_CAPACITY);
    let mut events: Vec<ParamEvent> = Vec::with_capacity(changes.len().max(1));
    let (mut pos, mut next) = (0usize, 0usize);
    while pos < x.len() {
        let n = match blocks {
            Blocks::Fixed(n) => n.max(1),
            Blocks::Random { max, .. } => {
                if rng.chance(0.08) {
                    0
                } else {
                    rng.range_u32(1, max as u32) as usize
                }
            }
        }
        .min(x.len() - pos);
        events.clear();
        while let Some(&(at, id, value)) = changes.get(next)
            && (at < pos + n || (n == 0 && at == pos))
        {
            events.push(ParamEvent {
                offset: at.saturating_sub(pos) as u32,
                id,
                value,
            });
            next += 1;
        }
        out_events.clear();
        let mut ctx = ProcessContext::new(
            n as u32,
            pos as u64,
            Transport {
                playing: true,
                position_samples: Some(pos as u64),
            },
            &events,
            &mut out_events,
        );
        let inputs = [&x[pos..pos + n]];
        let mut outputs = [&mut y[pos..pos + n]];
        no_alloc(|| m.process(&mut ctx, &inputs, &mut outputs)).expect("process() allocated");
        observe(pos, n);
        pos += n;
    }
    assert_eq!(next, changes.len(), "changes past the end of the signal");
    y
}

/// Reads (and so consumes) every telemetry channel.
pub fn read_all(t: &dyn Telemetry) -> Vec<f32> {
    (0..t.channels().len()).map(|i| t.read(i)).collect()
}

pub fn peak(x: &[f32]) -> f64 {
    x.iter().fold(0.0, |m, &s| m.max(f64::from(s).abs()))
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

/// FNV-1a over the sample bits.
pub fn fnv1a(x: &[f32]) -> u64 {
    x.iter()
        .flat_map(|s| s.to_bits().to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

pub fn bit_equal(a: &[f32], b: &[f32]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

/// Sine of peak level `dbfs`, phase 0 at sample 0.
pub fn sine(freq: f64, dbfs: f64, n: usize, fs: f64) -> Vec<f32> {
    sine_segments(freq, &[(dbfs, n)], fs)
}

/// One sine with continuous phase whose level steps between `(dbfs, samples)` segments.
pub fn sine_segments(freq: f64, segs: &[(f64, usize)], fs: f64) -> Vec<f32> {
    let w = TAU * freq / fs;
    let mut out = Vec::new();
    for &(dbfs, n) in segs {
        let a = lin(dbfs);
        let start = out.len();
        out.extend((start..start + n).map(|i| (a * (w * i as f64).sin()) as f32));
    }
    out
}

/// DC segments `(dbfs, samples)`; `f64::NEG_INFINITY` is silence.
pub fn dc_segments(segs: &[(f64, usize)]) -> Vec<f32> {
    segs.iter()
        .flat_map(|&(dbfs, n)| std::iter::repeat_n(lin(dbfs) as f32, n))
        .collect()
}

/// SPEC-016 §5.1 AM tone: 997 Hz carrier, sinusoidal envelope at 46.875 Hz (period 1024
/// samples at 48 kHz) swinging (linearly in amplitude) between the two peak levels.
pub fn am_tone(lo_dbfs: f64, hi_dbfs: f64, n: usize) -> Vec<f32> {
    let (lo, hi) = (lin(lo_dbfs), lin(hi_dbfs));
    let w = TAU * 997.0 / ZIPPER_SAMPLE_RATE;
    (0..n)
        .map(|i| {
            let env = (lo + hi) / 2.0 + (hi - lo) / 2.0 * (TAU * i as f64 / 1024.0).sin();
            (env * (w * i as f64).sin()) as f32
        })
        .collect()
}

/// SPEC-013/016 §5.1 keyed tone: 997 Hz at −20 dBFS, 1024 samples on / 1024 off, 2 ms
/// raised-cosine edges inside the on part.
pub fn keyed_tone(n: usize) -> Vec<f32> {
    let a = lin(-20.0);
    let w = TAU * 997.0 / ZIPPER_SAMPLE_RATE;
    let edge = 96usize;
    (0..n)
        .map(|i| {
            let p = i % 2048;
            let env = if p < edge {
                (1.0 - (PI * p as f64 / edge as f64).cos()) / 2.0
            } else if p < 1024 - edge {
                1.0
            } else if p < 1024 {
                (1.0 - (PI * (1024 - p) as f64 / edge as f64).cos()) / 2.0
            } else {
                0.0
            };
            (a * env * (w * i as f64).sin()) as f32
        })
        .collect()
}

/// Seeded white noise with peak amplitude `amp`.
pub fn white(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut rng = TestRng::new(seed);
    (0..n).map(|_| amp * rng.bipolar_f32()).collect()
}

/// §4.3 runs (up, down, drag × realtime, offline) of `param` on `signal` with `settings`.
pub fn zipper_reports(
    make: Make,
    settings: &[(&str, f64)],
    param: ParamId,
    signal: &[f32],
) -> Vec<(String, ZipperReport)> {
    let p = make()
        .params()
        .iter()
        .find(|p| p.id == param)
        .expect("parameter")
        .clone();
    let mut out = Vec::new();
    for realtime in [true, false] {
        for (label, from, to, drag) in [
            ("up", 0.25, 0.75, false),
            ("down", 0.75, 0.25, false),
            ("drag", 0.25, 0.75, true),
        ] {
            let mut s: Vec<(&str, f64)> = settings.to_vec();
            s.push((p.key.as_str(), p.from_normalized(from)));
            let (mode, max_block, blocks) = if realtime {
                (
                    ProcessMode::Realtime,
                    1024,
                    Blocks::Random {
                        seed: 0x5302_0000 ^ u64::from(param.0),
                        max: 1024,
                    },
                )
            } else {
                (ProcessMode::Offline, 4096, Blocks::Fixed(4096))
            };
            let mut m = activated_with(make, &s, ZIPPER_SAMPLE_RATE, mode, max_block);
            let changes: Vec<Change> = if drag {
                (0..ZIPPER_DRAG_EVENTS)
                    .map(|i| {
                        let at = ZIPPER_CHANGE_AT
                            + (i as f64 * ZIPPER_DRAG_INTERVAL_MS / 1000.0 * ZIPPER_SAMPLE_RATE)
                                .round() as usize;
                        let t = 0.25 + 0.5 * i as f64 / (ZIPPER_DRAG_EVENTS - 1) as f64;
                        (at, param, p.from_normalized(t))
                    })
                    .collect()
            } else {
                vec![(ZIPPER_CHANGE_AT, param, p.from_normalized(to))]
            };
            let y = render(&mut *m, signal, &changes, blocks);
            let window = ZipperWindow {
                start: ZIPPER_CHANGE_AT,
                end: changes.last().map_or(ZIPPER_CHANGE_AT, |c| c.0),
                t_s_ms: f64::from(p.smoothing_ms),
            };
            let report =
                analyze_zipper(&y, m.latency_samples() as usize, ZIPPER_SAMPLE_RATE, window)
                    .expect("analyze_zipper");
            let mode = if realtime { "realtime" } else { "offline" };
            out.push((format!("{} {label} {mode}", p.key), report));
        }
    }
    out
}

/// [`zipper_reports`], asserting every run passes; prints each report; returns the worst excess.
pub fn assert_zipper(make: Make, settings: &[(&str, f64)], param: ParamId, signal: &[f32]) -> f64 {
    let reports = zipper_reports(make, settings, param, signal);
    let mut worst = f64::NEG_INFINITY;
    for (label, r) in &reports {
        println!("§4.3 {label}: {r}");
        worst = worst.max(r.worst_excess_db);
    }
    let failed: Vec<String> = reports
        .iter()
        .filter(|(_, r)| !r.pass)
        .map(|(l, r)| format!("{l}: {r}"))
        .collect();
    assert!(failed.is_empty(), "§4.3 failures:\n{}", failed.join("\n"));
    println!("§4.3 worst excess {worst:+.1} dB");
    worst
}
