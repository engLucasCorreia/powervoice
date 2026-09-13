//! True-Peak Limiter, SPEC-017 (S3-05 lean slice + H-03 hardening): AC-1, AC-2, AC-3/AC-4 (a
//! representative subset here; the full 44.1/48/96 kHz × ceilings × input gains × look-ahead /
//! release matrix is `ac3_ac4_full_matrix`, `#[ignore]`d, run by `just test-big` in release),
//! AC-6, AC-7, AC-8, AC-9, AC-10, AC-11 (offline and realtime), AC-12, AC-13, AC-14, AC-15,
//! AC-16 (short timing run here; 60 s in `ac16_full_timing`, `#[ignore]`d). AC-17 is
//! `just bench` (`benches/true_peak_limiter.rs`).
//!
//! Run with `--nocapture` to see the measured numbers.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time

use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use vox_dsp::true_peak::{
    DETECTOR_DELAY, REQUIREMENT_DELAY, TruePeakDetector, TruePeakLimiterCore, lookahead_samples,
    release_coeff,
};
use vox_module_api::test_util::{
    ModuleTestHost, TestRng, ZIPPER_CHANGE_AT, ZIPPER_DRAG_EVENTS, ZIPPER_DRAG_INTERVAL_MS,
    ZIPPER_FREQ_HZ, ZIPPER_LEN, ZIPPER_SAMPLE_RATE, ZipperMove, ZipperReport, ZipperTest,
    ZipperWindow, analyze_zipper, no_alloc,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, ExtensionId, HostRequest, Module, ModuleFactory, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessMode, Tail, Taper, TelemetryKind,
    Transport, Unit, prepare_state, response_curve, telemetry,
};
use vox_modules::{TruePeakLimiter, TruePeakLimiterFactory, builtin_factories};
use vox_testkit::bandlimit::{bandlimited_clicks, fade_and_pad, kaiser_lowpass};
use vox_testkit::measure::loudness;
use vox_testkit::prng::Pcg32;
use vox_testkit::signal;
use vox_testkit::true_peak::{
    true_peak_4x_dbtp, true_peak_reference_dbtp, true_peak_reference_range_dbtp,
};

vox_module_api::install_test_allocator!();

const SR: u32 = 48_000;
const FS: f64 = 48_000.0;
const CEILING_DBTP: f64 = -1.0;
const INPUT: ParamId = TruePeakLimiter::INPUT_GAIN_DB;
const CEILING: ParamId = TruePeakLimiter::CEILING_DBTP;
const RELEASE: ParamId = TruePeakLimiter::RELEASE_MS;
const LOOKAHEAD: ParamId = TruePeakLimiter::LOOKAHEAD_MS;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// An activated limiter: defaults with `overrides` applied.
fn limiter_in(
    rate: f64,
    overrides: &[(ParamId, f64)],
    mode: ProcessMode,
    max_block: u32,
) -> Box<dyn Module> {
    let mut m: Box<dyn Module> = Box::new(TruePeakLimiter::new());
    let mut state = m.save_state().unwrap();
    for &(id, v) in overrides {
        let key = m.params().iter().find(|p| p.id == id).unwrap().key.clone();
        state.params.insert(key, v);
    }
    let state = prepare_state(&*m, state).unwrap();
    m.load_state(&state).unwrap();
    m.activate(&ActivateConfig {
        sample_rate: rate,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    })
    .unwrap();
    m
}

fn limiter(rate: f64, overrides: &[(ParamId, f64)]) -> Box<dyn Module> {
    limiter_in(rate, overrides, ProcessMode::Offline, 4096)
}

/// One `process()` call under the allocation checker; returns whether Restart was requested.
fn block(
    m: &mut dyn Module,
    x: &[f32],
    y: &mut [f32],
    events: &[ParamEvent],
    pos: usize,
    out_events: &mut OutputEvents,
) -> bool {
    out_events.clear();
    no_alloc(|| {
        let mut ctx = ProcessContext::new(
            x.len() as u32,
            pos as u64,
            Transport {
                playing: true,
                position_samples: Some(pos as u64),
            },
            events,
            out_events,
        );
        m.process(&mut ctx, &[x], &mut [y]);
        ctx.requested(HostRequest::Restart)
    })
    .expect("process() allocated")
}

/// Renders `x` with events at absolute input positions and block sizes from `next_block`
/// (0 = a zero-length parameter flush, which carries the events at its position).
fn render_with(
    m: &mut dyn Module,
    x: &[f32],
    events: &[(usize, ParamId, f64)],
    mut next_block: impl FnMut() -> usize,
) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    let mut out_events = OutputEvents::with_capacity(64);
    let mut block_events: Vec<ParamEvent> = Vec::with_capacity(events.len().max(1));
    let (mut pos, mut next) = (0usize, 0usize);
    while pos < x.len() {
        let n = next_block().min(x.len() - pos);
        block_events.clear();
        while let Some(&(at, id, value)) = events.get(next)
            && (at < pos + n || (n == 0 && at == pos))
        {
            block_events.push(ParamEvent {
                offset: (at - pos) as u32,
                id,
                value,
            });
            next += 1;
        }
        block(
            m,
            &x[pos..pos + n],
            &mut y[pos..pos + n],
            &block_events,
            pos,
            &mut out_events,
        );
        pos += n;
    }
    y
}

fn offline(m: &mut dyn Module, x: &[f32], events: &[(usize, ParamId, f64)]) -> Vec<f32> {
    render_with(m, x, events, || 4096)
}

fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

fn sample_peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |a, v| a.max(v.abs()))
}

fn scale_to(x: &[f32], factor: f64) -> Vec<f32> {
    x.iter().map(|&v| (f64::from(v) * factor) as f32).collect()
}

fn peak_normalize(x: &[f32], peak_dbfs: f64) -> Vec<f32> {
    let p = f64::from(sample_peak(x));
    scale_to(x, 10f64.powf(peak_dbfs / 20.0) / p)
}

fn rms_normalize(x: &[f32], rms_dbfs: f64) -> Vec<f32> {
    let rms = (x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    scale_to(x, 10f64.powf(rms_dbfs / 20.0) / rms)
}

/// 19.5 kHz Kaiser low-pass (1 kHz transition, 110 dB): content ≤ 20 kHz.
fn band_limit_at(x: &[f32], rate: u32) -> Vec<f32> {
    kaiser_lowpass(x, 19_500.0, 1_000.0, 110.0, rate).unwrap()
}

fn band_limit(x: &[f32]) -> Vec<f32> {
    band_limit_at(x, SR)
}

/// 5 ms fades and 50 ms of silence on each side (SPEC-017 §5).
fn prep_at(x: &[f32], rate: u32) -> Vec<f32> {
    fade_and_pad(x, 5.0, 50.0, rate)
}

fn prep(x: &[f32]) -> Vec<f32> {
    prep_at(x, SR)
}

fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// The fs/4 pattern `+a +a −a −a` (true peak a·√2).
fn fs4_square(a: f32, len: usize) -> Vec<f32> {
    (0..len).map(|i| if i % 4 < 2 { a } else { -a }).collect()
}

// ---------------------------------------------------------------------------------------------
// AC-1 schema, identity, host obligations
// ---------------------------------------------------------------------------------------------

#[test]
fn ac1_descriptor_params_and_telemetry() {
    let f = TruePeakLimiterFactory::new();
    let d = f.descriptor();
    assert_eq!(d.id, "org.powervoice.true-peak-limiter");
    assert_eq!(d.version.to_string(), "1.0.0");
    assert_eq!(d.state_format_version, 1);
    assert_eq!(d.features, ["limiter", "mastering", "mono"]);
    assert_eq!(d.name.text, "True-Peak Limiter");
    assert_eq!(d.name.key.as_deref(), Some("module.true_peak_limiter.name"));

    let m = f.create().unwrap();
    let p = m.params();
    assert_eq!(p.len(), 4);
    let expect = [
        (0, "input_gain_db", Unit::Db, -12.0, 24.0, 0.0, 1, 100.0),
        (1, "ceiling_dbtp", Unit::Dbtp, -12.0, 0.0, -1.0, 1, 20.0),
        (2, "release_ms", Unit::Ms, 10.0, 1000.0, 100.0, 0, 0.0),
        (3, "lookahead_ms", Unit::Ms, 1.0, 10.0, 5.0, 1, 0.0),
    ];
    for (info, (id, key, unit, min, max, default, decimals, smoothing)) in p.iter().zip(expect) {
        assert_eq!(info.id, ParamId(id));
        assert_eq!(info.key, key);
        assert_eq!(info.unit, unit);
        assert_eq!((info.min, info.max, info.default), (min, max, default));
        assert_eq!(info.decimals, decimals);
        assert_eq!(info.smoothing_ms, smoothing, "{key}");
        assert!(info.flags.contains(ParamFlags::AUTOMATABLE));
        assert!(!info.flags.contains(ParamFlags::HIDDEN));
    }
    for i in 0..2 {
        assert_eq!(
            p[i].taper,
            Taper::Db {
                neg_inf_at_min: false
            }
        );
        assert_eq!(p[i].step, None);
    }
    assert_eq!(p[2].taper, Taper::Log);
    assert_eq!(p[3].taper, Taper::Linear);
    assert_eq!(p[3].step, Some(0.5));
    assert!(p[3].flags.contains(ParamFlags::STEPPED));
    assert_eq!(p[1].value_to_text(-1.0), "-1.0 dBTP");

    let mut m = limiter(FS, &[]);
    let t = telemetry(&*m).expect("Telemetry extension");
    let ch = t.channels();
    assert_eq!(ch.len(), 1);
    assert_eq!(ch[0].id, 0);
    assert_eq!(ch[0].key, "gain_reduction_db");
    assert_eq!(ch[0].kind, TelemetryKind::GainReduction);
    assert_eq!(ch[0].unit, Unit::Db);
    assert_eq!((ch[0].min, ch[0].max), (-24.0, 0.0));
    assert_eq!(t.read(0), 0.0);
    assert!(response_curve(&*m).is_none());
    assert!(m.extension(ExtensionId::NoiseProfile).is_none());
    assert_eq!(m.tail(), Tail::Samples(0));
    m.deactivate();

    assert!(
        builtin_factories()
            .iter()
            .any(|f| f.descriptor().id == TruePeakLimiter::ID)
    );
}

#[test]
fn ac1_passes_module_test_host() {
    let report = ModuleTestHost::new(|| Box::new(TruePeakLimiter::new()))
        .allow_delayed_effect(INPUT)
        .allow_delayed_effect(CEILING)
        .allow_delayed_effect(RELEASE)
        .allow_delayed_effect(LOOKAHEAD)
        .assert_passes();
    assert!(report.alloc_checked);
    assert!(report.zero_length_blocks > 0);
    for probe in &report.first_effect {
        println!("first effect: {probe:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// AC-2 latency
// ---------------------------------------------------------------------------------------------

#[test]
fn ac2_latency_is_exact_and_an_impulse_stays_aligned() {
    // L + D + 1 (SPEC-017 Amendment 2).
    let table = [
        (44_100.0, 1.0, 61),
        (44_100.0, 5.0, 239),
        (44_100.0, 10.0, 459),
        (48_000.0, 1.0, 65),
        (48_000.0, 5.0, 257),
        (48_000.0, 10.0, 497),
        (96_000.0, 1.0, 113),
        (96_000.0, 5.0, 497),
        (96_000.0, 10.0, 977),
    ];
    for (rate, ms, latency) in table {
        let mut m = limiter(rate, &[(LOOKAHEAD, ms)]);
        assert_eq!(m.latency_samples(), latency, "{rate} Hz / {ms} ms");
        let latency = latency as usize;
        let mut x = vec![0.0f32; 1_000 + latency + 300];
        x[1_000] = 0.25;
        let y = offline(&mut *m, &x, &[]);
        for (i, &v) in y.iter().enumerate() {
            let want: f32 = if i == 1_000 + latency { 0.25 } else { 0.0 };
            assert_eq!(
                v.to_bits(),
                want.to_bits(),
                "{rate} Hz / {ms} ms, sample {i}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-3 / AC-4 ceiling guarantee on the stress set (48 kHz, ceiling −1, input gain 0 and +12)
// ---------------------------------------------------------------------------------------------

/// One AC-3/AC-4 run's settings: ceiling (dBTP), input gain (dB), release and look-ahead (ms).
#[derive(Clone, Copy, Debug)]
struct Setting {
    ceiling: f64,
    gain: f64,
    release: f64,
    lookahead: f64,
}

impl Setting {
    /// Default release and look-ahead.
    const fn new(ceiling: f64, gain: f64) -> Self {
        Self::with(ceiling, gain, 100.0, 5.0)
    }

    const fn with(ceiling: f64, gain: f64, release: f64, lookahead: f64) -> Self {
        Self {
            ceiling,
            gain,
            release,
            lookahead,
        }
    }
}

/// Output peaks of one run.
#[derive(Clone, Copy, Debug)]
struct Measured {
    /// 16× reference true peak, dBTP.
    reference: f64,
    /// Plain 4× reading, dBTP.
    four: f64,
    /// Sample peak, linear.
    sample_peak: f32,
    /// ebur128's true peak (44.1/48 kHz only), dBTP.
    ebur: Option<f64>,
    /// Every output sample is finite.
    finite: bool,
}

fn measure_run(rate: u32, x: &[f32], s: Setting) -> Measured {
    let mut m = limiter(
        f64::from(rate),
        &[
            (INPUT, s.gain),
            (CEILING, s.ceiling),
            (RELEASE, s.release),
            (LOOKAHEAD, s.lookahead),
        ],
    );
    let y = offline(&mut *m, x, &[]);
    Measured {
        reference: true_peak_reference_dbtp(&y),
        four: true_peak_4x_dbtp(&y),
        sample_peak: sample_peak(&y),
        ebur: (rate != 96_000).then(|| loudness(&y, 1, rate).unwrap().true_peak_dbtp),
        finite: y.iter().all(|v| v.is_finite()),
    }
}

/// AC-3 (reference and plain 4× ≤ ceiling + 0.10 dB, sample peak ≤ ceiling + 1 f32 ULP) and
/// AC-4 (ebur128 ≤ ceiling + 0.25 dB) violations of one run.
fn violations(rate: u32, name: &str, s: Setting, m: &Measured) -> Vec<String> {
    let c = db_to_lin(s.ceiling);
    let sample_bound = f32::from_bits((c as f32).to_bits() + 1);
    let tag = format!("{rate} Hz {name} {s:?}");
    let mut v = Vec::new();
    if !m.finite {
        v.push(format!("{tag}: non-finite output"));
    }
    if m.reference > s.ceiling + 0.10 {
        v.push(format!("{tag}: reference {}", m.reference));
    }
    if m.four > s.ceiling + 0.10 {
        v.push(format!("{tag}: 4x {}", m.four));
    }
    if m.sample_peak > sample_bound {
        v.push(format!("{tag}: sample peak {}", m.sample_peak));
    }
    if let Some(e) = m.ebur
        && e > s.ceiling + 0.25
    {
        v.push(format!("{tag}: ebur128 {e}"));
    }
    v
}

fn print_run(rate: u32, name: &str, s: Setting, m: &Measured) {
    let ebur = m
        .ebur
        .map_or(String::new(), |e| format!(" ebur128 {e:+.4}"));
    println!(
        "AC-3 {rate} Hz {name:<22} c {:+5.1} g {:+3} rel {:>4} la {:>4} | out ref {:+.4} \
         (+{:.4}) 4x {:+.4} sample {:+.4}{ebur} dBTP",
        s.ceiling,
        s.gain,
        s.release,
        s.lookahead,
        m.reference,
        m.reference - s.ceiling,
        m.four,
        db(f64::from(m.sample_peak)),
    );
}

/// Renders `x` at `rate` with `s` and asserts AC-3/AC-4.
fn check_run(rate: u32, name: &str, x: &[f32], s: Setting) -> Measured {
    let m = measure_run(rate, x, s);
    print_run(rate, name, s, &m);
    let v = violations(rate, name, s, &m);
    assert!(v.is_empty(), "{v:#?}");
    m
}

/// The lean-slice check: 48 kHz, ceiling −1, input gains 0 and +12.
fn check_ceiling(name: &str, x: &[f32]) {
    for gain in [0.0, 12.0] {
        check_run(SR, name, x, Setting::new(CEILING_DBTP, gain));
    }
}

#[test]
fn ac3_fs4_squares_and_sweep() {
    let square: Vec<f32> = (0..24_000)
        .map(|i| if i % 4 < 2 { 1.0 } else { -1.0 })
        .collect();
    check_ceiling("fs/4 square a=1", &prep(&square));
    let sweep = signal::log_sweep(20.0, 20_000.0, 0.0, 2.0, SR).unwrap();
    check_ceiling("log sweep 20-20k", &prep(&sweep));
}

#[test]
fn ac3_sines() {
    let mut rng = Pcg32::new(0x5E17_0017, 1);
    for f in [
        997.0, 5_000.0, 10_000.0, 12_000.0, 15_000.0, 18_000.0, 19_900.0,
    ] {
        let phase = 360.0 * rng.next_f64();
        let x = signal::sine_with_phase(f, 0.0, phase, 0.5, SR).unwrap();
        check_ceiling(&format!("sine {f} Hz"), &prep(&x));
    }
}

#[test]
fn ac3_noise() {
    let white = signal::white_noise(0x17, -6.0, 1.0, SR).unwrap();
    check_ceiling(
        "white -6 dBFS RMS",
        &prep(&rms_normalize(&band_limit(&white), -6.0)),
    );
    let pink = signal::pink_noise(0x18, -14.0, 1.0, SR).unwrap();
    check_ceiling(
        "pink -14 dBFS RMS",
        &prep(&rms_normalize(&band_limit(&pink), -14.0)),
    );
}

#[test]
fn ac3_clicks() {
    let sparse = bandlimited_clicks(0x19, 40, 0.5, 1.0, 19_500.0, 1.0, SR).unwrap();
    check_ceiling("clicks sparse (40)", &prep(&sparse));
    let dense = bandlimited_clicks(0x1A, 2_000, 0.5, 1.0, 19_500.0, 1.0, SR).unwrap();
    check_ceiling("clicks dense (2000)", &prep(&dense));
}

#[test]
fn ac3_bursts_and_voice() {
    let bursts = signal::tone_bursts(0x1B, 997.0, 0.0, None, 0.05, 0.05, 1.0, SR).unwrap();
    check_ceiling("997 Hz bursts", &prep(&band_limit(&bursts)));
    let voice = signal::voice_like(0x1C, 220.0, -6.0, 20.0, 0.25, 0.15, 1.5, SR).unwrap();
    check_ceiling(
        "voice_like -6 dBFS pk",
        &prep(&peak_normalize(&band_limit(&voice), -6.0)),
    );
}

// ---------------------------------------------------------------------------------------------
// AC-6 no over-limiting, AC-14 telemetry
// ---------------------------------------------------------------------------------------------

#[test]
fn ac6_steady_sines_sit_at_the_ceiling() {
    for (f, level, gain, tol) in [
        (997.0, 0.0, 0.0, 0.05),
        (5_000.0, 0.0, 0.0, 0.05),
        (10_000.0, 0.0, 0.0, 0.05),
        (15_000.0, 0.0, 0.0, 0.05),
        (19_900.0, 0.0, 0.0, 0.05),
        (997.0, -6.0, 6.0, 0.02),
    ] {
        let x = signal::sine_with_phase(f, level, 30.0, 0.8, SR).unwrap();
        let mut m = limiter(FS, &[(INPUT, gain)]);
        let latency = m.latency_samples() as usize;
        let t = telemetry(&*m).unwrap();
        let split = 24_000;
        let mut y = offline(&mut *m, &x[..split], &[]);
        t.read(0);
        y.extend(offline(&mut *m, &x[split..split + 4_800], &[]));
        let gr = f64::from(t.read(0));
        y.extend(offline(&mut *m, &x[split + 4_800..], &[]));
        let tp = true_peak_reference_range_dbtp(&y, split + latency..split + latency + 4_800);
        println!(
            "AC-6 {f:>7} Hz {level:+} dBFS +{gain} dB: steady out {tp:+.4} dBTP, GR {gr:+.3} dB"
        );
        assert!((tp - CEILING_DBTP).abs() <= tol, "{f} Hz: {tp}");
        if level == 0.0 {
            assert!((gr + 1.0).abs() <= 0.1, "{f} Hz: telemetry {gr}");
        }
    }
}

#[test]
fn ac14_telemetry_holds_the_minimum_until_read() {
    let mut m = limiter(FS, &[(CEILING, -3.0)]);
    let t = telemetry(&*m).unwrap();
    let latency = m.latency_samples() as usize;
    let g_in = 1.0;
    let x = signal::sine_with_phase(997.0, 0.0, 10.0, 1.0, SR).unwrap();
    let y = offline(&mut *m, &x[..24_000], &[]);
    let _ = t.read(0);
    // A steady limited block: telemetry equals the minimum applied gain (from y / x) ± 0.01 dB.
    let n = 4_800;
    let mut yb = vec![0.0f32; n];
    let mut oe = OutputEvents::with_capacity(8);
    block(
        &mut *m,
        &x[24_000..24_000 + n],
        &mut yb,
        &[],
        24_000,
        &mut oe,
    );
    let min_gain = (0..n)
        .filter_map(|i| {
            let u = g_in * f64::from(x[24_000 + i - latency]);
            (u.abs() > 0.5).then(|| db(f64::from(yb[i]) / u))
        })
        .fold(f64::INFINITY, f64::min);
    // No read in between: a zero-length flush (writes 0.0) doesn't lose the held minimum.
    block(&mut *m, &[], &mut [], &[], 24_000 + n, &mut oe);
    let gr = f64::from(t.read(0));
    println!("AC-14 telemetry {gr:+.4} dB, min applied gain {min_gain:+.4} dB");
    assert!((gr - min_gain).abs() <= 0.01, "{gr} vs {min_gain}");
    assert!((gr + 3.0).abs() <= 0.1);
    assert!(y.iter().all(|v| v.is_finite()));
    // Silence: back to exactly 0.0, never positive.
    let silence = vec![0.0f32; 96_000];
    offline(&mut *m, &silence, &[]);
    let _ = t.read(0);
    offline(&mut *m, &silence[..1_000], &[]);
    assert_eq!(t.read(0).to_bits(), 0.0f32.to_bits());
}

// ---------------------------------------------------------------------------------------------
// AC-8 transparency
// ---------------------------------------------------------------------------------------------

#[test]
fn ac8_below_ceiling_is_bit_exact_after_latency() {
    let pink = signal::pink_noise(0x28, -20.0, 1.5, SR).unwrap();
    let voice = signal::voice_like(0x29, 180.0, -6.0, 25.0, 0.3, 0.2, 1.5, SR).unwrap();
    for (name, x) in [
        ("pink -20 dBFS RMS", pink),
        ("voice_like -6 dBFS pk", peak_normalize(&voice, -6.0)),
    ] {
        let tp = true_peak_reference_dbtp(&x);
        assert!(tp <= -1.1, "{name}: precondition, reference TP {tp}");
        let mut m = limiter(FS, &[]);
        let latency = m.latency_samples() as usize;
        let mut input = x.clone();
        input.extend(std::iter::repeat_n(0.0, latency));
        let y = offline(&mut *m, &input, &[]);
        assert!(y[..latency].iter().all(|&v| v.to_bits() == 0));
        for i in 0..x.len() {
            assert_eq!(
                y[i + latency].to_bits(),
                x[i].to_bits(),
                "{name}: sample {i}"
            );
        }
        assert_eq!(telemetry(&*m).unwrap().read(0).to_bits(), 0.0f32.to_bits());
        println!("AC-8 {name}: input TP {tp:+.3} dBTP, bit-exact");
    }
}

// ---------------------------------------------------------------------------------------------
// AC-9 distortion
// ---------------------------------------------------------------------------------------------

/// THD+N in dB: residual power over fundamental power after a least-squares fit of the
/// fundamental (known frequency) and DC over `range`.
fn thd_n_db(y: &[f32], f: f64, fs: f64, range: Range<usize>) -> f64 {
    let w = std::f64::consts::TAU * f / fs;
    // Normal equations for [cos, sin, 1].
    let mut a = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for n in range.clone() {
        let basis = [(w * n as f64).cos(), (w * n as f64).sin(), 1.0];
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += basis[i] * basis[j];
            }
            b[i] += basis[i] * f64::from(y[n]);
        }
    }
    // Gaussian elimination (3×3, well conditioned over many periods).
    for col in 0..3 {
        let pivot = a[col][col];
        for row in col + 1..3 {
            let k = a[row][col] / pivot;
            for j in col..3 {
                a[row][j] -= k * a[col][j];
            }
            b[row] -= k * b[col];
        }
    }
    let mut coef = [0.0f64; 3];
    for i in (0..3).rev() {
        let s: f64 = (i + 1..3).map(|j| a[i][j] * coef[j]).sum();
        coef[i] = (b[i] - s) / a[i][i];
    }
    let residual: f64 = range
        .clone()
        .map(|n| {
            let fit = coef[0] * (w * n as f64).cos() + coef[1] * (w * n as f64).sin() + coef[2];
            (f64::from(y[n]) - fit).powi(2)
        })
        .sum::<f64>()
        / range.len() as f64;
    let fundamental = (coef[0] * coef[0] + coef[1] * coef[1]) / 2.0;
    10.0 * (residual / fundamental).log10()
}

fn limited_thd(rate: u32, f: f64, gain: f64, extra: &[(ParamId, f64)]) -> f64 {
    let fs = f64::from(rate);
    let x = signal::sine(f, -6.0, 2.0, rate).unwrap();
    let mut overrides = vec![(INPUT, gain)];
    overrides.extend_from_slice(extra);
    let mut m = limiter(fs, &overrides);
    let latency = m.latency_samples() as usize;
    let y = offline(&mut *m, &x, &[]);
    let range = (0.5 * fs) as usize + latency..(1.9 * fs) as usize;
    thd_n_db(&y, f, fs, range)
}

#[test]
fn ac9_thd_n_of_a_tone_pushed_into_the_limiter() {
    for rate in [44_100u32, 48_000, 96_000] {
        for f in [997.0, 1_000.0] {
            for gain in [6.0, 18.0] {
                let thd = limited_thd(rate, f, gain, &[]);
                println!("AC-9 {rate} Hz, {f} Hz −6 dBFS +{gain} dB: THD+N {thd:.1} dB");
                assert!(thd <= -80.0, "{rate} Hz / {f} Hz / +{gain} dB: {thd}");
            }
        }
        let bass = limited_thd(rate, 80.0, 6.0, &[]);
        println!("AC-9 {rate} Hz, 80 Hz −6 dBFS +6 dB: THD+N {bass:.1} dB");
        assert!(bass <= -55.0, "{rate} Hz, 80 Hz: {bass}");
    }
    // Informative (printed, not gated).
    let hum = limited_thd(48_000, 50.0, 6.0, &[]);
    let fast = limited_thd(48_000, 100.0, 6.0, &[(LOOKAHEAD, 1.0), (RELEASE, 10.0)]);
    println!(
        "AC-9 informative, 48000 Hz: 50 Hz +6 dB {hum:.1} dB (spec sim −44); 100 Hz +6 dB with \
         look-ahead 1 ms / release 10 ms {fast:.1} dB (spec sim −28)"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-10 release timing (and AC-8's return to unity)
// ---------------------------------------------------------------------------------------------

#[test]
fn ac10_release_is_twelve_db_per_release_time_and_returns_to_unity() {
    // (release, accepted −12.5 → −0.5 dB time in ms): release ± 2 %; 10 ms → 10.0–12.0 ms.
    for (release, lo, hi) in [
        (100.0, 98.0, 102.0),
        (1_000.0, 980.0, 1_020.0),
        (10.0, 10.0, 12.0),
    ] {
        let gain_db = 12.0;
        let g_in = db_to_lin(gain_db);
        let drop = SR as usize; // 1 s loud, then −40 dBFS
        let total = drop + (FS * (release / 1000.0 * 1.25 + 0.6)) as usize;
        let w = std::f64::consts::TAU * 997.0 / FS;
        let amp = |n: usize| if n < drop { 1.0 } else { 0.01 };
        let x: Vec<f32> = (0..total)
            .map(|n| (amp(n) * (w * n as f64).sin()) as f32)
            .collect();
        let mut m = limiter(FS, &[(INPUT, gain_db), (RELEASE, release)]);
        let latency = m.latency_samples() as usize;
        let l = latency - REQUIREMENT_DELAY;
        let y = offline(&mut *m, &x, &[]);
        // Applied gain (dB) at output samples whose delayed input is not near a zero crossing.
        let gains: Vec<(usize, f64)> = (latency..total)
            .filter_map(|n| {
                let src = n - latency;
                let u = g_in * f64::from(x[src]);
                (f64::from(x[src]).abs() >= 0.5 * amp(src)).then(|| (n, db(f64::from(y[n]) / u)))
            })
            .collect();
        let release_start = drop + latency;
        let held_max = gains
            .iter()
            .filter(|(n, _)| (release_start - 4_800..release_start - l).contains(n))
            .map(|&(_, g)| g)
            .fold(f64::NEG_INFINITY, f64::max);
        let crossing = |level: f64| {
            let i = gains
                .iter()
                .position(|&(n, g)| n >= release_start - l && g >= level)
                .unwrap();
            let ((n0, g0), (n1, g1)) = (gains[i - 1], gains[i]);
            n0 as f64 + (level - g0) / (g1 - g0) * (n1 - n0) as f64
        };
        let (from, to) = (crossing(-12.5), crossing(-0.5));
        let t = (to - from) / FS * 1000.0;
        println!(
            "AC-10 release {release} ms: held GR {held_max:+.3} dB, −12.5 → −0.5 dB in {t:.2} ms"
        );
        assert!(
            held_max <= -12.5,
            "release {release}: gain rose before the look-ahead passed the last peak"
        );
        assert!((lo..=hi).contains(&t), "release {release}: took {t} ms");
        // Exactly unity no later than release/12 + look-ahead after the −0.5 dB crossing: from
        // there on the output is the delayed, input-gained input, bit for bit (AC-8).
        let unity_from =
            to.ceil() as usize + ((release / 12.0 + 5.0) / 1000.0 * FS).ceil() as usize;
        assert!(unity_from + 4_800 < total);
        for n in unity_from..total {
            let want = (g_in * f64::from(x[n - latency])) as f32;
            assert_eq!(
                y[n].to_bits(),
                want.to_bits(),
                "release {release}: sample {n}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-11 no zipper noise (SPEC-012 §4.3), offline
// ---------------------------------------------------------------------------------------------

/// A §4.3 run of `param` on a 997 Hz tone at `level_dbfs` (the standard −20 dBFS, or a
/// limiting-active substitute), offline or realtime (random blocks, 0-length flushes). The
/// change window is widened by the latency: the ceiling/gain move reaches the output between S
/// (the gain computer anticipates) and end + latency + T_s (the moved sample itself: the ceiling
/// is attached at k + L + 1, the input gain applied at k + latency), in latency-shifted time.
fn zipper(param: ParamId, level_dbfs: f64, mv: ZipperMove, mode: ProcessMode) -> ZipperReport {
    let proto = TruePeakLimiter::new();
    let p = proto
        .params()
        .iter()
        .find(|p| p.id == param)
        .unwrap()
        .clone();
    let (from, to) = match mv {
        ZipperMove::Up | ZipperMove::Drag => (0.25, 0.75),
        ZipperMove::Down => (0.75, 0.25),
    };
    let mut m = limiter_in(
        ZIPPER_SAMPLE_RATE,
        &[(param, p.from_normalized(from))],
        mode,
        1024,
    );
    let events: Vec<(usize, ParamId, f64)> = match mv {
        ZipperMove::Up | ZipperMove::Down => {
            vec![(ZIPPER_CHANGE_AT, param, p.from_normalized(to))]
        }
        ZipperMove::Drag => (0..ZIPPER_DRAG_EVENTS)
            .map(|i| {
                let at = ZIPPER_CHANGE_AT
                    + (i as f64 * ZIPPER_DRAG_INTERVAL_MS / 1000.0 * ZIPPER_SAMPLE_RATE).round()
                        as usize;
                let t = 0.25 + 0.5 * i as f64 / (ZIPPER_DRAG_EVENTS - 1) as f64;
                (at, param, p.from_normalized(t))
            })
            .collect(),
    };
    let amp = 10f64.powf(level_dbfs / 20.0);
    let w = std::f64::consts::TAU * ZIPPER_FREQ_HZ / ZIPPER_SAMPLE_RATE;
    let x: Vec<f32> = (0..ZIPPER_LEN)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect();
    let y = match mode {
        ProcessMode::Offline => offline(&mut *m, &x, &events),
        _ => {
            let mut blocks = TestRng::new(0x2199 ^ u64::from(param.0));
            render_with(&mut *m, &x, &events, || match blocks.below(8) {
                0 => 0,
                1 => 1,
                _ => blocks.range_u32(1, 1024) as usize,
            })
        }
    };
    let latency = m.latency_samples() as usize;
    let window = ZipperWindow {
        start: ZIPPER_CHANGE_AT,
        end: events.last().unwrap().0 + latency,
        t_s_ms: f64::from(p.smoothing_ms),
    };
    analyze_zipper(&y, latency, ZIPPER_SAMPLE_RATE, window).unwrap()
}

const MODES: [ProcessMode; 2] = [ProcessMode::Offline, ProcessMode::Realtime];

#[test]
fn ac11_ceiling_changes_are_click_free() {
    let mut failed = Vec::new();
    for mode in MODES {
        for mv in [ZipperMove::Up, ZipperMove::Down, ZipperMove::Drag] {
            let r = zipper(CEILING, 0.0, mv, mode);
            println!("AC-11 ceiling @ 0 dBFS {mode:?} {mv:?}: {r}");
            if !r.pass {
                failed.push(format!("{mode:?} {mv:?}: {r}"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn ac11_input_gain_changes_are_click_free() {
    let mut failed = Vec::new();
    for mode in MODES {
        for level in [-20.0, -6.0, 0.0] {
            for mv in [ZipperMove::Up, ZipperMove::Down, ZipperMove::Drag] {
                let r = zipper(INPUT, level, mv, mode);
                println!("AC-11 input gain @ {level} dBFS {mode:?} {mv:?}: {r}");
                if !r.pass {
                    failed.push(format!("{level} dBFS {mode:?} {mv:?}: {r}"));
                }
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn ac11_standard_harness_passes() {
    for param in [INPUT, CEILING, RELEASE] {
        let reports =
            ZipperTest::new(|| Box::new(TruePeakLimiter::new()), param).assert_passes(0x5E17);
        for (label, r) in &reports {
            println!("AC-11 standard {param:?} {label}: {r}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-13 look-ahead change
// ---------------------------------------------------------------------------------------------

#[test]
fn ac13_lookahead_change_requests_restart_and_leaves_the_live_instance() {
    let x = signal::sine_with_phase(997.0, 0.0, 0.0, 0.1, SR).unwrap();
    let mut a = limiter(FS, &[]);
    let mut b = limiter(FS, &[]);
    let mut oe = OutputEvents::with_capacity(8);
    let (mut ya, mut yb) = (vec![0.0f32; x.len()], vec![0.0f32; x.len()]);
    let ev = ParamEvent {
        offset: 100,
        id: LOOKAHEAD,
        value: 10.0,
    };
    assert!(block(&mut *a, &x, &mut ya, &[ev], 0, &mut oe));
    assert!(!block(&mut *b, &x, &mut yb, &[], 0, &mut oe));
    assert!(ya.iter().zip(&yb).all(|(p, q)| p.to_bits() == q.to_bits()));
    assert_eq!(a.latency_samples(), 257);
    assert_eq!(a.param_value(LOOKAHEAD), Some(10.0));
    // The active value again: no restart.
    let same = ParamEvent {
        offset: 0,
        id: LOOKAHEAD,
        value: 5.0,
    };
    assert!(!block(&mut *b, &x, &mut yb, &[same], 0, &mut oe));
    // The replacement (loaded from the saved state) reports the new L + D.
    let state = a.save_state().unwrap();
    let mut fresh: Box<dyn Module> = Box::new(TruePeakLimiter::new());
    fresh.load_state(&state).unwrap();
    fresh
        .activate(&ActivateConfig {
            sample_rate: FS,
            max_block: 1024,
            mode: ProcessMode::Realtime,
            layout: ChannelLayout::MONO,
        })
        .unwrap();
    assert_eq!(fresh.latency_samples(), 497);
}

// ---------------------------------------------------------------------------------------------
// AC-15 realtime = offline, deterministic, RT-safe
// ---------------------------------------------------------------------------------------------

#[test]
fn ac15_realtime_equals_offline_and_renders_are_deterministic() {
    let voice = signal::voice_like(0x31, 200.0, -3.0, 20.0, 0.2, 0.1, 1.5, SR).unwrap();
    let pink = signal::pink_noise(0x32, -14.0, 1.5, SR).unwrap();
    let x: Vec<f32> = voice.iter().zip(&pink).map(|(a, b)| a + b).collect();
    let mut rng = TestRng::new(0xAC15);
    let events: Vec<(usize, ParamId, f64)> = (0..30)
        .map(|i| {
            let at = 1_000 + i * 2_300;
            match i % 3 {
                0 => (at, INPUT, -6.0 + 18.0 * rng.unit_f64()),
                1 => (at, CEILING, -6.0 + 6.0 * rng.unit_f64()),
                _ => (at, RELEASE, 10.0 + 990.0 * rng.unit_f64()),
            }
        })
        .collect();
    let settings = [(INPUT, 6.0)];
    let off1 = offline(&mut *limiter(FS, &settings), &x, &events);
    let off2 = offline(&mut *limiter(FS, &settings), &x, &events);
    assert!(
        off1.iter()
            .zip(&off2)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    let mut rt_module = limiter_in(FS, &settings, ProcessMode::Realtime, 1024);
    let mut blocks = TestRng::new(0x0B10C);
    let rt = render_with(&mut *rt_module, &x, &events, || match blocks.below(8) {
        0 => 0,
        1 => 1,
        _ => blocks.range_u32(1, 1024) as usize,
    });
    let max_diff = rt
        .iter()
        .zip(&off1)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let bit_exact = rt
        .iter()
        .zip(&off1)
        .all(|(a, b)| a.to_bits() == b.to_bits());
    println!("AC-15 realtime vs offline: max |diff| {max_diff:e}, bit-exact {bit_exact}");
    assert!(max_diff <= 1e-6);
    assert!(bit_exact, "the limiter is block-independent by design");
}

// ---------------------------------------------------------------------------------------------
// AC-3 / AC-4: representative subset across rates, ceilings, gains, look-ahead and release
// ---------------------------------------------------------------------------------------------

#[test]
fn ac3_subset_across_rates_ceilings_and_lookahead() {
    let mut rng = Pcg32::new(0x0AC3_005B, 7);
    // 44.1 kHz: dense clicks and a 19.9 kHz sine at extreme ceilings and gains.
    let r = 44_100;
    let clicks = prep_at(
        &bandlimited_clicks(0x3A, 2_000, 0.5, 1.0, 19_500.0, 1.0, r).unwrap(),
        r,
    );
    check_run(r, "clicks dense (2000)", &clicks, Setting::new(-0.1, 24.0));
    check_run(r, "clicks dense (2000)", &clicks, Setting::new(-12.0, 6.0));
    let phase = 360.0 * rng.next_f64();
    let sine = signal::sine_with_phase(19_900.0, 0.0, phase, 0.5, r).unwrap();
    check_run(
        r,
        "sine 19900 Hz",
        &prep_at(&sine, r),
        Setting::new(-3.0, 12.0),
    );
    // 96 kHz: a sweep pushed hard; fs/4 at the loosest ceiling.
    let r = 96_000;
    let sweep = signal::log_sweep(20.0, 20_000.0, 0.0, 1.0, r).unwrap();
    check_run(
        r,
        "log sweep 20-20k",
        &prep_at(&sweep, r),
        Setting::new(-1.0, 24.0),
    );
    let square = prep_at(&fs4_square(1.0, 24_000), r);
    check_run(r, "fs/4 square a=1", &square, Setting::new(-0.1, 0.0));
    // 48 kHz: look-ahead 1 and 10 ms, release 10 and 1 000 ms, at ceiling −1.
    let white = signal::white_noise(0x3B, -6.0, 1.0, SR).unwrap();
    let white = prep(&rms_normalize(&band_limit(&white), -6.0));
    for s in [
        Setting::with(CEILING_DBTP, 12.0, 100.0, 1.0),
        Setting::with(CEILING_DBTP, 24.0, 10.0, 1.0),
        Setting::with(CEILING_DBTP, 12.0, 1_000.0, 10.0),
    ] {
        check_run(SR, "white -6 dBFS RMS", &white, s);
    }
    let bursts = signal::tone_bursts(0x3C, 997.0, 0.0, None, 0.05, 0.05, 1.0, SR).unwrap();
    let bursts = prep(&band_limit(&bursts));
    check_run(
        SR,
        "997 Hz bursts",
        &bursts,
        Setting::with(CEILING_DBTP, 24.0, 10.0, 1.0),
    );
}

// ---------------------------------------------------------------------------------------------
// AC-7 attack before the peak (the DSP core's per-sample G is the test hook)
// ---------------------------------------------------------------------------------------------

#[test]
fn ac7_attack_starts_exactly_l_samples_before_the_first_over_ceiling_interval() {
    let c = db_to_lin(CEILING_DBTP);
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let l = lookahead_samples(5.0, rate);
        let onset = 4_000;
        let w = std::f64::consts::TAU * 997.0 / rate;
        let amp = db_to_lin(6.0);
        // Digital silence, then a +6 dBFS 997 Hz burst (f32 input, as the module sees it).
        let u: Vec<f64> = (0..onset + 12_000)
            .map(|n| {
                if n < onset {
                    0.0
                } else {
                    f64::from((amp * (w * (n - onset) as f64).sin()) as f32)
                }
            })
            .collect();
        // The first over-ceiling interval (a, a + 1) as the detector reads it: its reading at
        // push t is the interval (t − D − 1, t − D).
        let mut det = TruePeakDetector::new();
        let q: Vec<f64> = u.iter().map(|&x| det.push(x)).collect();
        let a = (0..q.len()).find(|&t| q[t] > c).unwrap() - DETECTOR_DELAY - 1;
        let mut core = TruePeakLimiterCore::new(l, release_coeff(100.0, rate), c);
        let latency = core.latency();
        let g: Vec<f64> = u.iter().map(|&x| core.process(x, c).1).collect();
        // Gain applied to input sample s (output at s + latency).
        let gain_of = |s: usize| g[s + latency];
        // The burst's first peak: the largest |u| in its first half period.
        let half = (rate / 997.0 / 2.0) as usize;
        let p = (onset..onset + half)
            .max_by(|&i, &j| u[i].abs().total_cmp(&u[j].abs()))
            .unwrap();
        println!(
            "AC-7 {rate} Hz: first over-ceiling interval ({a}, {}), onset {onset}, first peak \
             {p}, L {l}: G first drops at sample {} ({:.6}), G at the peak {:+.3} dB",
            a + 1,
            a - l,
            gain_of(a - l),
            db(gain_of(p)),
        );
        assert!(
            (0..a - l).all(|s| gain_of(s) == 1.0),
            "{rate} Hz: G moved before a − L"
        );
        assert!(gain_of(a - l) < 1.0, "{rate} Hz: G didn't drop at a − L");
        for s in a - l..p {
            assert!(
                gain_of(s + 1) <= gain_of(s),
                "{rate} Hz: attack not monotonic at sample {s}"
            );
        }
        assert!(gain_of(p) * u[p].abs() <= c * (1.0 + 1e-12));
    }
}

// ---------------------------------------------------------------------------------------------
// AC-12 parameter timing and ceiling changes
// ---------------------------------------------------------------------------------------------

#[test]
fn ac12_an_event_changes_nothing_before_event_plus_latency() {
    let voice = signal::voice_like(0x12, 200.0, -3.0, 20.0, 0.2, 0.1, 1.0, SR).unwrap();
    let settings = [(INPUT, 12.0)];
    let k = 20_011;
    let base = offline(&mut *limiter(FS, &settings), &voice, &[]);
    let latency = limiter(FS, &settings).latency_samples() as usize;
    for (param, value) in [
        (INPUT, -6.0),
        (INPUT, 24.0),
        (CEILING, -9.0),
        (CEILING, 0.0),
        (RELEASE, 10.0),
        (RELEASE, 1_000.0),
        (LOOKAHEAD, 10.0),
    ] {
        let y = offline(&mut *limiter(FS, &settings), &voice, &[(k, param, value)]);
        let first = y
            .iter()
            .zip(&base)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        println!(
            "AC-12 {param:?} → {value}: first changed output sample {first:?} (event {k}, \
             latency {latency})"
        );
        match first {
            Some(n) => assert!(n >= k + latency, "{param:?} → {value}: changed at {n}"),
            None => assert_eq!(param, LOOKAHEAD, "{param:?} → {value} never took effect"),
        }
    }
}

#[test]
fn ac12_ceiling_change_travels_with_the_audio() {
    let x = signal::sine_with_phase(997.0, 0.0, 0.0, 1.0, SR).unwrap();
    let k = 20_000;
    let mut m = limiter(FS, &[]);
    let latency = m.latency_samples() as usize;
    let l = latency - REQUIREMENT_DELAY;
    let y = offline(&mut *m, &x, &[(k, CEILING, -6.0)]);
    let ramp = (0.020 * FS).round() as usize;
    // Settled: from k + L + 20 ms + latency on, the reference true peak is ≤ −5.9 dBTP.
    let from = k + l + ramp + latency;
    let settled = true_peak_reference_range_dbtp(&y, from..x.len() - 200);
    // Each input sample's attached ceiling: −1 until k + L + 1, then the 20 ms ramp in dB.
    let attach = k + l + 1;
    let ceiling_db = |s: usize| {
        if s < attach {
            -1.0
        } else {
            -1.0 - 5.0 * ((s - attach + 1) as f64 / ramp as f64).min(1.0)
        }
    };
    let mut worst_sample = f64::NEG_INFINITY;
    for n in latency..x.len() {
        let c = ceiling_db(n - latency);
        let bound = f32::from_bits((db_to_lin(c) as f32).to_bits() + 1);
        assert!(y[n].abs() <= bound, "output {n}: {} > {bound}", y[n]);
        worst_sample = worst_sample.max(db(f64::from(y[n].abs())) - c);
    }
    // During the ramp, each 1 ms window's true peak stays ≤ the highest ceiling attached to its
    // samples + 0.10 dB.
    let mut worst_window = f64::NEG_INFINITY;
    for start in (k + latency..from + 4_800).step_by(48) {
        let tp = true_peak_reference_range_dbtp(&y, start..start + 48);
        let c = (start..=start + 48)
            .map(|n| ceiling_db(n - latency))
            .fold(f64::NEG_INFINITY, f64::max);
        worst_window = worst_window.max(tp - c);
        assert!(
            tp <= c + 0.10,
            "window at {start}: {tp} dBTP vs ceiling {c}"
        );
    }
    println!(
        "AC-12 ceiling −1 → −6 dBTP: settled {settled:+.4} dBTP; worst sample {worst_sample:+.4} \
         dB, worst 1 ms window {worst_window:+.4} dB relative to its own ceiling"
    );
    assert!(settled <= -5.9, "settled {settled}");
}

// ---------------------------------------------------------------------------------------------
// AC-16 robustness (FTZ/DAZ off: test threads never enable them)
// ---------------------------------------------------------------------------------------------

/// Median `process()` time of 256-frame blocks for white noise, f32 subnormal input (1e-40) and
/// digital silence, interleaved block by block so load affects all three alike. Also checks the
/// outputs: silence gives exact zeros; the subnormal DC is below the ceiling, so it passes
/// bit-exactly delayed (AC-8). Returns (subnormal / noise, silence / noise).
fn ac16_timing(seconds: f64) -> (f64, f64) {
    let n = (seconds * FS) as usize;
    let noise = signal::white_noise(0x16, -6.0, seconds, SR).unwrap();
    let sub = vec![1e-40f32; n];
    assert!(sub[0].is_subnormal());
    let silence = vec![0.0f32; n];
    let inputs = [&noise[..n], &sub[..], &silence[..]];
    let mut modules: Vec<Box<dyn Module>> = (0..3)
        .map(|_| limiter_in(FS, &[], ProcessMode::Realtime, 256))
        .collect();
    let latency = modules[0].latency_samples() as usize;
    let mut outs = vec![vec![0.0f32; n]; 3];
    let mut times: Vec<Vec<f64>> = (0..3).map(|_| Vec::with_capacity(n / 256 + 1)).collect();
    let mut oe = OutputEvents::with_capacity(8);
    let mut pos = 0;
    while pos < n {
        let len = 256.min(n - pos);
        for k in 0..3 {
            let started = Instant::now();
            block(
                &mut *modules[k],
                &inputs[k][pos..pos + len],
                &mut outs[k][pos..pos + len],
                &[],
                pos,
                &mut oe,
            );
            times[k].push(started.elapsed().as_nanos() as f64);
        }
        pos += len;
    }
    assert!(outs[2].iter().all(|v| v.to_bits() == 0), "silence → zeros");
    assert!(outs[1][..latency].iter().all(|v| v.to_bits() == 0));
    assert!(
        outs[1][latency..]
            .iter()
            .all(|v| v.to_bits() == 1e-40f32.to_bits()),
        "subnormal DC passes bit-exactly"
    );
    let mut median = |k: usize| {
        times[k].sort_by(f64::total_cmp);
        times[k][times[k].len() / 2]
    };
    let (noise_ns, sub_ns, silence_ns) = (median(0), median(1), median(2));
    println!(
        "AC-16 {seconds} s, 256-frame blocks, FTZ/DAZ off: median block noise {noise_ns:.0} ns, \
         subnormal {sub_ns:.0} ns, silence {silence_ns:.0} ns"
    );
    (sub_ns / noise_ns, silence_ns / noise_ns)
}

#[test]
fn ac16_extreme_input_and_subnormals_without_ftz() {
    // ±4.0 squares (the fs/4 pattern at a = 4, true peak +15 dBFS) with +24 dB input gain.
    let squares = prep(&fs4_square(4.0, 24_000));
    check_run(
        SR,
        "fs/4 square a=4",
        &squares,
        Setting::new(CEILING_DBTP, 24.0),
    );
    // A naive (not band-limited) 1 kHz ±4 square: content above 20 kHz has no well-defined
    // true peak (SPEC-017 §2.4), so only finiteness and the sample peak are gated here.
    let naive: Vec<f32> = (0..24_000)
        .map(|i| if (i / 24) % 2 == 0 { 4.0 } else { -4.0 })
        .collect();
    let s = Setting::new(CEILING_DBTP, 24.0);
    let m = measure_run(SR, &prep(&naive), s);
    print_run(SR, "naive 1 kHz square a=4", s, &m);
    assert!(m.finite);
    assert!(m.sample_peak <= f32::from_bits((db_to_lin(CEILING_DBTP) as f32).to_bits() + 1));
    let (sub, silence) = ac16_timing(3.0);
    assert!(sub <= 1.5, "subnormal / noise = {sub}");
    assert!(silence <= 1.5, "silence / noise = {silence}");
}

#[test]
#[ignore = "60 s AC-16 timing run: `just test-big` (release)"]
fn ac16_full_timing() {
    let (sub, silence) = ac16_timing(60.0);
    println!("AC-16 60 s: subnormal / noise {sub:.3}, silence / noise {silence:.3}");
    assert!(sub <= 1.5, "subnormal / noise = {sub}");
    assert!(silence <= 1.5, "silence / noise = {silence}");
}

// ---------------------------------------------------------------------------------------------
// AC-3 / AC-4 full matrix (`just test-big`)
// ---------------------------------------------------------------------------------------------

/// SPEC-017 §5 stress set S at `rate` (all content ≤ 20 kHz, 5 ms fades, 50 ms padding).
fn stress_set(rate: u32) -> Vec<(String, Vec<f32>)> {
    let fs = f64::from(rate);
    let mut rng = Pcg32::new(0x5E17_0017, u64::from(rate));
    let mut set = vec![(
        "fs/4 square a=1".to_owned(),
        prep_at(&fs4_square(1.0, rate as usize / 2), rate),
    )];
    for f in [
        997.0,
        5_000.0,
        10_000.0,
        fs / 4.0,
        15_000.0,
        18_000.0,
        19_900.0,
    ] {
        let phase = 360.0 * rng.next_f64();
        let x = signal::sine_with_phase(f, 0.0, phase, 1.0, rate).unwrap();
        set.push((format!("sine {f} Hz"), prep_at(&x, rate)));
    }
    let sweep = signal::log_sweep(20.0, 20_000.0, 0.0, 5.0, rate).unwrap();
    set.push(("log sweep 20-20k 5 s".into(), prep_at(&sweep, rate)));
    let white = signal::white_noise(0x17, -6.0, 3.0, rate).unwrap();
    let white = rms_normalize(&band_limit_at(&white, rate), -6.0);
    set.push(("white -6 dBFS RMS".into(), prep_at(&white, rate)));
    let sparse = bandlimited_clicks(0x19, 40, 0.5, 1.0, 19_500.0, 2.0, rate).unwrap();
    set.push(("clicks sparse (40)".into(), prep_at(&sparse, rate)));
    let dense = bandlimited_clicks(0x1A, 2_000, 0.5, 1.0, 19_500.0, 2.0, rate).unwrap();
    set.push(("clicks dense (2000)".into(), prep_at(&dense, rate)));
    let bursts = signal::tone_bursts(0x1B, 997.0, 0.0, None, 0.05, 0.05, 2.0, rate).unwrap();
    set.push((
        "997 Hz bursts".into(),
        prep_at(&band_limit_at(&bursts, rate), rate),
    ));
    let voice = signal::voice_like(0x1C, 220.0, -6.0, 20.0, 0.25, 0.15, 3.0, rate).unwrap();
    let voice = peak_normalize(&band_limit_at(&voice, rate), -6.0);
    set.push(("voice_like -6 dBFS pk".into(), prep_at(&voice, rate)));
    let pink = signal::pink_noise(0x18, -14.0, 3.0, rate).unwrap();
    let pink = rms_normalize(&band_limit_at(&pink, rate), -14.0);
    set.push(("pink -14 dBFS RMS".into(), prep_at(&pink, rate)));
    set
}

/// Ceilings × input gains at the defaults, plus look-ahead 1/10 ms and release 10/1 000 ms at
/// ceiling −1 (every input gain).
fn matrix_settings() -> Vec<Setting> {
    let gains = [0.0, 6.0, 12.0, 24.0];
    let mut v = Vec::new();
    for ceiling in [-12.0, -3.0, -1.0, -0.1] {
        for gain in gains {
            v.push(Setting::new(ceiling, gain));
        }
    }
    for (release, lookahead) in [(100.0, 1.0), (100.0, 10.0), (10.0, 5.0), (1_000.0, 5.0)] {
        for gain in gains {
            v.push(Setting::with(CEILING_DBTP, gain, release, lookahead));
        }
    }
    v
}

#[test]
#[ignore = "full SPEC-017 AC-3/AC-4 matrix (1 440 renders): `just test-big` (release)"]
fn ac3_ac4_full_matrix() {
    let started = Instant::now();
    let rates = [44_100u32, 48_000, 96_000];
    let jobs: Vec<(u32, String, Vec<f32>)> = rates
        .iter()
        .flat_map(|&r| stress_set(r).into_iter().map(move |(n, x)| (r, n, x)))
        .collect();
    let settings = matrix_settings();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(u32, String, Setting, Measured)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some((rate, name, x)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    for &s in &settings {
                        let m = measure_run(*rate, x, s);
                        results.lock().unwrap().push((*rate, name.clone(), s, m));
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    // Job order; each job's settings were pushed in order by one thread (stable sort keeps it).
    results.sort_by(|a, b| {
        (a.0, jobs.iter().position(|j| j.0 == a.0 && j.1 == a.1))
            .cmp(&(b.0, jobs.iter().position(|j| j.0 == b.0 && j.1 == b.1)))
    });
    let mut failures = Vec::new();
    for (rate, name, s, m) in &results {
        print_run(*rate, name, *s, m);
        failures.extend(violations(*rate, name, *s, m));
    }
    for rate in rates {
        let (mut reference, mut four, mut sample, mut ebur) = (
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut runs = 0;
        for (_, _, s, m) in results.iter().filter(|r| r.0 == rate) {
            runs += 1;
            reference = reference.max(m.reference - s.ceiling);
            four = four.max(m.four - s.ceiling);
            sample = sample.max(db(f64::from(m.sample_peak)) - s.ceiling);
            if let Some(e) = m.ebur {
                ebur = ebur.max(e - s.ceiling);
            }
        }
        println!(
            "AC-3 summary {rate} Hz ({runs} runs): worst over the ceiling — reference \
             {reference:+.4} dB, plain 4x {four:+.4} dB, sample peak {sample:+.5} dB; ebur128 \
             {ebur:+.4} dB (AC-4, 44.1/48 kHz)"
        );
    }
    println!(
        "AC-3/AC-4 full matrix: {} runs in {:.1} s",
        results.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(results.len(), jobs.len() * settings.len());
    assert!(failures.is_empty(), "{failures:#?}");
}
