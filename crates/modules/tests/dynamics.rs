//! S3-02 Dynamics (compressor + limiter; SPEC-016 subset): schema and ModuleTestHost (AC-1),
//! compressor static curves (AC-2/AC-3), limiter steady state (AC-6), time constants (AC-8),
//! disabled sections inert / all-off passthrough (AC-10/AC-11), enable crossfades (AC-12),
//! telemetry (AC-14), §4.3 zipper (AC-15), realtime = offline bit-identical (AC-16).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time

mod common;

use std::sync::Arc;

use common::*;
use vox_module_api::test_util::{ModuleTestHost, TestRng, ZIPPER_LEN, zipper_signal};
use vox_module_api::{
    Module, ModuleFactory, ParamFlags, ParamId, ProcessMode, Taper, TelemetryKind, features,
    telemetry,
};
use vox_modules::{Dynamics, DynamicsFactory, builtin_factories};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;
const W_PK: f64 = 240.0;
const SINE_PEAK_TO_RMS_DB: f64 = 3.010_299_956_639_812;

fn make() -> Box<dyn Module> {
    Box::new(Dynamics::new())
}

fn dynamics(settings: &[(&str, f64)]) -> Box<dyn Module> {
    activated(make, settings, SR)
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
        let hidden = p.flags.contains(ParamFlags::HIDDEN);
        assert_eq!(
            hidden,
            Dynamics::NOT_YET_AVAILABLE.contains(&p.id),
            "{} hidden",
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
    let sections: [(&str, &[&'static str], bool); 4] = [
        ("compressor_enabled", &COMP_KEYS, false),
        ("limiter_enabled", &LIM_KEYS, false),
        ("autogate_enabled", &AG_KEYS, true),
        ("expander_enabled", &EX_KEYS, true),
    ];
    for (si, &(enable, keys, stub)) in sections.iter().enumerate() {
        let mut rng = TestRng::new(0xAC10 + si as u64);
        // The other sections at seeded random settings, the live ones enabled.
        let mut others = random_values(&mut rng, &GLOBAL_KEYS);
        for (other_enable, other_keys, _) in &sections {
            if *other_enable != enable {
                others.extend(random_values(&mut rng, other_keys));
                if !stub || *other_enable == "compressor_enabled" {
                    others.push((other_enable, 1.0));
                }
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
            // A stub section is inert even when switched on (not yet available).
            let on = if stub && rng.chance(0.5) { 1.0 } else { 0.0 };
            s.push((enable, on));
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

#[test]
fn all_sections_off_is_a_bit_exact_passthrough() {
    let signals = [white(0xAC11, 1.0, 48_000), burst_signal(3, 33_600)];
    let mut rng = TestRng::new(0xAC11);
    for set in 0..20u64 {
        let mut s = Vec::new();
        for keys in [&GLOBAL_KEYS[..], &COMP_KEYS, &LIM_KEYS, &AG_KEYS, &EX_KEYS] {
            s.extend(random_values(&mut rng, keys));
        }
        s.push(("compressor_enabled", 0.0));
        s.push(("limiter_enabled", 0.0));
        s.push(("autogate_enabled", 1.0));
        s.push(("expander_enabled", 1.0));
        for x in &signals {
            let y = render(
                &mut *dynamics(&s),
                x,
                &[],
                Blocks::Random {
                    seed: set,
                    max: 1024,
                },
            );
            assert_eq!(bit_equal(&y, x), None, "set {set}: {s:?}");
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
