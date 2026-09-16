//! S3-02 Noise Gate (SPEC-013 subset): schema and ModuleTestHost (AC-1), open / close
//! thresholds (AC-2/AC-3), no chatter (AC-4), hold (AC-5), attack shape (AC-6), release (AC-7),
//! range (AC-8), sidechain HPF (AC-9), §4.3 zipper (AC-11), realtime = offline (AC-12),
//! telemetry (AC-13).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time

mod common;

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use common::*;
use vox_dsp::dynamics::sidechain::ButterworthHighpass4;
use vox_module_api::test_util::no_alloc;
use vox_module_api::test_util::{ModuleTestHost, TestRng, ZIPPER_LEN};
use vox_module_api::{
    CurveBranch, Module, ModuleFactory, ParamFlags, ParamId, ProcessMode, Taper, TelemetryKind,
    features, telemetry,
};
use vox_modules::{NoiseGate, NoiseGateFactory, builtin_factories};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;
const W_PK: usize = 240;
const OPEN: usize = NoiseGate::TELEMETRY_GATE_OPEN;
const GAIN: usize = NoiseGate::TELEMETRY_GAIN_DB;
const LEVEL: usize = NoiseGate::TELEMETRY_SIDECHAIN_LEVEL;

fn make() -> Box<dyn Module> {
    Box::new(NoiseGate::new())
}

/// Tests isolate the gate core: the sidechain HPF is off unless stated (SPEC-013 §5).
fn gate(settings: &[(&str, f64)]) -> Box<dyn Module> {
    let mut s = vec![("sc_hpf_enabled", 0.0)];
    s.extend_from_slice(settings);
    activated(make, &s, SR)
}

/// Renders with 256-frame blocks, reading the gate lamp after every block.
fn render_lamp(
    m: &mut dyn Module,
    x: &[f32],
    block: usize,
) -> (Vec<f32>, Vec<(usize, usize, f32)>) {
    let t = telemetry(m).unwrap();
    let mut lamps = Vec::new();
    let y = render_observed(m, x, &[], Blocks::Fixed(block), |s, n| {
        lamps.push((s, n, t.read(OPEN)));
    });
    (y, lamps)
}

// ---------------------------------------------------------------------------------------------
// AC-1
// ---------------------------------------------------------------------------------------------

#[test]
fn schema_identity_and_registration() {
    let f = NoiseGateFactory::new();
    let d = f.descriptor();
    assert_eq!(d.id, "org.powervoice.noise-gate");
    assert_eq!(d.version.to_string(), "1.0.0");
    assert_eq!(d.state_format_version, 1);
    assert_eq!(
        d.features,
        [features::AUDIO_EFFECT, features::GATE, features::MONO]
    );
    let m = f.create().unwrap();
    type Row<'a> = (u32, &'a str, Option<u32>, f64, f64, f64, f32);
    let got: Vec<Row> = m
        .params()
        .iter()
        .map(|p| {
            (
                p.id.0,
                p.key.as_str(),
                p.group.map(|g| g.0),
                p.min,
                p.max,
                p.default,
                p.smoothing_ms,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (1, "threshold_db", None, -80.0, 0.0, -40.0, 0.0),
            (2, "hysteresis_db", None, 0.0, 20.0, 6.0, 0.0),
            (3, "attack_ms", None, 0.1, 100.0, 2.0, 0.0),
            (4, "hold_ms", None, 0.1, 1000.0, 50.0, 0.0),
            (5, "release_ms", None, 1.0, 2000.0, 100.0, 0.0),
            (6, "range_db", None, -100.0, 0.0, -30.0, 20.0),
            (7, "sc_hpf_enabled", Some(1), 0.0, 1.0, 1.0, 0.0),
            (8, "sc_hpf_hz", Some(1), 20.0, 2000.0, 100.0, 0.0),
            (9, "lookahead_ms", None, 0.0, 20.0, 0.0, 0.0),
        ]
    );
    let g = &m.groups()[0];
    assert_eq!(
        (
            g.id.0,
            g.key.as_str(),
            g.enable_param,
            g.collapsed_by_default
        ),
        (1, "sidechain", Some(NoiseGate::SC_HPF_ENABLED), false)
    );
    let range = &m.params()[5];
    assert_eq!(
        range.taper,
        Taper::Db {
            neg_inf_at_min: true
        }
    );
    assert_eq!(range.value_to_text(-100.0), "-inf dB");
    assert_eq!(range.text_to_value("-inf"), Some(-100.0));
    assert_eq!(range.value_to_text(-30.0), "-30.0 dB");
    assert_eq!(m.params()[7].value_to_text(100.0), "100 Hz");
    assert!(m.params()[8].flags.contains(ParamFlags::HIDDEN));
    assert!(m.params()[..8].iter().all(
        |p| !p.flags.contains(ParamFlags::HIDDEN) && p.flags.contains(ParamFlags::AUTOMATABLE)
    ));
    let t = telemetry(&*m).unwrap();
    let ch: Vec<(&str, TelemetryKind, Option<u32>, f64)> = t
        .channels()
        .iter()
        .map(|c| (c.key.as_str(), c.kind, c.group.map(|g| g.0), c.min))
        .collect();
    assert_eq!(
        ch,
        [
            ("gate_open", TelemetryKind::Indicator, None, 0.0),
            ("gain_db", TelemetryKind::GainReduction, None, -100.0),
            (
                "sidechain_level_dbfs",
                TelemetryKind::Level,
                Some(1),
                -100.0
            ),
        ]
    );
    assert!(
        builtin_factories()
            .iter()
            .any(|f| f.descriptor().id == NoiseGate::ID)
    );
    ModuleTestHost::from_factory(Arc::new(NoiseGateFactory::new()))
        .check_schema()
        .unwrap();
}

#[test]
fn passes_module_test_host_with_all_params_delayed() {
    let mut host = ModuleTestHost::new(make);
    for p in make().params() {
        host = host.allow_delayed_effect(p.id);
    }
    let report = host.assert_passes();
    assert!(report.alloc_checked);
    assert!(report.zero_length_blocks > 0);
}

// ---------------------------------------------------------------------------------------------
// AC-2 / AC-3: thresholds
// ---------------------------------------------------------------------------------------------

/// 200 ms silence, then three 300 ms 997 Hz bursts separated by 1 s of silence.
fn burst_train(level_dbfs: f64) -> (Vec<f32>, Vec<(usize, usize)>) {
    let (lead, on, off) = (9_600, 14_400, 48_000);
    let mut x = vec![0.0f32; lead];
    let mut spans = Vec::new();
    for _ in 0..3 {
        let s = x.len();
        x.extend(sine(997.0, level_dbfs, on, SR));
        spans.push((s, s + on));
        x.extend(std::iter::repeat_n(0.0, off));
    }
    (x, spans)
}

#[test]
fn open_threshold_is_exact() {
    for t in [-60.0, -40.0, -20.0] {
        let (x, spans) = burst_train(t + 0.5);
        let (y, lamps) = render_lamp(&mut *gate(&[("threshold_db", t)]), &x, 256);
        for &(s, e) in &spans {
            for i in s + W_PK..e {
                assert_eq!(
                    y[i].to_bits(),
                    x[i].to_bits(),
                    "T {t}: sample {i} not passed"
                );
            }
            assert!(
                lamps
                    .iter()
                    .filter(|l| l.0 >= s + W_PK && l.0 + l.1 <= e)
                    .all(|l| l.2 == 1.0),
                "T {t}: lamp off during a burst"
            );
        }
        let (x, _) = burst_train(t - 0.5);
        let (y, lamps) = render_lamp(&mut *gate(&[("threshold_db", t)]), &x, 256);
        let f = lin(-30.0) as f32;
        for i in 0..x.len() {
            assert_eq!(
                y[i].to_bits(),
                (x[i] * f).to_bits(),
                "T {t}: gate opened at {i}"
            );
        }
        assert!(lamps.iter().all(|l| l.2 == 0.0));
    }
}

/// True if a 50 ms burst at `level` (997 Hz) opens a gate with threshold `t`.
fn opens(t: f64, level: f64) -> bool {
    let x = sine(997.0, level, 2_400, SR);
    let (_, lamps) = render_lamp(&mut *gate(&[("threshold_db", t)]), &x, 2_400);
    lamps.iter().any(|l| l.2 == 1.0)
}

/// True if, after a 50 ms opening burst at −30 dBFS, a 997 Hz tone at `level` closes the gate
/// (threshold −40, hysteresis `h`, hold 0.1 ms).
fn closes(h: f64, level: f64) -> bool {
    let x = sine_segments(997.0, &[(-30.0, 2_400), (level, 9_600)], SR);
    let s = [("hysteresis_db", h), ("hold_ms", 0.1)];
    let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(4096));
    (2_400..x.len()).any(|i| y[i].to_bits() != x[i].to_bits())
}

#[test]
fn close_threshold_is_threshold_minus_hysteresis() {
    for h in [3.0, 6.0, 12.0] {
        let t = -40.0;
        let s = [("hysteresis_db", h)];
        // T − h + 0.5 keeps the opened gate open (gain exactly 1.0) for 2 s.
        let x = sine_segments(997.0, &[(-30.0, 4_800), (t - h + 0.5, 96_000)], SR);
        let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(1024));
        assert_eq!(bit_equal(&y[W_PK..], &x[W_PK..]), None, "h {h}: closed");
        // T − h − 0.5 starts the release after W_pk + hold.
        let x = sine_segments(997.0, &[(-30.0, 4_800), (t - h - 0.5, 9_600)], SR);
        let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(1024));
        let e = 4_799;
        let start = (e + 1..x.len())
            .find(|&i| y[i] != x[i])
            .expect("never closed");
        let want = e + W_PK + 2_400;
        assert!(
            start.abs_diff(want) <= 2,
            "h {h}: release at {start}, expected {want}"
        );
    }
    // Measured thresholds (report): resolution 0.005 dB.
    let steps: Vec<f64> = (0..=40).map(|k| -0.1 + 0.005 * f64::from(k)).collect();
    let open_at = steps.iter().find(|&&d| opens(-40.0, -40.0 + d)).copied();
    let close_at = steps
        .iter()
        .rev()
        .find(|&&d| closes(6.0, -46.0 + d))
        .copied();
    let (open_at, close_at) = (
        open_at.expect("never opened"),
        close_at.expect("never closed"),
    );
    assert!(open_at.abs() <= 0.5 && close_at.abs() <= 0.5);
    println!(
        "AC-2/3 measured: opens at T{open_at:+.3} dB (T −40), closes below T−h{close_at:+.3} dB \
         (h 6); sample-peak detection of a 997 Hz sine reads ≤ 0.019 dB low"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-4: no chatter
// ---------------------------------------------------------------------------------------------

/// 997 Hz tone whose level alternates T + 0.5 / T − 0.5 dB every 10 ms (1 ms raised-cosine
/// transitions).
fn dither_tone(t: f64, n: usize) -> Vec<f32> {
    let (seg, tr) = (480usize, 48usize);
    (0..n)
        .map(|i| {
            let (k, p) = (i / seg, i % seg);
            let cur = if k % 2 == 0 { 0.5 } else { -0.5 };
            let d = if k > 0 && p < tr {
                -cur + 2.0 * cur * (1.0 - (PI * p as f64 / tr as f64).cos()) / 2.0
            } else {
                cur
            };
            (lin(t + d) * (TAU * 997.0 * i as f64 / SR).sin()) as f32
        })
        .collect()
}

fn transitions(hysteresis: f64, x: &[f32]) -> (usize, usize) {
    let s = [("hysteresis_db", hysteresis), ("hold_ms", 0.1)];
    let (_, lamps) = render_lamp(&mut *gate(&s), x, 1);
    let mut prev = 0.0;
    let (mut opened, mut closed) = (0, 0);
    for l in lamps {
        if l.2 != prev {
            if l.2 == 1.0 {
                opened += 1
            } else {
                closed += 1
            }
            prev = l.2;
        }
    }
    (opened, closed)
}

#[test]
fn hysteresis_prevents_chatter() {
    let x = dither_tone(-40.0, 96_000);
    assert_eq!(transitions(3.0, &x), (1, 0));
    let s = [("hysteresis_db", 3.0), ("hold_ms", 0.1)];
    let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(1024));
    assert_eq!(
        bit_equal(&y[W_PK..], &x[W_PK..]),
        None,
        "gain not 1.0 after the attack"
    );
    // Discrimination: without hysteresis the same signal chatters.
    let (opened, _) = transitions(0.0, &x);
    assert!(opened >= 40, "only {opened} openings without hysteresis");
    println!("AC-4: 1 transition with 3 dB hysteresis, {opened} openings without (2 s)");
}

// ---------------------------------------------------------------------------------------------
// AC-5 / AC-6 / AC-7: hold, attack shape, release
// ---------------------------------------------------------------------------------------------

#[test]
fn hold_time() {
    let mut report = Vec::new();
    for hold in [0.1, 10.0, 50.0, 500.0] {
        let h = (hold * SR / 1000.0).round() as usize;
        let on = 4_800;
        let e = on - 1;
        let s = [("hold_ms", hold)];
        // DC bursts: exact to ±1 sample.
        let x = dc_segments(&[(-20.0, on), (-60.0, h + 24_000)]);
        let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(1024));
        let start = (e + 1..x.len())
            .find(|&i| y[i] != x[i])
            .expect("no release");
        let want = e + W_PK + h;
        assert!(
            start.abs_diff(want) <= 1,
            "hold {hold}: DC release at {start}, want {want}"
        );
        // 997 Hz bursts: ±1 ms.
        let x = sine_segments(997.0, &[(-20.0, on), (-60.0, h + 24_000)], SR);
        let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(1024));
        let tone = (e + 1..x.len())
            .find(|&i| y[i] != x[i])
            .expect("no release");
        assert!(
            tone.abs_diff(want) <= 48,
            "hold {hold}: tone release at {tone}, want {want}"
        );
        report.push(format!(
            "hold {hold} ms: DC {:+} smp, tone {:+} smp",
            start as i64 - want as i64,
            tone as i64 - want as i64
        ));
    }
    println!("AC-5: {}", report.join("; "));
}

#[test]
fn attack_is_a_raised_cosine_of_exact_length() {
    let f = lin(-30.0);
    for attack in [0.1, 2.0, 20.0, 100.0] {
        let a = (attack * SR / 1000.0).round().max(1.0) as usize;
        let s0 = 2_400;
        let x = dc_segments(&[(-60.0, s0), (-20.0, a + 4_800)]);
        let y = render(
            &mut *gate(&[("attack_ms", attack)]),
            &x,
            &[],
            Blocks::Fixed(1024),
        );
        let mut prev = f;
        let max_step = (1.0 - f) * PI / (2.0 * a as f64);
        for p in 1..=a + 100 {
            let i = s0 + p - 1;
            let g = f64::from(y[i]) / f64::from(x[i]);
            let want = if p < a {
                f + (1.0 - f) * (1.0 - (PI * p as f64 / a as f64).cos()) / 2.0
            } else {
                1.0
            };
            assert!(
                (g - want).abs() <= 1e-6,
                "attack {attack} p {p}: {g} vs {want}"
            );
            if p >= a {
                assert_eq!(
                    y[i].to_bits(),
                    x[i].to_bits(),
                    "attack {attack}: not unity at p {p}"
                );
            }
            assert!(
                g - prev <= max_step * (1.0 + 1e-5) + 1e-6,
                "attack {attack}: step at p {p}"
            );
            prev = g;
        }
    }
}

#[test]
fn release_is_exponential_with_exact_time_constant() {
    let mut report = Vec::new();
    for release in [5.0, 100.0, 1000.0] {
        for range in [-30.0, -100.0] {
            let f = NoiseGate::range_gain(range);
            let tau = release * SR / 1000.0;
            let alpha = (-1.0 / tau).exp();
            let on = 2_400;
            let len = (14.0 * tau) as usize + 9_600;
            let x = dc_segments(&[(-20.0, on), (-60.0, len)]);
            let s = [
                ("hold_ms", 0.1),
                ("release_ms", release),
                ("range_db", range),
            ];
            let y = render(&mut *gate(&s), &x, &[], Blocks::Fixed(4096));
            let start = on - 1 + W_PK + 5;
            assert_eq!(
                y[start - 1].to_bits(),
                x[start - 1].to_bits(),
                "release too early"
            );
            let mut tau_at = None;
            for k in 0..len - W_PK - 10 {
                let i = start + k;
                let g = f64::from(y[i]) / f64::from(x[i]);
                let want = f + (1.0 - f) * alpha.powf((k + 1) as f64);
                // Within 1e-6 until the snap; from the snap on the gain is the floor itself (the
                // snap fires within SNAP_LIN = 1e-6 of it).
                let snapped = (g - f).abs() <= 1e-8 && want - f <= 1.001e-6;
                assert!(
                    snapped || (g - want).abs() <= 1e-6,
                    "release {release} range {range} k {k}: {g} vs {want}"
                );
                if tau_at.is_none() && (1.0 - g) / (1.0 - f) >= TAU_FRACTION {
                    tau_at = Some(k + 1);
                }
            }
            let n = tau_at.expect("63.2 % never reached");
            let tol = (0.02 * tau).max(1.0);
            assert!(
                (n as f64 - tau).abs() <= tol,
                "release {release}: 63.2 % at {n}, τ {tau}"
            );
            if range <= -100.0 {
                let silent_from = start + (14.0 * tau).ceil() as usize + 1;
                assert!(
                    y[silent_from..].iter().all(|&v| v == 0.0),
                    "−∞ not exact silence"
                );
            }
            report.push(format!(
                "release {release} ms / range {range}: 63.2 % at {n} smp (τ {tau})"
            ));
        }
    }
    println!("AC-7: {}", report.join("; "));
}

// ---------------------------------------------------------------------------------------------
// AC-8: range
// ---------------------------------------------------------------------------------------------

#[test]
fn range_attenuation() {
    let x = sine(997.0, -60.0, 48_000, SR);
    let mut worst: f64 = 0.0;
    for range in [-6.0, -30.0, -60.0, -99.9] {
        let y = render(
            &mut *gate(&[("range_db", range)]),
            &x,
            &[],
            Blocks::Fixed(1024),
        );
        let att = to_db(rms(&y) / rms(&x));
        assert!((att - range).abs() <= 0.2, "range {range}: {att:.4} dB");
        worst = worst.max((att - range).abs());
    }
    let y = render(
        &mut *gate(&[("range_db", -100.0)]),
        &x,
        &[],
        Blocks::Fixed(1024),
    );
    assert!(y.iter().all(|&v| v == 0.0), "−∞ must be exact silence");
    println!("AC-8: worst closed-attenuation error {worst:.5} dB");
}

#[test]
fn range_zero_is_a_bit_exact_passthrough() {
    let n = 96_000;
    let keyed = keyed_tone(n);
    let noise = white(0xAC08, 0.02, n);
    let x: Vec<f32> = keyed.iter().zip(&noise).map(|(a, b)| a + b).collect();
    let mut rng = TestRng::new(0xAC08);
    let keys = [
        "threshold_db",
        "hysteresis_db",
        "attack_ms",
        "hold_ms",
        "release_ms",
        "sc_hpf_enabled",
        "sc_hpf_hz",
    ];
    let probe = make();
    for set in 0..20u64 {
        let mut s: Vec<(&str, f64)> = keys
            .iter()
            .map(|&k| {
                let p = probe.params().iter().find(|p| p.key == k).unwrap();
                (k, p.from_normalized(rng.unit_f64()))
            })
            .collect();
        s.push(("range_db", 0.0));
        let y = render(
            &mut *activated(make, &s, SR),
            &x,
            &[],
            Blocks::Random {
                seed: set,
                max: 1024,
            },
        );
        assert_eq!(bit_equal(&y, &x), None, "set {set}: {s:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// AC-9: sidechain HPF
// ---------------------------------------------------------------------------------------------

#[test]
fn sidechain_highpass_rejects_rumble() {
    // A steady rumble, faded in over 100 ms. An abrupt full-level onset at phase 0 is a slope
    // step whose HPF transient is far louder than the steady −51.8 dBFS (measured below).
    let fade = 4_800usize;
    let rumble: Vec<f32> = sine(40.0, -20.0, 96_000, SR)
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if i < fade {
                v * ((1.0 - (PI * i as f64 / fade as f64).cos()) / 2.0) as f32
            } else {
                v
            }
        })
        .collect();
    let abrupt_onset_dbfs = {
        let mut hpf = ButterworthHighpass4::new(100.0, SR);
        let x = sine(40.0, -20.0, 4_800, SR);
        to_db(
            x.iter()
                .map(|&v| hpf.process(f64::from(v)).abs())
                .fold(0.0, f64::max),
        )
    };
    println!(
        "AC-9: abrupt 40 Hz −20 dBFS onset reaches the detector at {abrupt_onset_dbfs:.1} dBFS"
    );
    // HPF on (default, 100 Hz): never opens.
    let (y, lamps) = render_lamp(&mut *activated(make, &[], SR), &rumble, 256);
    assert!(lamps.iter().all(|l| l.2 == 0.0), "rumble opened the gate");
    let f = lin(-30.0) as f32;
    assert!(
        y.iter()
            .zip(&rumble)
            .all(|(a, b)| a.to_bits() == (b * f).to_bits())
    );
    // HPF off: the same rumble opens it.
    let (_, lamps) = render_lamp(&mut *gate(&[]), &rumble, 256);
    assert!(lamps.iter().any(|l| l.2 == 1.0), "HPF off must open");
    // With the gate open, 40 Hz + 997 Hz passes bit-identical (the audio bypasses the filter).
    let voice = sine(997.0, -20.0, 96_000, SR);
    let mix: Vec<f32> = rumble.iter().zip(&voice).map(|(a, b)| a + b).collect();
    let y = render(
        &mut *activated(make, &[], SR),
        &mix,
        &[],
        Blocks::Fixed(1024),
    );
    assert_eq!(bit_equal(&y[2_400..], &mix[2_400..]), None);
    // Detector level = input peak + 4th-order Butterworth magnitude.
    let mut report = Vec::new();
    for freq in [25.0, 40.0, 63.0, 100.0, 1000.0] {
        let n = 48_000;
        let x = sine(freq, -20.0, n, SR);
        let mut m = activated(make, &[], SR);
        let t = telemetry(&*m).unwrap();
        let mut level = f32::NAN;
        render_observed(&mut *m, &x, &[], Blocks::Fixed(n / 2), |s, len| {
            let v = t.read(LEVEL);
            if s + len == n {
                level = v;
            }
        });
        let r4 = (freq / 100.0f64).powi(4);
        let want = -20.0 + to_db(r4 / (1.0 + r4 * r4).sqrt());
        assert!(
            (f64::from(level) - want).abs() <= 0.5,
            "{freq} Hz: {level} vs {want:.2}"
        );
        report.push(format!("{freq} Hz {level:.2} (want {want:.2})"));
    }
    println!("AC-9 sidechain level: {}", report.join(", "));
}

// ---------------------------------------------------------------------------------------------
// AC-13: telemetry
// ---------------------------------------------------------------------------------------------

#[test]
fn telemetry_values_and_initial_state() {
    let mut m = gate(&[]);
    let t = telemetry(&*m).unwrap();
    assert_eq!(read_all(&*t), [0.0, -30.0, -100.0], "after activate");
    let n = 24 * 1024;
    let read_last = |m: &mut dyn Module, x: &[f32]| {
        let t = telemetry(&*m).unwrap();
        let mut snap = Vec::new();
        render_observed(m, x, &[], Blocks::Fixed(1024), |s, len| {
            let v = read_all(&*t);
            if s + len == x.len() {
                snap = v;
            }
        });
        snap
    };
    let open = read_last(&mut *m, &sine(997.0, -20.0, n, SR));
    assert_eq!((open[OPEN], open[GAIN]), (1.0, 0.0));
    assert!((f64::from(open[LEVEL]) + 20.0).abs() <= 0.1, "{open:?}");
    let mut m = gate(&[]);
    let closed = read_last(&mut *m, &sine(997.0, -60.0, n, SR));
    assert_eq!(closed[OPEN], 0.0);
    assert!((f64::from(closed[GAIN]) + 30.0).abs() <= 0.01, "{closed:?}");
    assert!((f64::from(closed[LEVEL]) + 60.0).abs() <= 0.1, "{closed:?}");
    m.reset();
    assert_eq!(
        read_all(&*telemetry(&*m).unwrap()),
        [0.0, -30.0, -100.0],
        "after reset"
    );
    let mut m = gate(&[("range_db", -100.0)]);
    let inf = read_last(&mut *m, &sine(997.0, -60.0, n, SR));
    assert_eq!(inf[GAIN], -100.0);
}

// ---------------------------------------------------------------------------------------------
// AC-12: realtime = offline
// ---------------------------------------------------------------------------------------------

#[test]
fn realtime_equals_offline_bit_identical() {
    let script: [(ParamId, f64); 20] = [
        (NoiseGate::THRESHOLD_DB, -30.0),
        (NoiseGate::HYSTERESIS_DB, 12.0),
        (NoiseGate::ATTACK_MS, 0.1),
        (NoiseGate::HOLD_MS, 5.0),
        (NoiseGate::RELEASE_MS, 20.0),
        (NoiseGate::RANGE_DB, -100.0),
        (NoiseGate::SC_HPF_HZ, 500.0),
        (NoiseGate::SC_HPF_ENABLED, 0.0),
        (NoiseGate::THRESHOLD_DB, -45.0),
        (NoiseGate::ATTACK_MS, 20.0),
        (NoiseGate::HYSTERESIS_DB, 0.0),
        (NoiseGate::RANGE_DB, -12.0),
        (NoiseGate::SC_HPF_ENABLED, 1.0),
        (NoiseGate::HOLD_MS, 300.0),
        (NoiseGate::RELEASE_MS, 1000.0),
        (NoiseGate::SC_HPF_HZ, 40.0),
        (NoiseGate::LOOKAHEAD_MS, 5.0),
        (NoiseGate::THRESHOLD_DB, -20.0),
        (NoiseGate::RANGE_DB, 0.0),
        (NoiseGate::ATTACK_MS, 2.0),
    ];
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        let n = (3.0 * fs) as usize;
        let floor = white(0xAC12, lin(-50.0) as f32, n);
        let tone = sine(997.0, -12.0, n, fs);
        let burst = (0.3 * fs) as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                floor[i]
                    + if i % (fs as usize) < burst {
                        tone[i]
                    } else {
                        0.0
                    }
            })
            .collect();
        let changes: Vec<Change> = script
            .iter()
            .enumerate()
            .map(|(i, &(id, v))| ((i + 1) * n / 21 + 3 * i, id, v))
            .collect();
        let offline = || {
            render(
                &mut *activated_with(make, &[], fs, ProcessMode::Offline, 4096),
                &x,
                &changes,
                Blocks::Fixed(4096),
            )
        };
        let (a, b) = (offline(), offline());
        assert_eq!(fnv1a(&a), fnv1a(&b));
        let rt = render(
            &mut *activated_with(make, &[], fs, ProcessMode::Realtime, 1024),
            &x,
            &changes,
            Blocks::Random {
                seed: 0xAC12 ^ fs as u64,
                max: 1024,
            },
        );
        assert_eq!(bit_equal(&rt, &a), None, "realtime ≠ offline at {fs} Hz");
        println!("AC-12 {fs} Hz: FNV-1a {:016x}", fnv1a(&a));
    }
}

// ---------------------------------------------------------------------------------------------
// AC-11: §4.3 zipper, SPEC-013 §5.1 setups (HPF on at 100 Hz)
// ---------------------------------------------------------------------------------------------

#[test]
fn zipper_threshold() {
    let x = sine(997.0, -40.0, ZIPPER_LEN, SR);
    assert_zipper(make, &[], NoiseGate::THRESHOLD_DB, &x);
}

#[test]
fn zipper_hysteresis() {
    let x = sine_segments(997.0, &[(-30.0, 4_800), (-50.0, ZIPPER_LEN - 4_800)], SR);
    assert_zipper(make, &[("hold_ms", 50.0)], NoiseGate::HYSTERESIS_DB, &x);
}

#[test]
fn zipper_attack_and_release() {
    let x = keyed_tone(ZIPPER_LEN);
    assert_zipper(make, &[("hold_ms", 0.1)], NoiseGate::ATTACK_MS, &x);
    assert_zipper(make, &[("hold_ms", 0.1)], NoiseGate::RELEASE_MS, &x);
}

#[test]
fn zipper_hold() {
    let x = keyed_tone(ZIPPER_LEN);
    assert_zipper(make, &[("release_ms", 5.0)], NoiseGate::HOLD_MS, &x);
}

#[test]
fn zipper_range() {
    let x = sine(997.0, -50.0, ZIPPER_LEN, SR);
    assert_zipper(make, &[], NoiseGate::RANGE_DB, &x);
}

#[test]
fn zipper_sidechain_frequency_keeps_audio_bit_identical() {
    let x = sine(997.0, -20.0, ZIPPER_LEN, SR);
    assert_zipper(make, &[], NoiseGate::SC_HPF_HZ, &x);
    let changes = [(72_013, NoiseGate::SC_HPF_HZ, 632.0)];
    let y = render(
        &mut *activated(make, &[("sc_hpf_hz", 63.0)], SR),
        &x,
        &changes,
        Blocks::Fixed(1024),
    );
    assert_eq!(
        bit_equal(&y[W_PK..], &x[W_PK..]),
        None,
        "gate must stay open"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-14: `TransferCurve` (SPEC-013 §4.3, SPEC-016 §4.11)
// ---------------------------------------------------------------------------------------------

const AC14_MIN_DBFS: f64 = -80.0;
const AC14_MAX_DBFS: f64 = 6.0;
const AC14_TOLERANCE_DB: f64 = 0.1;
/// Settling per level, then the window the output peak is read over.
const AC14_SETTLE_MS: f64 = 150.0;
const AC14_WINDOW_MS: f64 = 40.0;
/// Seeded random parameter sets.
const AC14_SETS: usize = 20;
/// The curve describes a 997 Hz tone, so only corners the tone passes are drawn (§4.3).
const AC14_MAX_HPF_HZ: f64 = 460.0;

fn ac14_levels() -> Vec<f64> {
    let n = (AC14_MAX_DBFS - AC14_MIN_DBFS) as usize;
    (0..=n).map(|i| AC14_MIN_DBFS + i as f64).collect()
}

/// Plain target values in `params()` order, as the rack mirror would hold them.
fn values_of(m: &dyn Module) -> Vec<f64> {
    m.params()
        .iter()
        .map(|p| m.param_value(p.id).unwrap_or(p.default))
        .collect()
}

/// One seeded parameter set. Threshold, hysteresis and range span their full schema range
/// (range hits −∞ about a fifth of the time); the timing values are drawn short, because §4.3 is
/// a settled curve that no timing value can change and 87 levels have to settle per set.
fn ac14_settings(rng: &mut TestRng) -> Vec<(&'static str, f64)> {
    fn r(rng: &mut TestRng, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * rng.unit_f64()
    }
    macro_rules! r {
        ($lo:expr, $hi:expr) => {
            r(rng, $lo, $hi)
        };
    }
    let hpf_on = rng.chance(0.5);
    vec![
        ("threshold_db", r!(-80.0, 0.0)),
        ("hysteresis_db", r!(0.0, 20.0)),
        ("attack_ms", r!(0.5, 5.0)),
        ("hold_ms", r!(1.0, 10.0)),
        ("release_ms", r!(2.0, 6.0)),
        (
            "range_db",
            if rng.chance(0.2) {
                NoiseGate::RANGE_MIN_DB
            } else {
                r!(-60.0, 0.0)
            },
        ),
        ("sc_hpf_enabled", f64::from(u8::from(hpf_on))),
        ("sc_hpf_hz", r!(20.0, AC14_MAX_HPF_HZ)),
        ("lookahead_ms", 0.0),
    ]
}

/// The settled output peak (dBFS, `-inf` for digital silence) at each level, approached in the
/// order given.
fn ac14_measure(settings: &[(&str, f64)], levels: &[f64]) -> Vec<f64> {
    let settle = (AC14_SETTLE_MS * SR / 1000.0) as usize;
    let window = (AC14_WINDOW_MS * SR / 1000.0) as usize;
    let step = settle + window;
    let segs: Vec<(f64, usize)> = levels.iter().map(|&l| (l, step)).collect();
    let x = sine_segments(997.0, &segs, SR);
    let mut m = activated(make, settings, SR);
    let y = render(&mut *m, &x, &[], Blocks::Fixed(4096));
    (0..levels.len())
        .map(|i| {
            let end = (i + 1) * step;
            let p = peak(&y[end - window..end]);
            if p > 0.0 { to_db(p) } else { f64::NEG_INFINITY }
        })
        .collect()
}

#[test]
fn ac14_transfer_curve_equals_the_measured_output() {
    let rising = ac14_levels();
    let falling: Vec<f64> = rising.iter().rev().copied().collect();
    let probe = make();
    let curve = vox_module_api::transfer_curve(&*probe).expect("Noise Gate answers TransferCurve");
    let mut rng = TestRng::new(0x_AC14_0001);

    for set in 0..AC14_SETS {
        let settings = ac14_settings(&mut rng);
        let m = new_with(make, &settings);
        let values = values_of(&*m);
        let value = |key: &str| {
            settings
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| *v)
                .expect("key")
        };
        assert_eq!(
            no_alloc(|| curve.has_hysteresis(&values)).expect("has_hysteresis allocated"),
            value("hysteresis_db") > 0.0,
            "set {set}: has_hysteresis follows hysteresis_db"
        );

        let mut branches = Vec::new();
        for (branch, levels) in [
            (CurveBranch::Rising, &rising),
            (CurveBranch::Falling, &falling),
        ] {
            let mut want = vec![0.0f64; levels.len()];
            no_alloc(|| curve.output_dbfs(&values, branch, levels, &mut want))
                .expect("output_dbfs allocated");
            let got = ac14_measure(&settings, levels);
            for (i, (&w, &g)) in want.iter().zip(&got).enumerate() {
                assert!(!w.is_nan(), "set {set} {branch:?} {i}: curve is NaN");
                if g == f64::NEG_INFINITY || w == f64::NEG_INFINITY {
                    assert_eq!(
                        w, g,
                        "set {set} {branch:?} at {} dBFS: −inf must line up with digital silence \
                         ({settings:?})",
                        levels[i]
                    );
                    continue;
                }
                assert!(
                    (w - g).abs() <= AC14_TOLERANCE_DB,
                    "set {set} {branch:?} at {} dBFS: curve {w:.4} vs measured {g:.4} dBFS \
                     ({settings:?})",
                    levels[i]
                );
            }
            branches.push(want);
        }

        // The branches differ only in [T − hysteresis, T).
        let (open_at, close_at) = (
            value("threshold_db"),
            value("threshold_db") - value("hysteresis_db"),
        );
        let mut falling_rising_order = branches[1].clone();
        falling_rising_order.reverse();
        for (i, (&r, &f)) in branches[0].iter().zip(&falling_rising_order).enumerate() {
            let x = rising[i];
            let inside = (close_at..open_at).contains(&x);
            let same = r == f;
            assert!(
                same != (inside && value("range_db") < 0.0),
                "set {set} at {x} dBFS (open {open_at}, close {close_at}): rising {r}, falling {f}"
            );
        }

        // One component, and it carries the whole gain.
        assert_eq!(curve.component_count(), NoiseGate::CURVE_COMPONENTS);
        let mut gains = vec![0.0f64; rising.len()];
        no_alloc(|| {
            curve.component_gain_db(NoiseGate::COMPONENT_GATE, &values, &rising, &mut gains)
        })
        .expect("component_gain_db allocated");
        for (i, (&t, &g)) in branches[0].iter().zip(&gains).enumerate() {
            let want = t - rising[i];
            if want == f64::NEG_INFINITY {
                assert_eq!(g, want, "set {set} {i}: the component must mute too");
            } else {
                assert!((g - want).abs() < 1e-9, "set {set} {i}: {g} vs {want}");
            }
        }
        assert_eq!(curve.component_group(NoiseGate::COMPONENT_GATE), None);
    }
}

#[test]
fn ac14_handle_matches_the_spec() {
    let probe = make();
    let curve = vox_module_api::transfer_curve(&*probe).expect("Noise Gate answers TransferCurve");
    let handles = no_alloc(|| curve.handles()).expect("handles allocated");
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].component, NoiseGate::COMPONENT_GATE);
    assert_eq!(handles[0].threshold, NoiseGate::THRESHOLD_DB);
    assert_eq!(handles[0].enable, None, "the gate is always on");
    let values = values_of(&*make());
    assert_eq!(
        no_alloc(|| curve.handle_offset_db(0, &values)).expect("offset allocated"),
        0.0,
        "the gate detector is peak, so the handle sits on the threshold"
    );
}
