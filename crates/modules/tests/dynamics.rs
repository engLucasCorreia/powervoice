//! Dynamics (SPEC-016): schema and ModuleTestHost (AC-1), compressor static curves (AC-2/AC-3),
//! expander static curve (AC-4), AutoGate static behaviour and hysteresis (AC-5), limiter steady
//! state (AC-6), time constants (AC-8), AutoGate shapes and hold (AC-9), disabled sections inert /
//! all-off passthrough (AC-10/AC-11), enable crossfades (AC-12), look-ahead latency and restart
//! (AC-13), telemetry (AC-14), §4.3 zipper (AC-15), realtime = offline bit-identical (AC-16),
//! silence and denormals (AC-18).
//!
//! H-58 (Dynamics part 2) made the AutoGate, the Expander and the look-ahead live; the golden
//! signal checks of the two new sections run at 44.1 and 48 kHz.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time

mod common;

use std::sync::Arc;

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::test_util::{ModuleTestHost, TestRng, ZIPPER_LEN, zipper_signal};
use vox_module_api::{
    DEFAULT_EVENT_CAPACITY, HostRequest, Module, ModuleFactory, OutputEvents, ParamEvent,
    ParamFlags, ParamId, ProcessContext, ProcessMode, Tail, Taper, TelemetryKind, Transport,
    features, telemetry,
};
use vox_modules::{Dynamics, DynamicsFactory, builtin_factories};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;
const W_PK: f64 = 240.0;
/// Golden-signal rates (SPEC-016 §2.7).
const RATES: [f64; 2] = [44_100.0, 48_000.0];
const SINE_PEAK_TO_RMS_DB: f64 = 3.010_299_956_639_812;

fn make() -> Box<dyn Module> {
    Box::new(Dynamics::new())
}

fn dynamics(settings: &[(&str, f64)]) -> Box<dyn Module> {
    activated(make, settings, SR)
}

/// `max(1, round(ms · fs / 1000))` — the SPEC-016 §4.2 window/count conversion.
fn samples_of(ms: f64, fs: f64) -> usize {
    ((ms * fs / 1000.0).round() as usize).max(1)
}

/// SPEC-016 §4.4 expander curve, written independently of `vox_dsp`.
fn g_exp(x: f64, t: f64, r: f64, w: f64) -> f64 {
    let d = x - t;
    let g = if w > 0.0 && 2.0 * d.abs() <= w {
        -(r - 1.0) * (d - w / 2.0).powi(2) / (2.0 * w)
    } else if d < 0.0 {
        (r - 1.0) * d
    } else {
        0.0
    };
    g.max(-120.0)
}

/// SPEC-016 §4.4 compressor curve, written independently of `vox_dsp`.
fn g_comp(x: f64, t: f64, r: f64, w: f64) -> f64 {
    let d = x - t;
    let g = if w > 0.0 && 2.0 * d.abs() <= w {
        (1.0 / r - 1.0) * (d + w / 2.0).powi(2) / (2.0 * w)
    } else if d > 0.0 {
        (1.0 / r - 1.0) * d
    } else {
        0.0
    };
    g.max(-120.0)
}

fn param_info(key: &str) -> vox_module_api::ParamInfo {
    make()
        .params()
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("no parameter {key}"))
        .clone()
}

// ---------------------------------------------------------------------------------------------
// AC-1: schema, identity, registration, ModuleTestHost
// ---------------------------------------------------------------------------------------------

#[test]
fn schema_identity_and_registration() {
    let f = DynamicsFactory::new();
    let d = f.descriptor();
    assert_eq!(d.id, "org.powervoice.dynamics");
    assert_eq!(d.version.to_string(), "1.0.0");
    assert_eq!(d.state_format_version, 1);
    for feat in [
        features::AUDIO_EFFECT,
        features::COMPRESSOR,
        features::EXPANDER,
        features::GATE,
        features::LIMITER,
        features::MONO,
    ] {
        assert!(d.features.iter().any(|f| f == feat), "feature {feat}");
    }
    let m = f.create().unwrap();
    let got: Vec<(u32, &str, Option<u32>)> = m
        .params()
        .iter()
        .map(|p| (p.id.0, p.key.as_str(), p.group.map(|g| g.0)))
        .collect();
    let want: Vec<(u32, &str, Option<u32>)> = vec![
        (1, "detection", None),
        (2, "knee_db", None),
        (3, "lookahead_ms", None),
        (10, "autogate_enabled", Some(1)),
        (11, "autogate_threshold_db", Some(1)),
        (12, "autogate_attack_ms", Some(1)),
        (13, "autogate_hold_ms", Some(1)),
        (14, "autogate_release_ms", Some(1)),
        (20, "expander_enabled", Some(2)),
        (21, "expander_threshold_db", Some(2)),
        (22, "expander_ratio", Some(2)),
        (23, "expander_attack_ms", Some(2)),
        (24, "expander_release_ms", Some(2)),
        (30, "compressor_enabled", Some(3)),
        (31, "compressor_threshold_db", Some(3)),
        (32, "compressor_ratio", Some(3)),
        (33, "compressor_attack_ms", Some(3)),
        (34, "compressor_release_ms", Some(3)),
        (35, "compressor_makeup_db", Some(3)),
        (40, "limiter_enabled", Some(4)),
        (41, "limiter_threshold_db", Some(4)),
        (42, "limiter_attack_ms", Some(4)),
        (43, "limiter_release_ms", Some(4)),
    ];
    assert_eq!(got, want);
    let groups: Vec<(u32, &str, Option<u32>, bool)> = m
        .groups()
        .iter()
        .map(|g| {
            (
                g.id.0,
                g.key.as_str(),
                g.enable_param.map(|p| p.0),
                g.collapsed_by_default,
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            (1, "autogate", Some(10), true),
            (2, "expander", Some(20), true),
            (3, "compressor", Some(30), false),
            (4, "limiter", Some(40), true)
        ]
    );
    // Ranges, defaults and smoothing of the live sections (§3).
    for (key, min, max, default, smoothing) in [
        ("detection", 0.0, 1.0, 1.0, 20.0),
        ("knee_db", 0.0, 20.0, 6.0, 20.0),
        ("lookahead_ms", 0.0, 20.0, 0.0, 0.0),
        ("autogate_enabled", 0.0, 1.0, 0.0, 20.0),
        ("autogate_threshold_db", -80.0, 0.0, -50.0, 0.0),
        ("autogate_attack_ms", 0.1, 100.0, 2.0, 0.0),
        ("autogate_hold_ms", 0.1, 1000.0, 50.0, 0.0),
        ("autogate_release_ms", 1.0, 2000.0, 100.0, 0.0),
        ("expander_enabled", 0.0, 1.0, 0.0, 20.0),
        ("expander_threshold_db", -80.0, 0.0, -45.0, 20.0),
        ("expander_ratio", 1.0, 30.0, 2.0, 20.0),
        ("expander_attack_ms", 0.1, 100.0, 2.0, 0.0),
        ("expander_release_ms", 1.0, 2000.0, 100.0, 0.0),
        ("compressor_enabled", 0.0, 1.0, 1.0, 20.0),
        ("compressor_threshold_db", -60.0, 0.0, -20.0, 20.0),
        ("compressor_ratio", 1.0, 30.0, 3.0, 20.0),
        ("compressor_attack_ms", 0.1, 200.0, 10.0, 0.0),
        ("compressor_release_ms", 1.0, 2000.0, 100.0, 0.0),
        ("compressor_makeup_db", 0.0, 30.0, 0.0, 20.0),
        ("limiter_enabled", 0.0, 1.0, 0.0, 20.0),
        ("limiter_threshold_db", -30.0, 0.0, -1.0, 20.0),
        ("limiter_attack_ms", 0.1, 50.0, 1.0, 0.0),
        ("limiter_release_ms", 1.0, 2000.0, 100.0, 0.0),
    ] {
        let p = param_info(key);
        assert_eq!(
            (p.min, p.max, p.default, p.smoothing_ms),
            (min, max, default, smoothing),
            "{key}"
        );
        assert!(p.flags.contains(ParamFlags::AUTOMATABLE), "{key}");
    }
    for p in m.params() {
        if let Taper::Db { neg_inf_at_min } = p.taper {
            assert!(!neg_inf_at_min, "{}", p.key);
        }
        // H-58: every parameter is live, so none is hidden any more.
        assert!(
            !p.flags.contains(ParamFlags::HIDDEN),
            "{} must not be hidden",
            p.key
        );
    }
    assert_eq!(param_info("detection").value_to_text(1.0), "RMS");
    assert_eq!(param_info("detection").value_to_text(0.0), "Peak");
    assert_eq!(param_info("compressor_ratio").value_to_text(3.0), "3.0:1");
    assert_eq!(
        param_info("compressor_threshold_db").value_to_text(-20.0),
        "-20.0 dBFS"
    );
    // Telemetry channels (§3).
    let t = telemetry(&*m).expect("Telemetry extension");
    let ch: Vec<(&str, TelemetryKind, Option<u32>)> = t
        .channels()
        .iter()
        .map(|c| (c.key.as_str(), c.kind, c.group.map(|g| g.0)))
        .collect();
    assert_eq!(
        ch,
        [
            ("gr_total_db", TelemetryKind::GainReduction, None),
            ("gr_autogate_db", TelemetryKind::GainReduction, Some(1)),
            ("gr_expander_db", TelemetryKind::GainReduction, Some(2)),
            ("gr_compressor_db", TelemetryKind::GainReduction, Some(3)),
            ("gr_limiter_db", TelemetryKind::GainReduction, Some(4)),
            ("input_level_dbfs", TelemetryKind::Level, None),
            ("autogate_open", TelemetryKind::Indicator, Some(1)),
        ]
    );
    assert!(
        builtin_factories()
            .iter()
            .any(|f| f.descriptor().id == Dynamics::ID)
    );
    ModuleTestHost::from_factory(Arc::new(DynamicsFactory::new()))
        .check_schema()
        .unwrap();
}

#[test]
fn state_round_trips_bit_identically_with_a_non_zero_lookahead() {
    let settings = [
        ("lookahead_ms", 7.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -37.5),
        ("autogate_hold_ms", 123.4),
        ("expander_enabled", 1.0),
        ("expander_ratio", 6.5),
        ("compressor_makeup_db", 3.5),
    ];
    let m = dynamics(&settings);
    assert_eq!(m.latency_samples(), 336, "7 ms at 48 kHz");
    let state = m.save_state().unwrap();
    let mut other = make();
    other.load_state(&state).unwrap();
    let again = other.save_state().unwrap();
    assert_eq!(state.format_version, again.format_version);
    for (k, v) in &state.params {
        assert_eq!(
            v.to_bits(),
            again.params[k].to_bits(),
            "{k} did not round-trip"
        );
    }
    for (key, value) in settings {
        assert_eq!(state.params[key].to_bits(), value.to_bits(), "{key}");
    }
}

#[test]
fn passes_module_test_host_with_delayed_opt_outs() {
    let mut host = ModuleTestHost::new(make);
    for p in make().params() {
        if p.id != Dynamics::COMPRESSOR_MAKEUP_DB {
            host = host.allow_delayed_effect(p.id);
        }
    }
    let report = host.assert_passes();
    assert!(report.alloc_checked);
    assert!(report.zero_length_blocks > 0);
    let makeup: Vec<_> = report
        .first_effect
        .iter()
        .filter(|p| p.key == "compressor_makeup_db")
        .collect();
    assert!(!makeup.is_empty());
    assert!(
        makeup.iter().all(|p| p.first_difference == Some(p.offset)),
        "makeup must act exactly at the event sample: {makeup:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-2 / AC-3: compressor static curve
// ---------------------------------------------------------------------------------------------

/// Settled gain (output peak / input peak over the second half of 0.5 s) of a steady sine.
fn steady_gain_db(settings: &[(&str, f64)], freq: f64, level_dbfs: f64) -> f64 {
    let n = 24_000;
    let x = sine(freq, level_dbfs, n, SR);
    let y = render(&mut *dynamics(settings), &x, &[], Blocks::Fixed(4096));
    to_db(peak(&y[n / 2..]) / peak(&x[n / 2..]))
}

/// The AC-2 grid; returns the worst |error| in dB.
fn static_curve_worst(detection: f64, level_offset_db: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for t in [-40.0, -20.0, -6.0] {
        for r in [1.5, 4.0, 30.0] {
            for w in [0.0, 6.0] {
                let settings = [
                    ("detection", detection),
                    ("knee_db", w),
                    ("compressor_threshold_db", t),
                    ("compressor_ratio", r),
                ];
                for level in -60..=0 {
                    let level = f64::from(level);
                    let got = steady_gain_db(&settings, 997.0, level);
                    let want = g_comp(level - level_offset_db, t, r, w);
                    let err = (got - want).abs();
                    assert!(
                        err <= 0.1,
                        "T {t} R {r} W {w} in {level} dBFS: gain {got:.4} dB, expected {want:.4}"
                    );
                    worst = worst.max(err);
                }
            }
        }
    }
    worst
}

#[test]
fn compressor_static_curve_peak() {
    let worst = static_curve_worst(0.0, 0.0);
    println!("AC-2 Peak: worst static-curve error {worst:.4} dB (1098 points)");
}

#[test]
fn compressor_static_curve_rms_and_makeup() {
    let worst = static_curve_worst(1.0, SINE_PEAK_TO_RMS_DB);
    println!("AC-3 RMS: worst static-curve error {worst:.4} dB (1098 points)");
    // 120 Hz: worst-case RMS window ripple, ±0.5 dB.
    let mut worst_120: f64 = 0.0;
    let s = [
        ("knee_db", 0.0),
        ("compressor_threshold_db", -20.0),
        ("compressor_ratio", 4.0),
    ];
    for level in [-40.0, -30.0, -20.0, -10.0, 0.0] {
        let got = steady_gain_db(&s, 120.0, level);
        let err = (got - g_comp(level - SINE_PEAK_TO_RMS_DB, -20.0, 4.0, 0.0)).abs();
        assert!(err <= 0.5, "120 Hz in {level}: error {err:.3} dB");
        worst_120 = worst_120.max(err);
    }
    println!("AC-3 RMS 120 Hz: worst error {worst_120:.4} dB");
    // Makeup 6 dB lifts every output by 6.00 ± 0.01 dB.
    let base = [
        ("knee_db", 6.0),
        ("compressor_threshold_db", -20.0),
        ("compressor_ratio", 4.0),
    ];
    for level in [-40.0, -20.0, 0.0] {
        let a = steady_gain_db(&base, 997.0, level);
        let mut with = base.to_vec();
        with.push(("compressor_makeup_db", 6.0));
        let b = steady_gain_db(&with, 997.0, level);
        assert!(
            (b - a - 6.0).abs() <= 0.01,
            "makeup at {level}: {:.4}",
            b - a
        );
    }
}

// ---------------------------------------------------------------------------------------------
// AC-4: expander static curve (Peak and RMS)
// ---------------------------------------------------------------------------------------------

/// Settled gain of a steady 997 Hz sine through the expander alone, measured over the last
/// 250 ms of 500 ms. AC-4 fixes no time constants; the test uses **equal** 20 ms attack and
/// release: the deepest reduction it checks is 120 dB, which the default 100 ms release would
/// still be approaching after 500 ms, and equal constants keep the branching smoother from
/// rectifying the RMS window's ±0.013 dB ripple into a bias (which `R − 1 = 29` would magnify).
fn expander_gain_db_measured(settings: &[(&str, f64)], level_dbfs: f64, fs: f64) -> f64 {
    let n = (0.5 * fs) as usize;
    let x = sine(997.0, level_dbfs, n, fs);
    let y = render(
        &mut *activated(make, settings, fs),
        &x,
        &[],
        Blocks::Fixed(4096),
    );
    to_db(peak(&y[n / 2..]) / peak(&x[n / 2..]))
}

#[test]
fn expander_static_curve() {
    let mut worst: f64 = 0.0;
    let mut points = 0usize;
    for (detection, offset) in [(0.0, 0.0), (1.0, SINE_PEAK_TO_RMS_DB)] {
        for t in [-50.0, -30.0] {
            for r in [2.0, 4.0, 30.0] {
                for w in [0.0, 6.0] {
                    let settings = [
                        ("detection", detection),
                        ("knee_db", w),
                        ("compressor_enabled", 0.0),
                        ("expander_enabled", 1.0),
                        ("expander_threshold_db", t),
                        ("expander_ratio", r),
                        ("expander_attack_ms", 20.0),
                        ("expander_release_ms", 20.0),
                    ];
                    for level in -90..=0 {
                        let level = f64::from(level);
                        let want = g_exp(level - offset, t, r, w);
                        if level + want < -120.0 {
                            continue; // the AC only claims outputs at or above −120 dBFS
                        }
                        let got = expander_gain_db_measured(&settings, level, SR);
                        let err = (got - want).abs();
                        assert!(
                            err <= 0.1,
                            "detection {detection} T {t} R {r} W {w} in {level} dBFS: \
                             gain {got:.4} dB, expected {want:.4}"
                        );
                        // Above the knee the section must be transparent.
                        if level - offset >= t + w / 2.0 {
                            assert!(got.abs() <= 0.01, "unity above T + W/2: {got:.4} dB");
                        }
                        worst = worst.max(err);
                        points += 1;
                    }
                }
            }
        }
    }
    println!("AC-4 expander: worst static-curve error {worst:.4} dB ({points} points)");
}

#[test]
fn expander_static_curve_at_44_1_khz() {
    // Golden signal at the second supported rate: one representative setting per branch.
    let settings = [
        ("compressor_enabled", 0.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -40.0),
        ("expander_ratio", 4.0),
        ("expander_attack_ms", 20.0),
        ("expander_release_ms", 20.0),
        ("knee_db", 6.0),
        ("detection", 0.0),
    ];
    for fs in RATES {
        for level in [-70.0, -55.0, -43.0, -40.0, -37.0, -20.0] {
            let got = expander_gain_db_measured(&settings, level, fs);
            let want = g_exp(level, -40.0, 4.0, 6.0);
            assert!(
                (got - want).abs() <= 0.1,
                "{fs} Hz, {level} dBFS: {got:.4} vs {want:.4}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-5: AutoGate static behaviour and hysteresis
// ---------------------------------------------------------------------------------------------

/// The AutoGate alone (every other section off).
fn autogate(threshold_db: f64, extra: &[(&str, f64)], fs: f64) -> Box<dyn Module> {
    let mut s = vec![
        ("compressor_enabled", 0.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", threshold_db),
    ];
    s.extend_from_slice(extra);
    activated(make, &s, fs)
}

#[test]
fn autogate_opens_holds_and_closes_with_hysteresis() {
    for fs in RATES {
        for t in [-40.0, -60.0, -20.0] {
            let burst = (0.3 * fs) as usize;
            let attack = samples_of(2.0, fs);
            let w_pk = samples_of(5.0, fs);

            // Bursts 0.5 dB above the threshold open it; after the attack the output is the input.
            let x = sine_segments(997.0, &[(t + 0.5, burst), (f64::NEG_INFINITY, burst)], fs);
            let mut m = autogate(t, &[], fs);
            let tlm = telemetry(&*m).unwrap();
            let y = render(&mut *m, &x, &[], Blocks::Fixed(1024));
            // The sliding peak needs a quarter period to see the sine's amplitude; 5 ms covers it.
            let open_from = attack + w_pk;
            assert_eq!(
                bit_equal(&y[open_from..burst], &x[open_from..burst]),
                None,
                "{fs} Hz T {t}: an open gate must pass the input bit-exactly"
            );
            assert_eq!(
                read_all(&*tlm)[Dynamics::TELEMETRY_AUTOGATE_OPEN],
                1.0,
                "{fs} Hz T {t}: lamp"
            );

            // Bursts 0.5 dB below never open it: exactly 0.0 from reset, lamp off.
            let x = sine(997.0, t - 0.5, burst, fs);
            let mut m = autogate(t, &[], fs);
            let tlm = telemetry(&*m).unwrap();
            let y = render(&mut *m, &x, &[], Blocks::Fixed(1024));
            assert!(
                y.iter().all(|&v| v == 0.0),
                "{fs} Hz T {t}: a closed gate must output exactly 0.0"
            );
            assert_eq!(
                read_all(&*tlm)[Dynamics::TELEMETRY_AUTOGATE_OPEN],
                0.0,
                "{fs} Hz T {t}: lamp off"
            );

            // Opened at T + 10, a tone at T − 2.5 (above T − 3) keeps it open for 10 s.
            let hold = (10.0 * fs) as usize;
            let x = sine_segments(997.0, &[(t + 10.0, burst), (t - 2.5, hold)], fs);
            let y = render(&mut *autogate(t, &[], fs), &x, &[], Blocks::Fixed(4096));
            assert_eq!(
                bit_equal(&y[burst + attack..], &x[burst + attack..]),
                None,
                "{fs} Hz T {t}: hysteresis must hold the gate open"
            );

            // A tone at T − 3.5 closes it: exactly 0.0 within W_pk + hold + 14 τ_R + 1.
            let tail = (2.0 * fs) as usize;
            let x = sine_segments(997.0, &[(t + 10.0, burst), (t - 3.5, tail)], fs);
            let y = render(&mut *autogate(t, &[], fs), &x, &[], Blocks::Fixed(4096));
            let silent_from = y.iter().rposition(|&v| v != 0.0).expect("signal") + 1;
            let limit = burst + w_pk + samples_of(50.0, fs) + (14.0 * 0.1 * fs) as usize + 1;
            assert!(
                silent_from <= limit,
                "{fs} Hz T {t}: silent at {silent_from}, limit {limit}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-6: limiter steady state
// ---------------------------------------------------------------------------------------------

#[test]
fn limiter_steady_state() {
    let mut worst: f64 = 0.0;
    for detection in [0.0, 1.0] {
        for t in [-12.0, -6.0, -1.0] {
            let s = [
                ("detection", detection),
                ("compressor_enabled", 0.0),
                ("limiter_enabled", 1.0),
                ("limiter_threshold_db", t),
            ];
            for over in [1.0, 6.0, 20.0] {
                let n = 24_000;
                let x = sine(997.0, t + over, n, SR);
                let y = render(&mut *dynamics(&s), &x, &[], Blocks::Fixed(4096));
                let out = to_db(peak(&y[n / 2..]));
                assert!(
                    (out - t).abs() <= 0.05,
                    "T {t}, input T+{over}: output peak {out:.4} dBFS"
                );
                worst = worst.max((out - t).abs());
            }
            let g = steady_gain_db(&s, 997.0, t - 1.0);
            assert!(g.abs() <= 0.01, "T {t}: input T−1 changed by {g:.4} dB");
        }
    }
    println!("AC-6 limiter: worst ceiling error {worst:.4} dB");
}

// ---------------------------------------------------------------------------------------------
// AC-8: time constants (DC steps, Peak, knee 0)
// ---------------------------------------------------------------------------------------------

fn gain_trace(settings: &[(&str, f64)], x: &[f32], changes: &[Change]) -> Vec<f64> {
    let y = render(&mut *dynamics(settings), x, changes, Blocks::Fixed(4096));
    x.iter()
        .zip(&y)
        .map(|(a, b)| to_db(f64::from(*b) / f64::from(*a)))
        .collect()
}

/// Samples processed from `step` (inclusive) until the gain first covers 63.2 % of the move
/// `from_db` → `to_db` (the convention of the dsp unit tests: after n one-pole updates the
/// covered fraction is `1 − αⁿ`).
fn tau_samples(trace: &[f64], step: usize, from_db: f64, to_db: f64) -> usize {
    let goal = from_db + (to_db - from_db) * TAU_FRACTION;
    let dir = (to_db - from_db).signum();
    (step..trace.len())
        .find(|&i| (trace[i] - goal) * dir >= 0.0)
        .expect("63.2 % never reached")
        - step
        + 1
}

/// Asserts ±2 % or ±1 sample (spec; the ticket allows ±10 %); returns the relative error.
fn check_tau(label: &str, measured: usize, expected: f64) -> f64 {
    let tol = (0.02 * expected).max(1.0);
    let err = measured as f64 - expected;
    assert!(
        err.abs() <= tol,
        "{label}: {measured} samples, expected {expected:.1} ± {tol:.1}"
    );
    err / expected
}

#[test]
fn compressor_and_limiter_time_constants() {
    let mut report = Vec::new();
    let comp = [
        ("detection", 0.0),
        ("knee_db", 0.0),
        ("compressor_threshold_db", -20.0),
        ("compressor_ratio", 4.0),
    ];
    for tau in [0.5, 10.0, 100.0] {
        let mut s = comp.to_vec();
        s.push(("compressor_attack_ms", tau));
        let x = dc_segments(&[(-40.0, 4800), (-10.0, 24_000)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 4800, 0.0, -7.5);
        let e = check_tau(&format!("compressor attack {tau} ms"), n, tau * SR / 1000.0);
        report.push(format!(
            "comp attack {tau} ms: {n} smp ({:+.2} %)",
            e * 100.0
        ));
    }
    for tau in [10.0, 100.0, 1000.0] {
        let mut s = comp.to_vec();
        s.push(("compressor_release_ms", tau));
        let x = dc_segments(&[(-10.0, 14_400), (-40.0, 72_000)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 14_400, -7.5, 0.0);
        let want = W_PK + tau * SR / 1000.0;
        let e = check_tau(&format!("compressor release {tau} ms"), n, want);
        report.push(format!(
            "comp release {tau} ms: {n} smp vs W_pk+τ {want} ({:+.2} %)",
            e * 100.0
        ));
    }
    let lim = [
        ("detection", 0.0),
        ("compressor_enabled", 0.0),
        ("limiter_enabled", 1.0),
        ("limiter_threshold_db", -10.0),
    ];
    for tau in [0.1, 1.0, 10.0] {
        let mut s = lim.to_vec();
        s.push(("limiter_attack_ms", tau));
        let x = dc_segments(&[(-20.0, 4800), (-4.0, 4800)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 4800, 0.0, -6.0);
        let e = check_tau(&format!("limiter attack {tau} ms"), n, tau * SR / 1000.0);
        report.push(format!(
            "lim attack {tau} ms: {n} smp ({:+.2} %)",
            e * 100.0
        ));
    }
    for tau in [10.0, 100.0, 1000.0] {
        let mut s = lim.to_vec();
        s.push(("limiter_release_ms", tau));
        let x = dc_segments(&[(-4.0, 14_400), (-20.0, 72_000)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 14_400, -6.0, 0.0);
        let want = W_PK + tau * SR / 1000.0;
        let e = check_tau(&format!("limiter release {tau} ms"), n, want);
        report.push(format!(
            "lim release {tau} ms: {n} smp vs W_pk+τ {want} ({:+.2} %)",
            e * 100.0
        ));
    }
    println!("AC-8 time constants:\n  {}", report.join("\n  "));
}

#[test]
fn expander_time_constants() {
    let mut report = Vec::new();
    let base = [
        ("detection", 0.0),
        ("knee_db", 0.0),
        ("compressor_enabled", 0.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -40.0),
        ("expander_ratio", 2.0),
    ];
    // Attack = the level rising (−60 → −30): the gain climbs from −20 dB to 0. The first second
    // settles the gain on −20 dB with the 100 ms release.
    for tau in [0.5, 10.0, 100.0] {
        let mut s = base.to_vec();
        s.push(("expander_attack_ms", tau));
        s.push(("expander_release_ms", 100.0));
        let x = dc_segments(&[(-60.0, 48_000), (-30.0, 48_000)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 48_000, -20.0, 0.0);
        let e = check_tau(&format!("expander attack {tau} ms"), n, tau * SR / 1000.0);
        report.push(format!(
            "exp attack {tau} ms: {n} smp ({:+.2} %)",
            e * 100.0
        ));
    }
    // Release = the level falling (−30 → −60): the peak window holds for W_pk first.
    for tau in [10.0, 100.0, 1000.0] {
        let mut s = base.to_vec();
        s.push(("expander_attack_ms", 0.1));
        s.push(("expander_release_ms", tau));
        let x = dc_segments(&[(-30.0, 14_400), (-60.0, 120_000)]);
        let n = tau_samples(&gain_trace(&s, &x, &[]), 14_400, 0.0, -20.0);
        let want = W_PK + tau * SR / 1000.0;
        let e = check_tau(&format!("expander release {tau} ms"), n, want);
        report.push(format!(
            "exp release {tau} ms: {n} smp vs W_pk+τ {want} ({:+.2} %)",
            e * 100.0
        ));
    }
    println!("AC-8 expander time constants:\n  {}", report.join("\n  "));
}

// ---------------------------------------------------------------------------------------------
// AC-9: AutoGate opening shape, hold and release
// ---------------------------------------------------------------------------------------------

/// Linear gain per sample of the AutoGate alone on a DC signal (`y / x`).
fn gate_gain_trace(settings: &[(&str, f64)], x: &[f32], fs: f64) -> Vec<f64> {
    let mut s = vec![("compressor_enabled", 0.0), ("autogate_enabled", 1.0)];
    s.extend_from_slice(settings);
    let y = render(&mut *activated(make, &s, fs), x, &[], Blocks::Fixed(4096));
    x.iter()
        .zip(&y)
        .map(|(a, b)| f64::from(*b) / f64::from(*a))
        .collect()
}

#[test]
fn autogate_opening_is_a_raised_cosine_of_exact_length() {
    for fs in RATES {
        for attack_ms in [0.1, 2.0, 20.0, 100.0] {
            let a = samples_of(attack_ms, fs);
            let n = a + (0.2 * fs) as usize;
            let x = dc_segments(&[(-20.0, n)]);
            let g = gate_gain_trace(
                &[
                    ("autogate_threshold_db", -40.0),
                    ("autogate_attack_ms", attack_ms),
                ],
                &x,
                fs,
            );
            // DC: the sliding peak reads −20 dBFS at the very first sample, so p = 1 there.
            for p in 1..=a {
                let want = (1.0 - (std::f64::consts::PI * p as f64 / a as f64).cos()) / 2.0;
                assert!(
                    (g[p - 1] - want).abs() <= 1e-6,
                    "{fs} Hz attack {attack_ms} ms, p {p}: {} vs {want}",
                    g[p - 1]
                );
            }
            assert_eq!(
                g[a - 1],
                1.0,
                "{fs} Hz attack {attack_ms} ms: exactly 1 at A"
            );
        }
    }
}

#[test]
fn autogate_release_starts_after_the_peak_window_and_the_hold() {
    let mut report = Vec::new();
    for fs in RATES {
        let w_pk = samples_of(5.0, fs);
        for hold_ms in [0.1, 10.0, 50.0, 500.0] {
            let h = (hold_ms * fs / 1000.0).round() as usize;
            let burst = (0.5 * fs) as usize;
            let tail = h + w_pk + (1.0 * fs) as usize;
            let s = [
                ("autogate_threshold_db", -40.0),
                ("autogate_attack_ms", 1.0),
                ("autogate_hold_ms", hold_ms),
                ("autogate_release_ms", 100.0),
            ];
            // A DC burst, then DC below the close threshold (silence would carry no gain to
            // read): e = the last burst sample.
            let x = dc_segments(&[(-20.0, burst), (-50.0, tail)]);
            let g = gate_gain_trace(&s, &x, fs);
            let e = burst - 1;
            let start = (burst..g.len())
                .find(|&i| g[i] < 1.0)
                .expect("the gate releases");
            let want = e + w_pk + h;
            assert!(
                start.abs_diff(want) <= 1,
                "{fs} Hz hold {hold_ms} ms: release at {start}, expected {want}"
            );
            // One-pole release with 63.2 % at τ_R.
            let tau = (0..g.len() - start)
                .find(|&i| g[start + i] <= 1.0 - TAU_FRACTION)
                .expect("63.2 %")
                + 1;
            let expected = 0.1 * fs;
            assert!(
                (tau as f64 - expected).abs() <= (0.02 * expected).max(1.0),
                "{fs} Hz hold {hold_ms} ms: τ_R {tau} samples, expected {expected}"
            );
            report.push(format!(
                "{fs} Hz hold {hold_ms} ms: release at e+{}, τ_R {tau}",
                start - e
            ));
        }
        // With a 997 Hz burst instead of DC the release start is within ±1 ms.
        let burst = (0.5 * fs) as usize;
        let tail = (0.5 * fs) as usize;
        let x = sine_segments(997.0, &[(-20.0, burst), (-50.0, tail)], fs);
        let mut s = vec![
            ("compressor_enabled", 0.0),
            ("autogate_enabled", 1.0),
            ("autogate_threshold_db", -40.0),
            ("autogate_hold_ms", 10.0),
            ("autogate_release_ms", 100.0),
        ];
        s.push(("autogate_attack_ms", 1.0));
        let y = render(&mut *activated(make, &s, fs), &x, &[], Blocks::Fixed(4096));
        let start = (burst..y.len())
            .find(|&i| x[i] != 0.0 && (f64::from(y[i]) / f64::from(x[i])) < 1.0 - 1e-9)
            .expect("the gate releases");
        let want = burst - 1 + w_pk + samples_of(10.0, fs);
        assert!(
            start.abs_diff(want) as f64 <= fs / 1000.0,
            "{fs} Hz tone: release at {start}, expected {want}"
        );
    }
    println!("AC-9 AutoGate hold:\n  {}", report.join("\n  "));
}

// ---------------------------------------------------------------------------------------------
// AC-10 / AC-11: disabled sections are inert; all off = passthrough
// ---------------------------------------------------------------------------------------------

const COMP_KEYS: [&str; 5] = [
    "compressor_threshold_db",
    "compressor_ratio",
    "compressor_attack_ms",
    "compressor_release_ms",
    "compressor_makeup_db",
];
const LIM_KEYS: [&str; 3] = [
    "limiter_threshold_db",
    "limiter_attack_ms",
    "limiter_release_ms",
];
const AG_KEYS: [&str; 4] = [
    "autogate_threshold_db",
    "autogate_attack_ms",
    "autogate_hold_ms",
    "autogate_release_ms",
];
const EX_KEYS: [&str; 4] = [
    "expander_threshold_db",
    "expander_ratio",
    "expander_attack_ms",
    "expander_release_ms",
];
const GLOBAL_KEYS: [&str; 3] = ["detection", "knee_db", "lookahead_ms"];

fn random_values(rng: &mut TestRng, keys: &[&'static str]) -> Vec<(&'static str, f64)> {
    keys.iter()
        .map(|&k| (k, param_info(k).from_normalized(rng.unit_f64())))
        .collect()
}

/// 200 ms noise bursts at −6 dBFS peak every 700 ms over a −50 dBFS noise floor.
fn burst_signal(seed: u64, n: usize) -> Vec<f32> {
    let loud = white(seed, 0.5, n);
    let floor = white(seed ^ 0xF100, lin(-50.0) as f32, n);
    (0..n)
        .map(|i| {
            if i % 33_600 < 9_600 {
                loud[i]
            } else {
                floor[i]
            }
        })
        .collect()
}

#[test]
fn disabled_sections_are_inert() {
    let x = burst_signal(7, 33_600);
    let sections: [(&str, &[&'static str]); 4] = [
        ("autogate_enabled", &AG_KEYS),
        ("expander_enabled", &EX_KEYS),
        ("compressor_enabled", &COMP_KEYS),
        ("limiter_enabled", &LIM_KEYS),
    ];
    for (si, &(enable, keys)) in sections.iter().enumerate() {
        let mut rng = TestRng::new(0xAC10 + si as u64);
        // The other sections at seeded random settings, all of them enabled.
        let mut others = random_values(&mut rng, &GLOBAL_KEYS);
        for (other_enable, other_keys) in &sections {
            if *other_enable != enable {
                others.extend(random_values(&mut rng, other_keys));
                others.push((other_enable, 1.0));
            }
        }
        let mut reference = others.clone();
        reference.push((enable, 0.0));
        let want = render(
            &mut *dynamics(&reference),
            &x,
            &[],
            Blocks::Random { seed: 1, max: 1024 },
        );
        for set in 0..100u64 {
            let mut s = others.clone();
            s.extend(random_values(&mut rng, keys));
            s.push((enable, 0.0));
            let blocks = if set % 10 == 0 {
                Blocks::Fixed(4096)
            } else {
                Blocks::Random {
                    seed: set,
                    max: 1024,
                }
            };
            let y = render(&mut *dynamics(&s), &x, &[], blocks);
            assert_eq!(
                bit_equal(&y, &want),
                None,
                "{enable} off (set {set}): output differs; settings {s:?}"
            );
        }
    }
}

/// `x` delayed by `la` samples (zeros before it).
fn delayed(x: &[f32], la: usize) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    y[la..].copy_from_slice(&x[..x.len() - la]);
    y
}

#[test]
fn all_sections_off_is_a_bit_exact_passthrough() {
    let signals = [white(0xAC11, 1.0, 48_000), burst_signal(3, 33_600)];
    let mut rng = TestRng::new(0xAC11);
    for set in 0..20u64 {
        for lookahead_ms in [0.0, 5.0, 20.0] {
            let mut s = Vec::new();
            for keys in [&GLOBAL_KEYS[..], &COMP_KEYS, &LIM_KEYS, &AG_KEYS, &EX_KEYS] {
                s.extend(random_values(&mut rng, keys));
            }
            s.retain(|(k, _)| *k != "lookahead_ms");
            s.push(("lookahead_ms", lookahead_ms));
            for enable in [
                "compressor_enabled",
                "limiter_enabled",
                "autogate_enabled",
                "expander_enabled",
            ] {
                s.push((enable, 0.0));
            }
            let la = (lookahead_ms * SR / 1000.0).round() as usize;
            let mut m = dynamics(&s);
            assert_eq!(m.latency_samples() as usize, la, "{lookahead_ms} ms");
            for x in &signals {
                let y = render(
                    &mut *m,
                    x,
                    &[],
                    Blocks::Random {
                        seed: set,
                        max: 1024,
                    },
                );
                assert_eq!(
                    bit_equal(&y, &delayed(x, la)),
                    None,
                    "set {set}, look-ahead {lookahead_ms} ms: {s:?}"
                );
                m.reset();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-12: enable toggles crossfade linearly over 20 ms
// ---------------------------------------------------------------------------------------------

#[test]
fn enable_toggles_crossfade_linearly() {
    let x = dc_segments(&[(-10.0, 72_000)]);
    let (on_at, off_at, ramp) = (24_000usize, 48_000usize, 960usize);
    type Case = (ParamId, Vec<(&'static str, f64)>, f64);
    let cases: [Case; 2] = [
        (
            Dynamics::COMPRESSOR_ENABLED,
            vec![
                ("detection", 0.0),
                ("knee_db", 0.0),
                ("compressor_threshold_db", -40.0),
                ("compressor_ratio", 4.0),
                ("compressor_enabled", 0.0),
            ],
            -22.5,
        ),
        (
            Dynamics::LIMITER_ENABLED,
            vec![("compressor_enabled", 0.0), ("limiter_threshold_db", -30.0)],
            -20.0,
        ),
    ];
    for (id, settings, full_db) in cases {
        let changes = [(on_at, id, 1.0), (off_at, id, 0.0)];
        let tr = gain_trace(&settings, &x, &changes);
        assert!(tr[on_at - 1].abs() < 1e-9, "{id:?} before the toggle");
        for k in 0..ramp + 100 {
            let w = ((k + 1).min(ramp)) as f64 / ramp as f64;
            let on = tr[on_at + k];
            assert!(
                (on - full_db * w).abs() < 1e-4,
                "{id:?} on, sample {k}: {on} vs {}",
                full_db * w
            );
            let off = tr[off_at + k];
            assert!(
                (off - full_db * (1.0 - w)).abs() < 1e-4,
                "{id:?} off, sample {k}: {off} vs {}",
                full_db * (1.0 - w)
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-13: look-ahead — latency, restart request, pre-applied gain
// ---------------------------------------------------------------------------------------------

/// Renders `x` in fixed blocks, returning the output and the number of blocks in which the module
/// asked for a restart (`ctx.requested(HostRequest::Restart)`).
fn render_counting_restarts(
    m: &mut dyn Module,
    x: &[f32],
    changes: &[Change],
    block: usize,
) -> (Vec<f32>, usize) {
    let mut y = vec![0.0f32; x.len()];
    let mut out_events = OutputEvents::with_capacity(DEFAULT_EVENT_CAPACITY);
    let mut events: Vec<ParamEvent> = Vec::with_capacity(changes.len().max(1));
    let (mut pos, mut next, mut restarts) = (0usize, 0usize, 0usize);
    while pos < x.len() {
        let n = block.min(x.len() - pos);
        events.clear();
        while let Some(&(at, id, value)) = changes.get(next)
            && at < pos + n
        {
            events.push(ParamEvent {
                offset: (at - pos) as u32,
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
        restarts += usize::from(ctx.requested(HostRequest::Restart));
        pos += n;
    }
    (y, restarts)
}

#[test]
fn lookahead_latency_table() {
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        for step in 0..=20u32 {
            let ms = f64::from(step);
            let m = activated(make, &[("lookahead_ms", ms)], fs);
            let want = (ms * fs / 1000.0).round() as u32;
            assert_eq!(m.latency_samples(), want, "{fs} Hz, {ms} ms");
            assert_eq!(m.tail(), Tail::Samples(u64::from(want)), "{fs} Hz, {ms} ms");
        }
    }
}

#[test]
fn a_lookahead_event_requests_one_restart_and_does_not_change_the_output() {
    let x = burst_signal(0xAC13, 48_000);
    let s = [
        ("lookahead_ms", 5.0),
        ("compressor_enabled", 1.0),
        ("limiter_enabled", 1.0),
    ];
    let (want, none) = render_counting_restarts(&mut *dynamics(&s), &x, &[], 512);
    assert_eq!(none, 0, "no event, no restart request");

    let changes = [(12_345usize, Dynamics::LOOKAHEAD_MS, 12.0)];
    let (got, restarts) = render_counting_restarts(&mut *dynamics(&s), &x, &changes, 512);
    assert_eq!(restarts, 1, "exactly one restart request per changed value");
    assert_eq!(
        bit_equal(&got, &want),
        None,
        "the running instance is unchanged"
    );
    // The mirror reports the new target at once, and the state stores it.
    let mut m = dynamics(&s);
    let (_, _) = render_counting_restarts(&mut *m, &x, &changes, 512);
    assert_eq!(m.param_value(Dynamics::LOOKAHEAD_MS), Some(12.0));
    assert_eq!(m.latency_samples(), 240, "latency is fixed while active");
    assert_eq!(
        m.save_state().unwrap().params["lookahead_ms"],
        12.0,
        "save_state stores the target"
    );

    // Back to the active value: nothing more is asked for.
    let changes = [
        (1_000usize, Dynamics::LOOKAHEAD_MS, 12.0),
        (2_000, Dynamics::LOOKAHEAD_MS, 5.0),
        (3_000, Dynamics::LOOKAHEAD_MS, 12.0),
    ];
    let (_, restarts) = render_counting_restarts(&mut *dynamics(&s), &x, &changes, 512);
    assert_eq!(restarts, 2, "one request per changed value");
}

#[test]
fn lookahead_applies_the_reduction_before_the_transient() {
    // Look-ahead 5 ms with a 0.5 ms attack = 10 τ: ≥ 99.99 % of the target is already applied at
    // the first output sample of the step (latency-aligned).
    let la = 240usize;
    let s = [
        ("detection", 0.0),
        ("knee_db", 0.0),
        ("lookahead_ms", 5.0),
        ("compressor_threshold_db", -20.0),
        ("compressor_ratio", 4.0),
        ("compressor_attack_ms", 0.5),
    ];
    let step = 24_000usize;
    let x = dc_segments(&[(-40.0, step), (-10.0, 24_000)]);
    let mut m = dynamics(&s);
    assert_eq!(m.latency_samples() as usize, la);
    let y = render(&mut *m, &x, &[], Blocks::Fixed(4096));
    // Output sample `step + la` carries input sample `step`, the first sample of the step.
    let g = to_db(f64::from(y[step + la]) / f64::from(x[step]));
    let target = -7.5;
    assert!(
        g / target >= 0.9999,
        "gain {g:.6} dB is only {:.5} of the target {target} dB",
        g / target
    );
}

// ---------------------------------------------------------------------------------------------
// AC-14: telemetry
// ---------------------------------------------------------------------------------------------

/// Renders 24 576 samples of a steady sine; returns the measured gain and the telemetry of the
/// last 1024-sample block (read after consuming everything before it).
fn steady_telemetry(settings: &[(&str, f64)], level_dbfs: f64) -> (f64, Vec<f32>) {
    let n = 24 * 1024;
    let x = sine(997.0, level_dbfs, n, SR);
    let mut m = dynamics(settings);
    let t = telemetry(&*m).unwrap();
    let mut snapshot = Vec::new();
    let y = render_observed(&mut *m, &x, &[], Blocks::Fixed(1024), |start, len| {
        if start + len == n - 1024 {
            read_all(&*t);
        } else if start + len == n {
            snapshot = read_all(&*t);
        }
    });
    (to_db(peak(&y[n / 2..]) / peak(&x[n / 2..])), snapshot)
}

#[test]
fn telemetry_matches_measured_contributions() {
    const TOTAL: usize = Dynamics::TELEMETRY_GR_TOTAL;
    const COMP: usize = Dynamics::TELEMETRY_GR_COMPRESSOR;
    const LIM: usize = Dynamics::TELEMETRY_GR_LIMITER;
    const LEVEL: usize = Dynamics::TELEMETRY_INPUT_LEVEL;
    let near = |a: f32, b: f64| (f64::from(a) - b).abs() <= 0.1;
    let mut worst: f64 = 0.0;

    // Compressor alone.
    let comp = [
        ("detection", 0.0),
        ("knee_db", 0.0),
        ("compressor_threshold_db", -20.0),
        ("compressor_ratio", 4.0),
    ];
    for level in [-30.0, -10.0, 0.0] {
        let (g, t) = steady_telemetry(&comp, level);
        assert!(
            near(t[COMP], g) && near(t[TOTAL], g),
            "{level}: {g} vs {t:?}"
        );
        assert_eq!((t[LIM], t[1], t[2], t[6]), (0.0, 0.0, 0.0, 0.0));
        assert!(near(t[LEVEL], level), "level {level}: {t:?}");
        worst = worst.max((f64::from(t[COMP]) - g).abs());
    }
    // Limiter alone.
    let lim = [
        ("compressor_enabled", 0.0),
        ("limiter_enabled", 1.0),
        ("limiter_threshold_db", -6.0),
    ];
    let (g, t) = steady_telemetry(&lim, 0.0);
    assert!(
        near(t[LIM], g) && near(t[TOTAL], g) && (g + 6.0).abs() < 0.05,
        "{g} {t:?}"
    );
    assert_eq!(t[COMP], 0.0);
    worst = worst.max((f64::from(t[LIM]) - g).abs());
    // Both, with makeup: total = Σ sections, makeup excluded.
    let mut both = comp.to_vec();
    both.extend([
        ("compressor_makeup_db", 6.0),
        ("limiter_enabled", 1.0),
        ("limiter_threshold_db", -12.0),
    ]);
    let (g, t) = steady_telemetry(&both, 0.0);
    assert!(near(t[COMP], -15.0) && near(t[LIM], -3.0), "{t:?}");
    assert!(
        near(t[TOTAL], f64::from(t[COMP]) + f64::from(t[LIM])),
        "{t:?}"
    );
    assert!(
        near(t[TOTAL], g - 6.0),
        "total {t:?} vs measured {g} − makeup"
    );
    worst = worst.max((f64::from(t[TOTAL]) - (g - 6.0)).abs());
    println!("AC-14 telemetry: worst GR error {worst:.4} dB");

    // Initial values after activate and after reset (once the reader consumed).
    let mut m = dynamics(&comp);
    let t = telemetry(&*m).unwrap();
    assert_eq!(read_all(&*t), [0.0, 0.0, 0.0, 0.0, 0.0, -100.0, 0.0]);
    render(
        &mut *m,
        &sine(997.0, 0.0, 4096, SR),
        &[],
        Blocks::Fixed(1024),
    );
    assert!(read_all(&*t)[COMP] < -5.0);
    m.reset();
    assert_eq!(read_all(&*t), [0.0, 0.0, 0.0, 0.0, 0.0, -100.0, 0.0]);
}

#[test]
fn autogate_and_expander_telemetry() {
    const AG: usize = Dynamics::TELEMETRY_GR_AUTOGATE;
    const EX: usize = Dynamics::TELEMETRY_GR_EXPANDER;
    const TOTAL: usize = Dynamics::TELEMETRY_GR_TOTAL;
    const LAMP: usize = Dynamics::TELEMETRY_AUTOGATE_OPEN;
    let near = |a: f32, b: f64| (f64::from(a) - b).abs() <= 0.1;

    // Expander alone: the channel equals the measured contribution.
    let exp = [
        ("detection", 0.0),
        ("knee_db", 0.0),
        ("compressor_enabled", 0.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -30.0),
        ("expander_ratio", 3.0),
        ("expander_release_ms", 20.0),
    ];
    for level in [-40.0, -35.0, -20.0] {
        let (g, t) = steady_telemetry(&exp, level);
        assert!(near(t[EX], g) && near(t[TOTAL], g), "{level}: {g} vs {t:?}");
        assert_eq!((t[AG], t[LAMP]), (0.0, 0.0));
    }

    // AutoGate open: 0 dB and the lamp lit. Closed: the channel pins at the −60 dB floor.
    let gate = [
        ("compressor_enabled", 0.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -40.0),
    ];
    let (g, t) = steady_telemetry(&gate, -20.0);
    assert!(
        g.abs() <= 0.01 && t[AG] == 0.0 && t[LAMP] == 1.0,
        "{g} {t:?}"
    );
    let (_, t) = steady_telemetry(&gate, -50.0);
    assert_eq!((t[AG], t[TOTAL], t[LAMP]), (-60.0, -60.0, 0.0), "{t:?}");

    // Both plus the compressor: total = Σ sections.
    let mut both = exp.to_vec();
    both.extend([
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -60.0),
        ("compressor_enabled", 1.0),
        ("compressor_threshold_db", -40.0),
        ("compressor_ratio", 2.0),
    ]);
    let (g, t) = steady_telemetry(&both, -35.0);
    assert!(t[AG] == 0.0 && t[LAMP] == 1.0, "{t:?}");
    assert!(
        near(
            t[TOTAL],
            f64::from(t[EX]) + f64::from(t[Dynamics::TELEMETRY_GR_COMPRESSOR])
        ),
        "{t:?}"
    );
    assert!(near(t[TOTAL], g), "total {t:?} vs measured {g}");
}

#[test]
fn a_short_gain_reduction_dip_survives_until_the_next_read() {
    // A 5 ms dip to −12 dB GR between two reads 16.7 ms apart (Hold::Min).
    const LIM: usize = Dynamics::TELEMETRY_GR_LIMITER;
    let s = [
        ("compressor_enabled", 0.0),
        ("limiter_enabled", 1.0),
        ("limiter_threshold_db", -20.0),
        ("limiter_attack_ms", 0.1),
        ("limiter_release_ms", 1.0),
    ];
    let quiet = 4_800usize;
    let dip = 240usize; // 5 ms
    let x = dc_segments(&[(-30.0, quiet), (-8.0, dip), (-30.0, 4_800)]);
    let mut m = dynamics(&s);
    let t = telemetry(&*m).unwrap();
    // 200-sample process blocks, one read every 4 blocks (800 samples = 16.7 ms).
    let mut reads: Vec<f32> = Vec::new();
    let mut blocks = 0usize;
    render_observed(&mut *m, &x, &[], Blocks::Fixed(200), |_, _| {
        blocks += 1;
        if blocks.is_multiple_of(4) {
            reads.push(read_all(&*t)[LIM]);
        }
    });
    let worst = reads.iter().fold(0.0f32, |a, &b| a.min(b));
    assert!(
        worst <= -11.9,
        "the deepest reported reduction was {worst} dB (reads {reads:?})"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-16: realtime = offline, bit-identical and deterministic
// ---------------------------------------------------------------------------------------------

fn mixed_signal(n: usize, fs: f64) -> Vec<f32> {
    let noise = burst_signal(0xAC16, n);
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            // Log sweep 20 Hz → 20 kHz over the signal, at −12 dBFS, plus the bursts.
            let f = 20.0 * 1000f64.powf(i as f64 / n as f64);
            phase = (phase + std::f64::consts::TAU * f.min(0.45 * fs) / fs) % std::f64::consts::TAU;
            (0.25 * phase.sin()) as f32 + noise[i]
        })
        .collect()
}

#[test]
fn realtime_equals_offline_bit_identical() {
    let settings = [
        ("compressor_enabled", 1.0),
        ("limiter_enabled", 1.0),
        ("detection", 1.0),
        ("limiter_threshold_db", -6.0),
    ];
    let script: [(ParamId, f64); 30] = [
        (Dynamics::COMPRESSOR_THRESHOLD_DB, -30.0),
        (Dynamics::COMPRESSOR_RATIO, 8.0),
        (Dynamics::KNEE_DB, 0.0),
        (Dynamics::COMPRESSOR_ATTACK_MS, 1.0),
        (Dynamics::DETECTION, 0.0),
        (Dynamics::LIMITER_THRESHOLD_DB, -12.0),
        (Dynamics::COMPRESSOR_MAKEUP_DB, 6.0),
        (Dynamics::LIMITER_ENABLED, 0.0),
        (Dynamics::COMPRESSOR_RELEASE_MS, 500.0),
        (Dynamics::LIMITER_ATTACK_MS, 0.1),
        (Dynamics::COMPRESSOR_ENABLED, 0.0),
        (Dynamics::AUTOGATE_ENABLED, 1.0),
        (Dynamics::EXPANDER_ENABLED, 1.0),
        (Dynamics::LIMITER_ENABLED, 1.0),
        (Dynamics::COMPRESSOR_ENABLED, 1.0),
        (Dynamics::DETECTION, 1.0),
        (Dynamics::KNEE_DB, 12.0),
        (Dynamics::LIMITER_RELEASE_MS, 20.0),
        (Dynamics::COMPRESSOR_THRESHOLD_DB, -10.0),
        (Dynamics::COMPRESSOR_RATIO, 1.5),
        (Dynamics::LOOKAHEAD_MS, 5.0),
        (Dynamics::EXPANDER_THRESHOLD_DB, -30.0),
        (Dynamics::AUTOGATE_THRESHOLD_DB, -20.0),
        (Dynamics::COMPRESSOR_MAKEUP_DB, 0.0),
        (Dynamics::LIMITER_THRESHOLD_DB, -1.0),
        (Dynamics::COMPRESSOR_ATTACK_MS, 50.0),
        (Dynamics::KNEE_DB, 6.0),
        (Dynamics::COMPRESSOR_RELEASE_MS, 50.0),
        (Dynamics::EXPANDER_RATIO, 4.0),
        (Dynamics::LIMITER_ENABLED, 0.0),
    ];
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        let n = (3.0 * fs) as usize;
        let x = mixed_signal(n, fs);
        let changes: Vec<Change> = script
            .iter()
            .enumerate()
            .map(|(i, &(id, v))| ((i + 1) * n / 31 + i, id, v))
            .collect();
        let offline = |seed: u64| {
            let _ = seed;
            render(
                &mut *activated_with(make, &settings, fs, ProcessMode::Offline, 4096),
                &x,
                &changes,
                Blocks::Fixed(4096),
            )
        };
        let a = offline(0);
        let b = offline(1);
        assert_eq!(fnv1a(&a), fnv1a(&b), "two offline renders differ ({fs} Hz)");
        let rt = render(
            &mut *activated_with(make, &settings, fs, ProcessMode::Realtime, 1024),
            &x,
            &changes,
            Blocks::Random {
                seed: 0xAC16 ^ fs as u64,
                max: 1024,
            },
        );
        assert_eq!(bit_equal(&rt, &a), None, "realtime ≠ offline at {fs} Hz");
        println!("AC-16 {fs} Hz: FNV-1a {:016x}", fnv1a(&a));
    }
}

// ---------------------------------------------------------------------------------------------
// AC-15: §4.3 zipper, SPEC-016 §5.1 setups (compressor, limiter, enables, detection)
// ---------------------------------------------------------------------------------------------

#[test]
fn zipper_compressor_threshold() {
    assert_zipper(
        make,
        &[],
        Dynamics::COMPRESSOR_THRESHOLD_DB,
        &zipper_signal(),
    );
}

#[test]
fn zipper_compressor_ratio() {
    let s = [("compressor_threshold_db", -40.0)];
    assert_zipper(make, &s, Dynamics::COMPRESSOR_RATIO, &zipper_signal());
}

#[test]
fn zipper_compressor_makeup() {
    let s = [("compressor_threshold_db", -40.0)];
    assert_zipper(make, &s, Dynamics::COMPRESSOR_MAKEUP_DB, &zipper_signal());
}

#[test]
fn zipper_knee() {
    let s = [
        ("compressor_threshold_db", -23.0),
        ("compressor_ratio", 4.0),
    ];
    assert_zipper(make, &s, Dynamics::KNEE_DB, &zipper_signal());
}

#[test]
fn zipper_compressor_attack_and_release() {
    let x = am_tone(-32.0, -20.0, ZIPPER_LEN);
    let s = [
        ("compressor_threshold_db", -30.0),
        ("compressor_ratio", 4.0),
    ];
    assert_zipper(make, &s, Dynamics::COMPRESSOR_ATTACK_MS, &x);
    assert_zipper(make, &s, Dynamics::COMPRESSOR_RELEASE_MS, &x);
}

#[test]
fn zipper_limiter_threshold() {
    let x = sine(997.0, -10.0, ZIPPER_LEN, SR);
    let s = [("compressor_enabled", 0.0), ("limiter_enabled", 1.0)];
    assert_zipper(make, &s, Dynamics::LIMITER_THRESHOLD_DB, &x);
}

#[test]
fn zipper_limiter_attack_and_release() {
    let x = am_tone(-10.0, -2.0, ZIPPER_LEN);
    let s = [
        ("compressor_enabled", 0.0),
        ("limiter_enabled", 1.0),
        ("limiter_threshold_db", -6.0),
    ];
    assert_zipper(make, &s, Dynamics::LIMITER_ATTACK_MS, &x);
    assert_zipper(make, &s, Dynamics::LIMITER_RELEASE_MS, &x);
}

#[test]
fn zipper_enables_and_detection() {
    let x = zipper_signal();
    // Each section toggled alone (the harness sets the enable itself).
    let comp = [
        ("compressor_threshold_db", -40.0),
        ("compressor_ratio", 4.0),
    ];
    assert_zipper(make, &comp, Dynamics::COMPRESSOR_ENABLED, &x);
    let lim = [("compressor_enabled", 0.0), ("limiter_threshold_db", -30.0)];
    assert_zipper(make, &lim, Dynamics::LIMITER_ENABLED, &x);
    assert_zipper(make, &comp, Dynamics::DETECTION, &x);
}

#[test]
fn zipper_expander_threshold_and_ratio() {
    let x = sine(997.0, -30.0, ZIPPER_LEN, SR);
    let s = [("compressor_enabled", 0.0), ("expander_enabled", 1.0)];
    assert_zipper(make, &s, Dynamics::EXPANDER_THRESHOLD_DB, &x);
    let s = [
        ("compressor_enabled", 0.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -20.0),
    ];
    assert_zipper(make, &s, Dynamics::EXPANDER_RATIO, &x);
}

#[test]
fn zipper_expander_attack_and_release() {
    let x = am_tone(-32.0, -20.0, ZIPPER_LEN);
    let s = [
        ("compressor_enabled", 0.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -26.0),
        ("expander_ratio", 4.0),
    ];
    assert_zipper(make, &s, Dynamics::EXPANDER_ATTACK_MS, &x);
    assert_zipper(make, &s, Dynamics::EXPANDER_RELEASE_MS, &x);
}

#[test]
fn zipper_autogate_threshold() {
    let x = sine(997.0, -40.0, ZIPPER_LEN, SR);
    let s = [("compressor_enabled", 0.0), ("autogate_enabled", 1.0)];
    assert_zipper(make, &s, Dynamics::AUTOGATE_THRESHOLD_DB, &x);
}

#[test]
fn zipper_autogate_attack_release_and_hold() {
    let x = keyed_tone(ZIPPER_LEN);
    let s = [
        ("compressor_enabled", 0.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -40.0),
        ("autogate_hold_ms", 0.1),
    ];
    assert_zipper(make, &s, Dynamics::AUTOGATE_ATTACK_MS, &x);
    assert_zipper(make, &s, Dynamics::AUTOGATE_RELEASE_MS, &x);
    let s = [
        ("compressor_enabled", 0.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -40.0),
        ("autogate_release_ms", 5.0),
    ];
    assert_zipper(make, &s, Dynamics::AUTOGATE_HOLD_MS, &x);
}

#[test]
fn zipper_autogate_and_expander_enables() {
    let x = zipper_signal();
    let ag = [
        ("compressor_enabled", 0.0),
        ("autogate_threshold_db", -10.0),
    ];
    assert_zipper(make, &ag, Dynamics::AUTOGATE_ENABLED, &x);
    let ex = [
        ("compressor_enabled", 0.0),
        ("expander_threshold_db", -10.0),
        ("expander_ratio", 4.0),
    ];
    assert_zipper(make, &ex, Dynamics::EXPANDER_ENABLED, &x);
}

// ---------------------------------------------------------------------------------------------
// AC-18: silence and denormals
// ---------------------------------------------------------------------------------------------

#[test]
fn silence_after_a_loud_passage_is_exactly_zero() {
    // FTZ/DAZ is not enabled in tests (SPEC-016 §4.13: the snaps, not the host guard, carry the
    // denormal safety). All sections on, look-ahead 5 ms.
    let s = [
        ("lookahead_ms", 5.0),
        ("detection", 1.0),
        ("autogate_enabled", 1.0),
        ("autogate_threshold_db", -50.0),
        ("expander_enabled", 1.0),
        ("expander_threshold_db", -45.0),
        ("compressor_enabled", 1.0),
        ("limiter_enabled", 1.0),
    ];
    let la = 240usize;
    let loud = 48_000usize;
    let silence = 10 * 48_000usize;
    let mut x = white(0xAC18, 1.0, loud);
    x.extend(std::iter::repeat_n(0.0f32, silence));
    let y = render(
        &mut *dynamics(&s),
        &x,
        &[],
        Blocks::Random {
            seed: 0xAC18,
            max: 1024,
        },
    );
    for (i, &v) in y.iter().enumerate().skip(loud + la) {
        assert_eq!(v, 0.0, "sample {i} after the input went silent is {v}");
    }
    assert!(
        y.iter().all(|v| v.is_finite() && !v.is_subnormal()),
        "no subnormal or non-finite output sample"
    );
    // The module recovers: a fresh burst after the silence is processed normally.
    let after = render(
        &mut *dynamics(&s),
        &white(0xAC18 ^ 7, 0.5, 4_800),
        &[],
        Blocks::Fixed(512),
    );
    assert!(peak(&after) > 0.0 && after.iter().all(|v| v.is_finite()));
}
