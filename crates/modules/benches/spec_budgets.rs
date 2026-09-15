//! T-704: the per-module CPU budgets the specs name, as `BENCH_RESULT` pass/fail lines for
//! `scripts/bench/summary.py` (`just bench`). Offline renders of 60 s at 48 kHz in 4096-frame
//! blocks (SPEC-000: offline renders use 4096-frame blocks), best of [`PASSES`], in exactly the
//! configuration each AC names:
//! - SPEC-013 AC-17: Noise Gate, pink noise, sidechain HPF on, look-ahead 5 ms — ≤ 0.3 s.
//! - SPEC-014 AC-18: Noise Reduction with a print, the AC-6 signal — ≤ 1.2 s at the defaults and
//!   ≤ 1.8 s at N = 8192; capturing a 60 s excerpt — ≤ 0.3 s. Plus PROMPT §2's "noise-reduction
//!   latency ≤ 50 ms": the default instance's reported latency at 48 and 44.1 kHz.
//! - SPEC-016 AC-19: Dynamics, pink noise — all sections on, RMS, look-ahead 10 ms ≤ 0.6 s; the
//!   defaults ≤ 0.3 s.
//! - SPEC-015 AC-22: the EQ response curve, 2048 points, total + 9 components, all bands on at
//!   48 dB/oct — ≤ 2 ms, median of 100.
//!
//! SPEC-017 AC-17 (True-Peak Limiter) is `true_peak_limiter.rs`; the assembled rack is
//! `vox-rack`'s `full_rack.rs`. Plain `main` (no divan): each AC is one wall-clock number.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleState, OutputEvents, ProcessContext, ProcessMode,
    Transport, noise_profile, response_curve,
};
use vox_modules::{Dynamics, NoiseGate, NoiseReduction, ParametricEq};
use vox_testkit::bench_report::{self, Target};
use vox_testkit::signal;

const RATE: u32 = 48_000;
const SECONDS: f64 = 60.0;
const BLOCK: usize = 4096;
const PASSES: usize = 3;
const CRATE: &str = "vox-modules";

type MakeModule = fn() -> Box<dyn Module>;

/// `m`'s complete state: every parameter at its default except `overrides`, plus `blob`.
fn state_for(m: &dyn Module, overrides: &[(&str, f64)], blob: Option<Vec<u8>>) -> ModuleState {
    let mut params: BTreeMap<String, f64> = m
        .params()
        .iter()
        .map(|p| (p.key.clone(), p.default))
        .collect();
    for &(key, value) in overrides {
        assert!(params.contains_key(key), "unknown parameter key {key}");
        params.insert(key.to_owned(), value);
    }
    ModuleState {
        format_version: m.descriptor().state_format_version,
        params,
        blob,
    }
}

fn offline_config() -> ActivateConfig {
    ActivateConfig {
        sample_rate: f64::from(RATE),
        max_block: BLOCK as u32,
        mode: ProcessMode::Offline,
        layout: ChannelLayout::MONO,
    }
}

/// A fresh, offline-activated instance of `make()` in the given configuration.
fn configured(make: MakeModule, overrides: &[(&str, f64)], blob: Option<&[u8]>) -> Box<dyn Module> {
    let mut m = make();
    let state = state_for(m.as_ref(), overrides, blob.map(<[u8]>::to_vec));
    m.load_state(&state).expect("load_state");
    m.activate(&offline_config()).expect("activate");
    m
}

/// Best-of-[`PASSES`] wall time (s) to render `input` through fresh instances from `build`.
fn render_seconds(build: &dyn Fn() -> Box<dyn Module>, input: &[f32]) -> f64 {
    let mut out = vec![0.0f32; BLOCK];
    let mut events = OutputEvents::with_capacity(8);
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut m = build();
        let started = Instant::now();
        for (i, chunk) in input.chunks(BLOCK).enumerate() {
            let pos = (i * BLOCK) as u64;
            events.clear();
            let mut ctx = ProcessContext::new(
                chunk.len() as u32,
                pos,
                Transport {
                    playing: true,
                    position_samples: Some(pos),
                },
                &[],
                &mut events,
            );
            m.process(&mut ctx, &[chunk], &mut [&mut out[..chunk.len()]]);
            std::hint::black_box(&out);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    best
}

fn report_render(name: &str, label: &str, seconds: f64, budget_s: f64) {
    println!(
        "  {label}: 60 s rendered in {seconds:.3} s = {:.2} % of one core (budget {budget_s} s)",
        100.0 * seconds / SECONDS
    );
    bench_report::result(CRATE, name, seconds, "s", Some(Target::le(budget_s)));
}

fn noise_gate() -> Box<dyn Module> {
    Box::new(NoiseGate::new())
}

fn noise_reduction() -> Box<dyn Module> {
    Box::new(NoiseReduction::new())
}

fn dynamics() -> Box<dyn Module> {
    Box::new(Dynamics::new())
}

/// SPEC-013 AC-17.
fn gate_budget(pink: &[f32]) {
    let overrides = [("sc_hpf_enabled", 1.0), ("lookahead_ms", 5.0)];
    let s = render_seconds(&|| configured(noise_gate, &overrides, None), pink);
    report_render(
        "noise_gate_60s_hpf_lookahead5ms_render_s",
        "SPEC-013 AC-17 Noise Gate, HPF on, look-ahead 5 ms",
        s,
        0.3,
    );
}

/// SPEC-014 AC-18, with a real print loaded (the STFT path; without one NR is a delay line).
fn noise_reduction_budget() {
    let voice =
        signal::voice_like(7, 220.0, -20.0, 20.0, 0.4, 0.6, SECONDS, RATE).expect("voice_like");
    let print_noise = signal::white_noise(8, -43.0, 3.0, RATE).expect("white noise");
    let profile = noise_profile(&NoiseReduction::new()).expect("NoiseProfile extension");
    let cancel = AtomicBool::new(false);
    let print = profile
        .capture(&print_noise, f64::from(RATE), &[], &cancel)
        .expect("capture");

    let mut probe = NoiseReduction::new();
    probe
        .load_state(&state_for(&probe, &[], Some(print.clone())))
        .expect("load_state");
    probe.activate(&offline_config()).expect("activate");
    assert!(probe.has_active_print(), "NR must run the STFT path");

    let s = render_seconds(&|| configured(noise_reduction, &[], Some(&print)), &voice);
    report_render(
        "noise_reduction_60s_defaults_render_s",
        "SPEC-014 AC-18 Noise Reduction with a print, defaults (N = 2048)",
        s,
        1.2,
    );
    let s = render_seconds(
        &|| configured(noise_reduction, &[("fft_size", 3.0)], Some(&print)),
        &voice,
    );
    report_render(
        "noise_reduction_60s_n8192_render_s",
        "SPEC-014 AC-18 Noise Reduction with a print, N = 8192",
        s,
        1.8,
    );

    for rate_hz in [48_000.0f64, 44_100.0] {
        let mut m = NoiseReduction::new();
        m.activate(&ActivateConfig {
            sample_rate: rate_hz,
            ..offline_config()
        })
        .expect("activate");
        let latency_ms = f64::from(m.latency_samples()) * 1e3 / rate_hz;
        println!(
            "  PROMPT §2 NR latency at {rate_hz} Hz, defaults: {latency_ms:.2} ms (budget 50 ms)"
        );
        bench_report::result(
            CRATE,
            &format!("noise_reduction_latency_{}hz_ms", rate_hz as u32),
            latency_ms,
            "ms",
            Some(Target::le(50.0)),
        );
    }

    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let started = Instant::now();
        let blob = profile
            .capture(&voice, f64::from(RATE), &[], &cancel)
            .expect("capture");
        std::hint::black_box(&blob);
        best = best.min(started.elapsed().as_secs_f64());
    }
    println!("  SPEC-014 AC-18 capture of a 60 s excerpt: {best:.3} s (budget 0.3 s)");
    bench_report::result(
        CRATE,
        "noise_reduction_capture_60s_s",
        best,
        "s",
        Some(Target::le(0.3)),
    );
}

/// SPEC-016 AC-19.
fn dynamics_budget(pink: &[f32]) {
    let all_on = [
        ("autogate_enabled", 1.0),
        ("expander_enabled", 1.0),
        ("compressor_enabled", 1.0),
        ("limiter_enabled", 1.0),
        ("detection", 1.0),
        ("lookahead_ms", 10.0),
    ];
    let s = render_seconds(&|| configured(dynamics, &all_on, None), pink);
    report_render(
        "dynamics_60s_all_sections_rms_lookahead10ms_render_s",
        "SPEC-016 AC-19 Dynamics, all sections on, RMS, look-ahead 10 ms",
        s,
        0.6,
    );
    let s = render_seconds(&|| configured(dynamics, &[], None), pink);
    report_render(
        "dynamics_60s_defaults_render_s",
        "SPEC-016 AC-19 Dynamics, defaults",
        s,
        0.3,
    );
}

/// SPEC-015 AC-22: 2 048 points (SPEC-015 §4.10's column frequencies over 20 Hz–20 kHz), the
/// total plus each of the 9 components, every band on and both filters at 48 dB/oct.
fn eq_curve_budget() {
    const COLUMNS: usize = 2048;
    const RUNS: usize = 100;
    let eq = ParametricEq::new();
    let curve = response_curve(&eq).expect("ResponseCurve extension");
    let mut owned: Vec<(String, f64)> = vec![
        ("hp_on".into(), 1.0),
        ("hp_slope".into(), 7.0),
        ("lp_on".into(), 1.0),
        ("lp_slope".into(), 7.0),
        ("ls_on".into(), 1.0),
        ("ls_gain_db".into(), 3.0),
        ("hs_on".into(), 1.0),
        ("hs_gain_db".into(), -3.0),
    ];
    for n in 1..=5 {
        owned.push((format!("b{n}_on"), 1.0));
        owned.push((format!("b{n}_gain_db"), if n % 2 == 0 { -4.0 } else { 4.0 }));
    }
    let overrides: Vec<(&str, f64)> = owned.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let state = state_for(&eq, &overrides, None);
    let values: Vec<f64> = eq.params().iter().map(|p| state.params[&p.key]).collect();
    let (f_lo, f_hi) = (20.0f64, 20_000.0f64);
    let freqs: Vec<f64> = (0..COLUMNS)
        .map(|i| f_lo * (f_hi / f_lo).powf((i as f64 + 0.5) / COLUMNS as f64))
        .collect();
    let components = curve.component_count(&values);
    assert_eq!(components, 9, "SPEC-015 AC-22: 9 components");
    let sr = f64::from(RATE);
    let mut total = vec![0.0f64; COLUMNS];
    let mut component = vec![0.0f64; COLUMNS];
    let mut times_ms = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let started = Instant::now();
        curve.magnitude_db(&values, sr, &freqs, &mut total);
        for c in 0..components {
            curve.component_magnitude_db(c, &values, sr, &freqs, &mut component);
            std::hint::black_box(&component);
        }
        std::hint::black_box(&total);
        times_ms.push(started.elapsed().as_secs_f64() * 1e3);
    }
    times_ms.sort_by(f64::total_cmp);
    let median = times_ms[RUNS / 2];
    println!(
        "  SPEC-015 AC-22 EQ curve, {COLUMNS} points, total + {components} components: median \
         {median:.3} ms (budget 2 ms)"
    );
    bench_report::result(
        CRATE,
        "eq_response_curve_2048pts_9components_median_ms",
        median,
        "ms",
        Some(Target::le(2.0)),
    );
}

fn main() {
    println!("SPEC module CPU budgets: offline, {RATE} Hz, {BLOCK}-frame blocks, best of {PASSES}");
    let pink = signal::pink_noise(0x7061, -20.0, SECONDS, RATE).expect("pink noise");
    gate_budget(&pink);
    noise_reduction_budget();
    dynamics_budget(&pink);
    eq_curve_budget();
}
