//! Parametric EQ (S3-03, SPEC-015 lean-slice subset): AC-1 (schema, `ModuleTestHost` at
//! 44.1/48/96 kHz), AC-2 (bit-exact neutral / disabled bands), AC-3 (impulse-response DTFT vs an
//! independent cookbook implementation, ±0.1 dB, 44.1/48/96 kHz), AC-4 (`ResponseCurve`), AC-5
//! (Butterworth −3 dB and crossing on `process()` output), AC-7 (event timing), AC-8 (SPEC-012
//! §4.3 normative rows at 48 kHz, offline) and AC-10 (realtime = offline).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
#![allow(clippy::approx_constant)] // SPEC-015's shelf Q default is the plain value 0.7071

mod common;

use std::f64::consts::{PI, TAU};
use std::time::Instant;

use common::{Blocks, Change, activated, activated_with, bit_equal, fnv1a, new_with, render};
use vox_dsp::eq::coeffs::{GainShape, PassKind, butterworth};
use vox_module_api::test_util::{
    ModuleTestHost, TestRng, ZIPPER_CHANGE_AT, ZIPPER_DRAG_EVENTS, ZIPPER_DRAG_INTERVAL_MS,
    ZIPPER_SAMPLE_RATE, ZipperReport, ZipperWindow, analyze_zipper, no_alloc, zipper_signal,
};
use vox_module_api::{
    CurveHandle, Module, ModuleFactory, ParamFlags, ParamId, ParamInfo, ProcessMode, Tail, Taper,
    Unit, response_curve, validate_schema,
};
use vox_modules::{ParametricEq, ParametricEqFactory, builtin_factories};

vox_module_api::install_test_allocator!();

const RATES: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];
const KEYS: [&str; 9] = ParametricEq::BAND_KEYS;

fn make() -> Box<dyn Module> {
    Box::new(ParametricEq::new())
}

fn eq_param(key: &str) -> ParamInfo {
    ParametricEq::new()
        .params()
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("no parameter {key}"))
        .clone()
}

fn is_gain_band(k: usize) -> bool {
    (1..=7).contains(&k)
}

// ---------------------------------------------------------------------------------------------
// Configurations
// ---------------------------------------------------------------------------------------------

/// A static EQ configuration (band k: 0 HP, 1 L, 2–6 peaks 1–5, 7 H, 8 LP).
#[derive(Clone, Debug)]
struct Cfg {
    on: [bool; 9],
    freq: [f64; 9],
    gain: [f64; 9],
    q: [f64; 9],
    /// Butterworth orders of HP and LP (1…8).
    order: [usize; 2],
    master_db: f64,
}

fn log_uniform(rng: &mut TestRng, lo: f64, hi: f64) -> f64 {
    lo * (hi / lo).powf(rng.unit_f64())
}

impl Cfg {
    /// SPEC-015 §2.2 defaults.
    fn defaults() -> Self {
        Self {
            on: [false, true, true, true, true, true, true, true, false],
            freq: [
                80.0, 100.0, 200.0, 500.0, 1200.0, 3000.0, 6000.0, 10_000.0, 12_000.0,
            ],
            gain: [0.0; 9],
            q: [1.0, 0.7071, 1.0, 1.0, 1.0, 1.0, 1.0, 0.7071, 1.0],
            order: [4, 4],
            master_db: 0.0,
        }
    }

    /// Everything off except what the caller turns on.
    fn all_off() -> Self {
        Self {
            on: [false; 9],
            ..Self::defaults()
        }
    }

    fn random(rng: &mut TestRng) -> Self {
        let mut c = Self::defaults();
        for k in 0..9 {
            c.on[k] = rng.chance(0.5);
            c.freq[k] = log_uniform(rng, 20.0, 20_000.0);
            if is_gain_band(k) {
                c.gain[k] = -24.0 + 48.0 * rng.unit_f64();
                c.q[k] = if k == 1 || k == 7 {
                    log_uniform(rng, 0.3, 2.0)
                } else {
                    log_uniform(rng, 0.1, 30.0)
                };
            }
        }
        for o in &mut c.order {
            *o = 1 + ((rng.unit_f64() * 8.0) as usize).min(7);
        }
        c.master_db = -6.0 + 12.0 * rng.unit_f64();
        c
    }

    fn settings(&self) -> Vec<(String, f64)> {
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        let mut s = vec![("master_gain_db".to_owned(), self.master_db)];
        for (k, key) in KEYS.iter().enumerate() {
            s.push((format!("{key}_on"), b(self.on[k])));
            s.push((format!("{key}_freq_hz"), self.freq[k]));
            if is_gain_band(k) {
                s.push((format!("{key}_gain_db"), self.gain[k]));
                s.push((format!("{key}_q"), self.q[k]));
            } else {
                let o = self.order[usize::from(k == 8)];
                s.push((format!("{key}_slope"), (o - 1) as f64));
            }
        }
        s
    }

    /// Plain values in `params()` order, as the module stores them.
    fn values(&self) -> Vec<f64> {
        let m = new_with(make, &borrow(&self.settings()));
        m.params()
            .iter()
            .map(|p| m.param_value(p.id).unwrap())
            .collect()
    }
}

fn borrow(s: &[(String, f64)]) -> Vec<(&str, f64)> {
    s.iter().map(|(k, v)| (k.as_str(), *v)).collect()
}

fn corner_cases() -> Vec<Cfg> {
    let mut a = Cfg::all_off();
    for (k, f, g, q) in [
        (2, 20.0, 24.0, 30.0),
        (6, 20_000.0, -24.0, 30.0),
        (4, 1_000.0, 24.0, 0.1),
    ] {
        (a.on[k], a.freq[k], a.gain[k], a.q[k]) = (true, f, g, q);
    }
    a.master_db = -24.0;

    let mut b = Cfg::all_off();
    b.on = [true, true, false, true, false, false, false, true, true];
    (b.freq[0], b.freq[8], b.order) = (20.0, 20_000.0, [8, 8]);
    (b.freq[1], b.gain[1], b.q[1]) = (20.0, 24.0, 2.0);
    (b.freq[7], b.gain[7], b.q[7]) = (20_000.0, -24.0, 0.3);
    (b.freq[3], b.gain[3], b.q[3]) = (200.0, -24.0, 0.1);

    let mut c = Cfg::all_off();
    c.on = [true, false, false, false, true, true, false, false, true];
    (c.freq[0], c.freq[8], c.order) = (1_000.0, 2_000.0, [1, 7]);
    (c.freq[4], c.gain[4], c.q[4]) = (1_414.0, -24.0, 30.0);
    (c.freq[5], c.gain[5], c.q[5]) = (500.0, 12.0, 4.0);

    let mut d = Cfg::defaults();
    d.on[0] = true;
    d.on[8] = true;
    d.order = [3, 5];
    for (k, g, q) in [
        (2, -24.0, 30.0),
        (3, 24.0, 0.1),
        (4, -12.0, 2.0),
        (5, 6.0, 0.5),
        (6, -3.0, 8.0),
    ] {
        (d.gain[k], d.q[k]) = (g, q);
    }
    (d.gain[1], d.q[1], d.gain[7], d.q[7]) = (-24.0, 0.3, 24.0, 2.0);

    let mut e = Cfg::all_off();
    e.on = [true, false, false, false, false, false, false, false, true];
    (e.freq[0], e.freq[8], e.order) = (30.0, 8_000.0, [2, 6]);
    e.master_db = 24.0;

    vec![a, b, c, d, e]
}

// ---------------------------------------------------------------------------------------------
// Independent reference (W3C cookbook formulas, complex evaluation; closed-form Butterworth)
// ---------------------------------------------------------------------------------------------

/// |B(e^jω)| / |A(e^jω)| with un-normalized coefficients.
fn poly_ratio(b: [f64; 3], a: [f64; 3], w: f64) -> f64 {
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let (nr, ni) = (b[0] + b[1] * c1 + b[2] * c2, -(b[1] * s1 + b[2] * s2));
    let (dr, di) = (a[0] + a[1] * c1 + a[2] * c2, -(a[1] * s1 + a[2] * s2));
    ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
}

/// Band k of `cfg` at `f` in dB (whether on or off).
fn ref_band_db(cfg: &Cfg, k: usize, fs: f64, f: f64) -> f64 {
    let f0 = cfg.freq[k].min(0.49 * fs);
    if !is_gain_band(k) {
        let n = cfg.order[usize::from(k == 8)] as i32;
        let t = (PI * f / fs).tan() / (PI * f0 / fs).tan();
        let r = if k == 0 { 1.0 / t } else { t };
        return -10.0 * (1.0 + r.powi(2 * n)).log10();
    }
    if cfg.gain[k] == 0.0 {
        return 0.0;
    }
    let a = 10f64.powf(cfg.gain[k] / 40.0);
    let w0 = TAU * f0 / fs;
    let (c, s) = (w0.cos(), w0.sin());
    let alpha = s / (2.0 * cfg.q[k]);
    let sa = 2.0 * a.sqrt() * alpha;
    let (b, den) = match k {
        1 => (
            [
                a * ((a + 1.0) - (a - 1.0) * c + sa),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - sa),
            ],
            [
                (a + 1.0) + (a - 1.0) * c + sa,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - sa,
            ],
        ),
        7 => (
            [
                a * ((a + 1.0) + (a - 1.0) * c + sa),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - sa),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + sa,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - sa,
            ],
        ),
        _ => (
            [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a],
        ),
    };
    20.0 * poly_ratio(b, den, TAU * f / fs).log10()
}

fn ref_total_db(cfg: &Cfg, fs: f64, f: f64) -> f64 {
    cfg.master_db
        + (0..9)
            .filter(|&k| cfg.on[k])
            .map(|k| ref_band_db(cfg, k, fs, f))
            .sum::<f64>()
}

/// f_k = 20·2^(k/12) Hz with f_k ≤ 20 kHz and f_k < fs/2.
fn twelfth_octave_points(fs: f64) -> Vec<f64> {
    (0..)
        .map(|k| 20.0 * 2f64.powf(f64::from(k) / 12.0))
        .take_while(|&f| f <= 20_000.0 + 1e-9 && f < fs / 2.0)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Measurement helpers
// ---------------------------------------------------------------------------------------------

/// Slowest pole radius of the enabled, non-neutral bands (for the impulse-response length).
fn slowest_pole(cfg: &Cfg, fs: f64) -> f64 {
    (0..9)
        .filter(|&k| cfg.on[k])
        .map(|k| match k {
            0 => butterworth(PassKind::HighPass, cfg.order[0], cfg.freq[0], fs).pole_radius(),
            8 => butterworth(PassKind::LowPass, cfg.order[1], cfg.freq[8], fs).pole_radius(),
            _ if cfg.gain[k] == 0.0 => 0.0,
            _ => {
                let shape = match k {
                    1 => GainShape::LowShelf,
                    7 => GainShape::HighShelf,
                    _ => GainShape::Peak,
                };
                shape
                    .coeffs(cfg.freq[k], cfg.q[k], cfg.gain[k], fs)
                    .pole_radius()
            }
        })
        .fold(0.0, f64::max)
}

/// Samples until the remainder bound r^L / (1 − r) of the slowest pole is < 1e-9.
fn ir_len(r: f64) -> usize {
    if r <= 1e-3 {
        return 4096;
    }
    let n = (1e-9 * (1.0 - r)).ln() / r.ln();
    n.ceil() as usize + 4096
}

/// `process()` impulse response (offline, 4096-frame blocks).
fn impulse_response(cfg: &Cfg, fs: f64, len: usize) -> Vec<f32> {
    let mut m = activated(make, &borrow(&cfg.settings()), fs);
    let mut x = vec![0.0f32; len];
    x[0] = 1.0;
    render(&mut *m, &x, &[], Blocks::Fixed(4096))
}

/// |DTFT(h)| in dB at `freqs` (phasor re-anchored every 4096 samples).
fn dtft_db(h: &[f32], fs: f64, freqs: &[f64]) -> Vec<f64> {
    freqs
        .iter()
        .map(|&f| {
            let w = TAU * f / fs;
            let (dc, ds) = (w.cos(), w.sin());
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (ci, chunk) in h.chunks(4096).enumerate() {
                let p = w * (ci * 4096) as f64;
                let (mut c, mut s) = (p.cos(), p.sin());
                for &v in chunk {
                    let v = f64::from(v);
                    re += v * c;
                    im -= v * s;
                    (c, s) = (c * dc - s * ds, s * dc + c * ds);
                }
            }
            10.0 * (re * re + im * im).log10()
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// AC-1
// ---------------------------------------------------------------------------------------------

#[test]
fn ac1_descriptor_schema_and_groups() {
    let f = ParametricEqFactory::new();
    let d = f.descriptor();
    assert_eq!(d.id, "org.powervoice.parametric-eq");
    assert_eq!(d.version.to_string(), "1.0.0");
    assert_eq!(d.state_format_version, 1);
    assert_eq!(d.features, ["equalizer", "mono"]);
    assert_eq!(d.name.key.as_deref(), Some("module.parametric_eq.name"));
    let m = f.create().unwrap();

    // (id, key, unit, min, max, default, taper, step, decimals, group, labels)
    type Row = (
        u32,
        String,
        Unit,
        f64,
        f64,
        f64,
        Taper,
        Option<f64>,
        u8,
        Option<u32>,
        usize,
    );
    let db = Taper::Db {
        neg_inf_at_min: false,
    };
    let mut want: Vec<Row> = vec![(
        0,
        "master_gain_db".into(),
        Unit::Db,
        -24.0,
        24.0,
        0.0,
        db,
        None,
        1,
        None,
        0,
    )];
    let freqs = [
        80.0, 100.0, 200.0, 500.0, 1200.0, 3000.0, 6000.0, 10_000.0, 12_000.0,
    ];
    for (k, key) in KEYS.iter().enumerate() {
        let (base, g) = (10 * (k as u32 + 1), Some(k as u32 + 1));
        let on_default = if is_gain_band(k) { 1.0 } else { 0.0 };
        want.push((
            base,
            format!("{key}_on"),
            Unit::None,
            0.0,
            1.0,
            on_default,
            Taper::Linear,
            Some(1.0),
            0,
            g,
            0,
        ));
        want.push((
            base + 1,
            format!("{key}_freq_hz"),
            Unit::Hz,
            20.0,
            20_000.0,
            freqs[k],
            Taper::Log,
            None,
            0,
            g,
            0,
        ));
        if is_gain_band(k) {
            let (qmin, qmax, qdef) = if k == 1 || k == 7 {
                (0.3, 2.0, 0.7071)
            } else {
                (0.1, 30.0, 1.0)
            };
            want.push((
                base + 2,
                format!("{key}_gain_db"),
                Unit::Db,
                -24.0,
                24.0,
                0.0,
                db,
                None,
                1,
                g,
                0,
            ));
            want.push((
                base + 3,
                format!("{key}_q"),
                Unit::None,
                qmin,
                qmax,
                qdef,
                Taper::Log,
                None,
                2,
                g,
                0,
            ));
        } else {
            want.push((
                base + 4,
                format!("{key}_slope"),
                Unit::None,
                0.0,
                7.0,
                3.0,
                Taper::Linear,
                Some(1.0),
                0,
                g,
                8,
            ));
        }
    }
    let params = m.params();
    assert_eq!(params.len(), 35);
    assert_eq!(params.len(), want.len());
    for (p, w) in params.iter().zip(&want) {
        let got: Row = (
            p.id.0,
            p.key.clone(),
            p.unit.clone(),
            p.min,
            p.max,
            p.default,
            p.taper,
            p.step,
            p.decimals,
            p.group.map(|g| g.0),
            p.enum_labels.len(),
        );
        assert_eq!(&got, w);
        assert_eq!(p.smoothing_ms, 20.0, "{}", p.key);
        assert!(p.flags.contains(ParamFlags::AUTOMATABLE), "{}", p.key);
        assert!(!p.flags.contains(ParamFlags::HIDDEN), "{}", p.key);
        let is_on = p.key.ends_with("_on");
        assert_eq!(p.flags.contains(ParamFlags::BOOL), is_on, "{}", p.key);
        assert_eq!(
            p.is_stepped(),
            is_on || p.key.ends_with("_slope"),
            "{}",
            p.key
        );
        assert_eq!(
            p.name.key.as_deref(),
            Some(format!("module.parametric_eq.param.{}", p.key).as_str())
        );
    }
    let slope = &params[3];
    assert_eq!(slope.enum_labels[0].text, "6 dB/oct");
    assert_eq!(slope.enum_labels[7].text, "48 dB/oct");

    let groups = m.groups();
    let keys = [
        "hp",
        "low_shelf",
        "band_1",
        "band_2",
        "band_3",
        "band_4",
        "band_5",
        "high_shelf",
        "lp",
    ];
    assert_eq!(groups.len(), 9);
    for (i, g) in groups.iter().enumerate() {
        assert_eq!(g.id.0, i as u32 + 1);
        assert_eq!(g.key, keys[i]);
        assert_eq!(g.enable_param, Some(ParamId(10 * (i as u32 + 1))));
        assert!(g.collapsed_by_default);
        assert_eq!(g.parent, None);
    }
    validate_schema(params, groups).unwrap();

    let eq = builtin_factories()
        .into_iter()
        .find(|f| f.descriptor().id == ParametricEq::ID)
        .expect("the EQ is a built-in");
    ModuleTestHost::from_factory(eq)
        .blocks(16)
        .check_schema()
        .unwrap();
}

#[test]
fn ac1_module_test_host_at_every_rate() {
    let mut host = ModuleTestHost::new(make);
    for p in ParametricEq::new().params() {
        if p.id != ParametricEq::MASTER_GAIN_DB {
            host = host.allow_delayed_effect(p.id);
        }
    }
    let report = host.assert_passes();
    assert!(report.alloc_checked);
    assert!(report.zero_length_blocks > 0);
    let master: Vec<_> = report
        .first_effect
        .iter()
        .filter(|p| p.key == "master_gain_db")
        .collect();
    assert!(!master.is_empty());
    assert!(
        master.iter().all(|p| p.first_difference == Some(p.offset)),
        "{master:?}"
    );
}

#[test]
fn latency_and_worst_case_tail() {
    for fs in RATES {
        let m = activated(make, &[], fs);
        assert_eq!(m.latency_samples(), 0);
        let Tail::Samples(n) = m.tail() else {
            panic!("finite tail expected");
        };
        let s = n as f64 / fs;
        println!("tail at {fs} Hz: {n} samples = {s:.2} s");
        assert!((34.4..35.0).contains(&s), "{s} s");
        assert_eq!(n, ParametricEq::worst_case_tail_samples(fs));
    }
}

// ---------------------------------------------------------------------------------------------
// AC-2
// ---------------------------------------------------------------------------------------------

/// Silence, sines, noise, impulses, DC and ±4 squares.
fn varied_signal(fs: f64, each: usize) -> Vec<f32> {
    let mut x = vec![0.0f32; each];
    for (f, a) in [(997.0, 0.5), (50.0, 1.0), (15_000.0, 0.25)] {
        x.extend((0..each).map(|i| (a * (TAU * f * i as f64 / fs).sin()) as f32));
    }
    x.extend(common::white(0xE0, 1.0, each));
    x.extend((0..each).map(|i| if i % 500 == 0 { 1.0 } else { 0.0 }));
    x.extend(std::iter::repeat_n(0.25f32, each));
    x.extend((0..each).map(|i| if (i / 50) % 2 == 0 { 4.0 } else { -4.0 }));
    x.extend(std::iter::repeat_n(1e-40f32, each));
    x
}

#[test]
fn ac2_default_state_is_bit_exact() {
    for fs in RATES {
        let x = varied_signal(fs, 3000);
        for (mode, max_block, blocks) in [
            (ProcessMode::Offline, 4096, Blocks::Fixed(4096)),
            (ProcessMode::Offline, 4096, Blocks::Fixed(1)),
            (ProcessMode::Realtime, 1024, Blocks::Fixed(64)),
            (
                ProcessMode::Realtime,
                1024,
                Blocks::Random { seed: 7, max: 1024 },
            ),
        ] {
            let mut m = activated_with(make, &[], fs, mode, max_block);
            let y = render(&mut *m, &x, &[], blocks);
            assert_eq!(bit_equal(&x, &y), None, "{fs} Hz {blocks:?}");
        }
    }
}

#[test]
fn ac2_zero_db_bands_are_identity() {
    let mut rng = TestRng::new(0x0DB);
    for fs in RATES {
        let mut cfg = Cfg::random(&mut rng);
        cfg.on = [false, true, true, true, true, true, true, true, false];
        cfg.gain = [0.0; 9];
        cfg.master_db = 0.0;
        let x = varied_signal(fs, 2000);
        let mut m = activated(make, &borrow(&cfg.settings()), fs);
        let y = render(&mut *m, &x, &[], Blocks::Fixed(333));
        assert_eq!(bit_equal(&x, &y), None, "{fs} Hz {cfg:?}");
    }
}

#[test]
fn ac2_disabled_band_is_independent_of_its_settings() {
    let mut rng = TestRng::new(0xD15AB1E);
    let x = common::white(0xAB, 0.5, 24_000);
    for k in 0..9 {
        let mut base = Cfg::random(&mut rng);
        base.on[k] = false;
        let mut other = base.clone();
        let donor = Cfg::random(&mut rng);
        other.freq[k] = donor.freq[k];
        other.gain[k] = donor.gain[k];
        other.q[k] = donor.q[k];
        other.order = donor.order;
        other.order[usize::from(k != 8)] = base.order[usize::from(k != 8)];
        if k != 0 && k != 8 {
            other.order = base.order;
        }
        let run = |c: &Cfg| {
            let mut m = activated(make, &borrow(&c.settings()), 48_000.0);
            render(&mut *m, &x, &[], Blocks::Fixed(500))
        };
        let (a, b) = (run(&base), run(&other));
        assert_eq!(bit_equal(&a, &b), None, "band {k}: {base:?} vs {other:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// AC-3
// ---------------------------------------------------------------------------------------------

/// Returns the worst errors (≥ −60 dB, −80…−60 dB).
fn check_impulse_response(cfg: &Cfg, fs: f64, label: &str) -> (f64, f64) {
    let r = slowest_pole(cfg, fs);
    let len = ir_len(r);
    assert!(
        len < 12_000_000,
        "{label}: impulse response too long ({len})"
    );
    let t = Instant::now();
    let h = impulse_response(cfg, fs, len);
    let pts = twelfth_octave_points(fs);
    let got = dtft_db(&h, fs, &pts);
    let (mut worst, mut worst_low) = (0.0f64, 0.0f64);
    for (&f, &g) in pts.iter().zip(&got) {
        let want = ref_total_db(cfg, fs, f);
        let err = (g - want).abs();
        if want >= -60.0 {
            assert!(
                err <= 0.1,
                "{label}: {f:.1} Hz: {g:.4} dB vs {want:.4} dB\n{cfg:?}"
            );
            worst = worst.max(err);
        } else if want >= -80.0 {
            assert!(
                err <= 1.0,
                "{label}: {f:.1} Hz: {g:.4} dB vs {want:.4} dB\n{cfg:?}"
            );
            worst_low = worst_low.max(err);
        }
    }
    println!(
        "{label}: IR {len} samples ({:.2} s wall), worst {worst:.5} dB (≥ −60), {worst_low:.4} dB (−80…−60)",
        t.elapsed().as_secs_f64()
    );
    (worst, worst_low)
}

fn ac3_at(fs: f64, seed: u64, random_configs: usize) {
    let mut rng = TestRng::new(seed);
    let mut worst = (0.0f64, 0.0f64);
    let configs: Vec<(String, Cfg)> = corner_cases()
        .into_iter()
        .enumerate()
        .map(|(i, c)| (format!("corner {i}"), c))
        .chain((0..random_configs).map(|i| (format!("random {i}"), Cfg::random(&mut rng))))
        .collect();
    for (label, cfg) in &configs {
        let (a, b) = check_impulse_response(cfg, fs, &format!("{fs} Hz {label}"));
        worst = (worst.0.max(a), worst.1.max(b));
    }
    println!(
        "AC-3 {fs} Hz: {} configs, worst {:.5} dB (≥ −60 dB), {:.4} dB (−80…−60 dB)",
        configs.len(),
        worst.0,
        worst.1
    );
}

#[test]
fn ac3_response_matches_cookbook_44k1() {
    ac3_at(44_100.0, 0xAC3_441, 12);
}

#[test]
fn ac3_response_matches_cookbook_48k() {
    ac3_at(48_000.0, 0xAC3_480, 24);
}

#[test]
fn ac3_response_matches_cookbook_96k() {
    ac3_at(96_000.0, 0xAC3_960, 12);
}

// ---------------------------------------------------------------------------------------------
// AC-4
// ---------------------------------------------------------------------------------------------

#[test]
fn ac4_response_curve_is_exact_and_consistent() {
    let m = activated(make, &[], 48_000.0);
    let curve = response_curve(&*m).expect("ResponseCurve extension");
    let mut rng = TestRng::new(0xAC4);
    let (mut worst, mut worst_sum) = (0.0f64, 0.0f64);
    for fs in RATES {
        let pts = twelfth_octave_points(fs);
        let mut total = vec![0.0; pts.len()];
        let mut comp = vec![0.0; pts.len()];
        let configs: Vec<Cfg> = corner_cases()
            .into_iter()
            .chain((0..200).map(|_| Cfg::random(&mut rng)))
            .collect();
        for cfg in &configs {
            let v = cfg.values();
            curve.magnitude_db(&v, fs, &pts, &mut total);
            let mut sum = vec![cfg.master_db; pts.len()];
            assert_eq!(curve.component_count(&v), 9);
            for k in 0..9 {
                curve.component_magnitude_db(k, &v, fs, &pts, &mut comp);
                for (i, &f) in pts.iter().enumerate() {
                    let want = ref_band_db(cfg, k, fs, f);
                    if want >= -200.0 {
                        let e = (comp[i] - want).abs();
                        assert!(e <= 0.001, "band {k} {f} Hz @ {fs}: {} vs {want}", comp[i]);
                    }
                    if cfg.on[k] {
                        sum[i] += comp[i];
                    }
                }
            }
            for (i, &f) in pts.iter().enumerate() {
                let want = ref_total_db(cfg, fs, f);
                if want >= -200.0 {
                    let e = (total[i] - want).abs();
                    assert!(e <= 0.001, "{f} Hz @ {fs}: {} vs {want}\n{cfg:?}", total[i]);
                    worst = worst.max(e);
                }
                worst_sum = worst_sum.max((sum[i] - total[i]).abs());
            }
        }
    }
    println!("AC-4: worst |curve − analytic| {worst:.2e} dB, components sum {worst_sum:.2e} dB");
    assert!(worst_sum <= 0.001);
}

#[test]
fn ac4_curve_is_pure_allocation_free_and_thread_safe() {
    let m = activated(make, &[], 48_000.0);
    let curve = response_curve(&*m).unwrap();
    let cfg = &corner_cases()[3];
    let v = cfg.values();
    let freqs: Vec<f64> = (0..2048)
        .map(|i| 20.0 * 1000f64.powf((f64::from(i) + 0.5) / 2048.0))
        .collect();
    let mut a = vec![0.0; freqs.len()];
    let t = Instant::now();
    no_alloc(|| {
        curve.magnitude_db(&v, 48_000.0, &freqs, &mut a);
        for k in 0..9 {
            let mut c = [0.0; 4];
            curve.component_magnitude_db(k, &v, 48_000.0, &freqs[..4], &mut c);
        }
    })
    .expect("magnitude_db allocated");
    println!(
        "2048-point curve: {:.3} ms (debug build)",
        t.elapsed().as_secs_f64() * 1e3
    );
    let mut b = vec![0.0; freqs.len()];
    curve.magnitude_db(&v, 48_000.0, &freqs, &mut b);
    assert_eq!(bit_equal_f64(&a, &b), None);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    let mut out = vec![0.0; freqs.len()];
                    curve.magnitude_db(&v, 48_000.0, &freqs, &mut out);
                    out
                })
            })
            .collect();
        for h in handles {
            assert_eq!(bit_equal_f64(&a, &h.join().unwrap()), None);
        }
    });

    // Handles (§3).
    let want: Vec<CurveHandle> = (0..9)
        .map(|b| {
            let base = 10 * (b as u32 + 1);
            let gain_band = is_gain_band(b);
            CurveHandle {
                component: b,
                freq: ParamId(base + 1),
                gain: gain_band.then_some(ParamId(base + 2)),
                q: gain_band.then_some(ParamId(base + 3)),
                enable: Some(ParamId(base)),
            }
        })
        .collect();
    assert_eq!(curve.handles(), want.as_slice());
}

fn bit_equal_f64(a: &[f64], b: &[f64]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

/// AC-4 / AC-13: at 22.05 kHz a band at 20 kHz is exactly a band at 0.49·fs, in the curve and
/// in `process()`.
#[test]
fn ac4_frequencies_clamp_at_049_fs() {
    let fs = 22_050.0;
    let top = 0.49 * fs;
    let mut hi = Cfg::all_off();
    hi.on = [true, false, false, false, true, false, false, true, true];
    (hi.freq[0], hi.freq[8], hi.order) = (20_000.0, 20_000.0, [3, 8]);
    (hi.freq[4], hi.gain[4], hi.q[4]) = (20_000.0, 9.0, 3.0);
    (hi.freq[7], hi.gain[7], hi.q[7]) = (20_000.0, -6.0, 1.0);
    let mut at = hi.clone();
    for k in [0, 4, 7, 8] {
        at.freq[k] = top;
    }
    let m = activated(make, &[], fs);
    let curve = response_curve(&*m).unwrap();
    let pts = twelfth_octave_points(fs);
    let (mut a, mut b) = (vec![0.0; pts.len()], vec![0.0; pts.len()]);
    curve.magnitude_db(&hi.values(), fs, &pts, &mut a);
    curve.magnitude_db(&at.values(), fs, &pts, &mut b);
    assert_eq!(bit_equal_f64(&a, &b), None);
    assert!(a.iter().all(|v| v.is_finite()));
    let x = common::white(0x22, 0.5, 20_000);
    let run = |c: &Cfg| {
        let mut m = activated(make, &borrow(&c.settings()), fs);
        render(&mut *m, &x, &[], Blocks::Fixed(1000))
    };
    assert_eq!(bit_equal(&run(&hi), &run(&at)), None);
}

// ---------------------------------------------------------------------------------------------
// AC-5
// ---------------------------------------------------------------------------------------------

fn ac5_for(kind: usize) {
    let (mut worst_fc, mut n_checked) = (0.0f64, 0);
    for fs in RATES {
        for order in 1..=8 {
            for fc in [30.0, 80.0, 1_000.0, 8_000.0] {
                let mut cfg = Cfg::all_off();
                cfg.on[kind] = true;
                cfg.freq[kind] = fc;
                cfg.order = [order, order];
                let len = ir_len(slowest_pole(&cfg, fs));
                let h = impulse_response(&cfg, fs, len);
                let db = dtft_db(&h, fs, &[0.99 * fc, fc, 1.01 * fc]);
                let label = format!("{} N {order} fc {fc} @ {fs}", KEYS[kind]);
                let e = (db[1] + 3.0103).abs();
                assert!(e <= 0.02, "{label}: |H(fc)| = {:.4} dB", db[1]);
                worst_fc = worst_fc.max(e);
                // The −3.01 dB crossing lies within fc ± 1 %.
                let (below, above) = if kind == 0 {
                    (db[0], db[2])
                } else {
                    (db[2], db[0])
                };
                assert!(below < -3.0103 && above > -3.0103, "{label}: {db:?}");
                n_checked += 1;
            }
        }
    }
    println!(
        "AC-5 {}: {n_checked} cases, worst |H(fc)| error {worst_fc:.5} dB",
        KEYS[kind]
    );
}

#[test]
fn ac5_high_pass_is_minus_3db_at_fc() {
    ac5_for(0);
}

#[test]
fn ac5_low_pass_is_minus_3db_at_fc() {
    ac5_for(8);
}

// ---------------------------------------------------------------------------------------------
// AC-7
// ---------------------------------------------------------------------------------------------

#[test]
fn ac7_no_change_before_the_event() {
    let fs = 48_000.0;
    let mut cfg = Cfg::defaults();
    cfg.on = [true; 9];
    cfg.gain = [0.0, 3.0, -4.0, 6.0, 5.0, -2.0, 4.0, 2.0, 0.0];
    let x = common::white(0x7, 0.3, 12_000);
    let s = cfg.settings();
    let k = 5_001;
    let base = {
        let mut m = activated(make, &borrow(&s), fs);
        render(&mut *m, &x, &[], Blocks::Fixed(1024))
    };
    for (key, value) in [
        ("master_gain_db", -6.0),
        ("hp_freq_hz", 300.0),
        ("hp_slope", 7.0),
        ("hp_on", 0.0),
        ("ls_gain_db", -9.0),
        ("b3_freq_hz", 4_000.0),
        ("b3_gain_db", 12.0),
        ("b3_q", 8.0),
        ("b5_on", 0.0),
        ("hs_q", 1.8),
        ("lp_freq_hz", 3_000.0),
        ("lp_slope", 0.0),
    ] {
        let p = eq_param(key);
        let changes: Vec<Change> = vec![(k, p.id, value)];
        for blocks in [Blocks::Fixed(1024), Blocks::Random { seed: 3, max: 700 }] {
            let mut m = activated(make, &borrow(&s), fs);
            let y = render(&mut *m, &x, &changes, blocks);
            let d = bit_equal(&base, &y);
            assert!(d.is_some_and(|d| d >= k), "{key}: first difference {d:?}");
            assert!(
                d.unwrap() < k + 960,
                "{key}: no effect within 20 ms ({d:?})"
            );
        }
    }
}

/// AC-7: a master gain step 0 → −6 dB changes the output at exactly k and is at −6 dB within
/// 1e-6 relative from k + round(0.020·fs).
#[test]
fn ac7_master_gain_step() {
    for fs in RATES {
        let n = (0.020 * fs).round() as usize;
        let x: Vec<f32> = (0..3 * n + 500)
            .map(|i| 0.5 + 0.25 * (i as f32 * 0.01).sin())
            .collect();
        let target = 10f64.powf(-6.0 / 20.0);
        for k in [0usize, 1, 377, 2000] {
            let ch: Vec<Change> = vec![(k, ParametricEq::MASTER_GAIN_DB, -6.0)];
            let y = render(&mut *activated(make, &[], fs), &x, &ch, Blocks::Fixed(4096));
            for i in 0..k {
                assert_eq!(
                    y[i].to_bits(),
                    x[i].to_bits(),
                    "{fs}: sample {i} before k {k}"
                );
            }
            assert_ne!(y[k].to_bits(), x[k].to_bits(), "{fs}: no change at k {k}");
            for i in k + n..x.len() {
                let g = f64::from(y[i]) / f64::from(x[i]);
                assert!(
                    (g / target - 1.0).abs() <= 1e-6,
                    "{fs} k {k}: sample {i}: {g}"
                );
            }
            let g = f64::from(y[k + n - 2]) / f64::from(x[k + n - 2]);
            assert!(
                (g / target - 1.0).abs() > 1e-6,
                "{fs} k {k}: ramp too short"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC-8 — SPEC-012 §4.3, normative rows of SPEC-015 §4.5 at 48 kHz, offline mode
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Mv {
    Up,
    Down,
    Drag,
}

const STEPS: &[Mv] = &[Mv::Up, Mv::Down];
const ALL: &[Mv] = &[Mv::Up, Mv::Down, Mv::Drag];
const DRAG: &[Mv] = &[Mv::Drag];

/// One offline §4.3 run: `key` starts at `from`, then `events` (input sample, plain value).
fn zipper_run(
    settings: &[(&str, f64)],
    key: &str,
    from: f64,
    events: &[(usize, f64)],
) -> ZipperReport {
    let id = eq_param(key).id;
    let mut s = settings.to_vec();
    s.push((key, from));
    let mut m = activated(make, &s, ZIPPER_SAMPLE_RATE);
    let changes: Vec<Change> = events.iter().map(|&(at, v)| (at, id, v)).collect();
    let y = render(&mut *m, &zipper_signal(), &changes, Blocks::Fixed(4096));
    let window = ZipperWindow {
        start: events[0].0,
        end: events[events.len() - 1].0,
        t_s_ms: 20.0,
    };
    analyze_zipper(&y, 0, ZIPPER_SAMPLE_RATE, window).expect("analyze_zipper")
}

fn zipper_move(settings: &[(&str, f64)], key: &str, mv: Mv) -> ZipperReport {
    let p = eq_param(key);
    let v = |t: f64| p.from_normalized(t);
    match mv {
        Mv::Up => zipper_run(settings, key, v(0.25), &[(ZIPPER_CHANGE_AT, v(0.75))]),
        Mv::Down => zipper_run(settings, key, v(0.75), &[(ZIPPER_CHANGE_AT, v(0.25))]),
        Mv::Drag => {
            let events: Vec<(usize, f64)> = (0..ZIPPER_DRAG_EVENTS)
                .map(|i| {
                    let at = ZIPPER_CHANGE_AT
                        + (i as f64 * ZIPPER_DRAG_INTERVAL_MS / 1000.0 * ZIPPER_SAMPLE_RATE).round()
                            as usize;
                    (
                        at,
                        v(0.25 + 0.5 * i as f64 / (ZIPPER_DRAG_EVENTS - 1) as f64),
                    )
                })
                .collect();
            zipper_run(settings, key, v(0.25), &events)
        }
    }
}

/// Collects §4.3 results; panics at the end if any gated run failed.
#[derive(Default)]
struct Gate {
    worst: f64,
    failures: Vec<String>,
    runs: usize,
}

impl Gate {
    fn check(&mut self, label: &str, r: &ZipperReport) {
        println!("§4.3 {label}: {r}");
        self.runs += 1;
        self.worst = if self.runs == 1 {
            r.worst_excess_db
        } else {
            self.worst.max(r.worst_excess_db)
        };
        if !r.pass {
            self.failures.push(format!("{label}: {r}"));
        }
    }

    fn row(&mut self, settings: &[(&str, f64)], key: &str, moves: &[Mv]) {
        let row_label = format!("{key} {settings:?}");
        let mut row_worst = f64::NEG_INFINITY;
        for &mv in moves {
            let r = zipper_move(settings, key, mv);
            row_worst = row_worst.max(r.worst_excess_db);
            self.check(&format!("{row_label} {mv:?}"), &r);
        }
        println!("ROW {row_label}: worst {row_worst:+.1} dB");
    }

    fn finish(self, what: &str) {
        println!(
            "§4.3 {what}: {} runs, worst excess {:+.1} dB",
            self.runs, self.worst
        );
        assert!(
            self.failures.is_empty(),
            "§4.3 failures:\n{}",
            self.failures.join("\n")
        );
    }
}

#[test]
fn ac8_master_and_band_gains() {
    let mut g = Gate::default();
    g.row(&[], "master_gain_db", ALL);
    for q in [0.1, 1.0, 30.0] {
        g.row(&[("b3_freq_hz", 997.0), ("b3_q", q)], "b3_gain_db", ALL);
    }
    g.row(
        &[("ls_freq_hz", 1000.0), ("ls_q", 0.707)],
        "ls_gain_db",
        ALL,
    );
    g.row(
        &[("hs_freq_hz", 1000.0), ("hs_q", 0.707)],
        "hs_gain_db",
        ALL,
    );
    g.finish("gains");
}

#[test]
fn ac8_qs() {
    let mut g = Gate::default();
    g.row(&[("b3_freq_hz", 1414.0), ("b3_gain_db", 12.0)], "b3_q", ALL);
    for gain in [-24.0, 24.0] {
        g.row(
            &[("b3_freq_hz", 1414.0), ("b3_gain_db", gain)],
            "b3_q",
            STEPS,
        );
    }
    g.row(&[("ls_freq_hz", 2000.0), ("ls_gain_db", 12.0)], "ls_q", ALL);
    g.row(&[("hs_freq_hz", 2000.0), ("hs_gain_db", 12.0)], "hs_q", ALL);
    g.finish("Qs");
}

#[test]
fn ac8_peak_and_shelf_frequencies() {
    let mut g = Gate::default();
    for gain in [12.0, -12.0] {
        g.row(&[("b3_gain_db", gain), ("b3_q", 1.0)], "b3_freq_hz", ALL);
        for q in [0.1, 4.0, 30.0] {
            g.row(&[("b3_gain_db", gain), ("b3_q", q)], "b3_freq_hz", DRAG);
        }
    }
    for gain in [24.0, -24.0] {
        for q in [0.1, 1.0, 4.0] {
            g.row(&[("b3_gain_db", gain), ("b3_q", q)], "b3_freq_hz", DRAG);
        }
    }
    for gain in [12.0, -12.0] {
        g.row(&[("ls_gain_db", gain), ("ls_q", 0.707)], "ls_freq_hz", ALL);
        g.row(&[("hs_gain_db", gain), ("hs_q", 0.707)], "hs_freq_hz", ALL);
    }
    g.finish("peak/shelf frequencies");
}

fn ac8_pass_freq(band: &str) {
    let mut g = Gate::default();
    let on = format!("{band}_on");
    let slope = format!("{band}_slope");
    for idx in 0..8 {
        g.row(
            &[(on.as_str(), 1.0), (slope.as_str(), f64::from(idx))],
            &format!("{band}_freq_hz"),
            ALL,
        );
    }
    g.finish(&format!("{band} frequency"));
}

#[test]
fn ac8_high_pass_frequency_every_slope() {
    ac8_pass_freq("hp");
}

#[test]
fn ac8_low_pass_frequency_every_slope() {
    ac8_pass_freq("lp");
}

#[test]
fn ac8_stepped_slopes_and_enables() {
    let mut g = Gate::default();
    // 18 ↔ 36 dB/oct (normalized 0.25 ↔ 0.75 = index 2 ↔ 5).
    for (band, f) in [("hp", 500.0), ("hp", 900.0), ("lp", 1100.0), ("lp", 2000.0)] {
        let (on, freq) = (format!("{band}_on"), format!("{band}_freq_hz"));
        g.row(
            &[(on.as_str(), 1.0), (freq.as_str(), f)],
            &format!("{band}_slope"),
            STEPS,
        );
    }
    // 6 ↔ 48 dB/oct at LP 1.1 kHz.
    let s = [("lp_on", 1.0), ("lp_freq_hz", 1100.0)];
    g.check(
        "lp_slope 6→48 @1.1k",
        &zipper_run(&s, "lp_slope", 0.0, &[(ZIPPER_CHANGE_AT, 7.0)]),
    );
    g.check(
        "lp_slope 48→6 @1.1k",
        &zipper_run(&s, "lp_slope", 7.0, &[(ZIPPER_CHANGE_AT, 0.0)]),
    );
    // Enables.
    g.row(&[("hp_slope", 7.0), ("hp_freq_hz", 500.0)], "hp_on", STEPS);
    g.row(&[("lp_slope", 7.0), ("lp_freq_hz", 1100.0)], "lp_on", STEPS);
    g.row(
        &[("b3_gain_db", 12.0), ("b3_freq_hz", 1000.0)],
        "b3_on",
        STEPS,
    );
    g.finish("slopes and enables");
}

// ---------------------------------------------------------------------------------------------
// AC-10
// ---------------------------------------------------------------------------------------------

#[test]
fn ac10_realtime_equals_offline_and_is_deterministic() {
    let fs = 48_000.0;
    let pink = vox_testkit::signal::pink_noise(0x10, -20.0, 10.0, 48_000).unwrap();
    let sweep = vox_testkit::signal::log_sweep(20.0, 20_000.0, -20.0, 10.0, 48_000).unwrap();
    let x: Vec<f32> = pink.iter().zip(&sweep).map(|(a, b)| a + b).collect();
    let mut cfg = Cfg::defaults();
    cfg.on = [true; 9];
    (cfg.freq[0], cfg.order[0]) = (60.0, 4);
    (cfg.freq[8], cfg.order[1]) = (16_000.0, 2);
    cfg.gain = [0.0, 3.0, -4.0, 2.0, 5.0, -3.0, -6.0, 2.0, 0.0];
    cfg.q = [1.0, 0.7071, 2.0, 1.0, 1.5, 3.0, 6.0, 0.7071, 1.0];
    cfg.master_db = -1.0;
    let s = cfg.settings();
    let params = ParametricEq::new().params().to_vec();
    let mut rng = TestRng::new(0xAC10);
    let changes: Vec<Change> = (0..50)
        .map(|i| {
            let p = &params[(rng.unit_f64() * params.len() as f64) as usize % params.len()];
            (777 + i * 9_000, p.id, p.from_normalized(rng.unit_f64()))
        })
        .collect();
    let realtime = {
        let mut m = activated_with(make, &borrow(&s), fs, ProcessMode::Realtime, 1024);
        render(
            &mut *m,
            &x,
            &changes,
            Blocks::Random {
                seed: 0x5EED,
                max: 1024,
            },
        )
    };
    let offline = |_: ()| {
        let mut m = activated(make, &borrow(&s), fs);
        render(&mut *m, &x, &changes, Blocks::Fixed(4096))
    };
    let (a, b) = (offline(()), offline(()));
    assert_eq!(fnv1a(&a), fnv1a(&b), "offline renders differ");
    let max_diff = a
        .iter()
        .zip(&realtime)
        .map(|(p, q)| (f64::from(*p) - f64::from(*q)).abs())
        .fold(0.0, f64::max);
    println!(
        "AC-10: max |realtime − offline| = {max_diff:e}; output peak {:.3}",
        common::peak(&a)
    );
    assert!(max_diff <= 1e-6, "{max_diff}");
    assert!(a.iter().all(|v| v.is_finite()));
}
