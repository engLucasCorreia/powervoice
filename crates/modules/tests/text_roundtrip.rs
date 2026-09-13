//! SPEC-012 AC-11: exact text round-trip for stepped parameters (synthetic cases and every
//! stepped parameter of every built-in); continuous parameters within half a displayed unit.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
use vox_module_api::test_util::TestRng;
use vox_module_api::{LocalizedText, ParamFlags, ParamId, ParamInfo, Taper, Unit};
use vox_modules::builtin_factories;

fn stepped(unit: Unit, min: f64, max: f64, step: f64, decimals: u8) -> ParamInfo {
    ParamInfo {
        id: ParamId(0),
        key: "p".into(),
        name: LocalizedText::plain("p"),
        group: None,
        unit,
        min,
        max,
        default: min,
        taper: Taper::Linear,
        step: Some(step),
        enum_labels: Vec::new(),
        decimals,
        smoothing_ms: 0.0,
        flags: ParamFlags::AUTOMATABLE | ParamFlags::STEPPED,
    }
}

/// Every legal value `min + k·step`, or 10 000 seeded legal values when there are more.
fn legal_values(p: &ParamInfo) -> Vec<f64> {
    let step = p.step.unwrap();
    let count = ((p.max - p.min) / step + 1e-9).floor() as u64 + 1;
    if count <= 10_000 {
        (0..count)
            .map(|k| p.clamp_quantize(p.min + k as f64 * step))
            .collect()
    } else {
        let mut rng = TestRng::new(0xAC11);
        (0..10_000)
            .map(|_| p.clamp_quantize(p.min + rng.below(count) as f64 * step))
            .collect()
    }
}

/// One unit of the last digit shown in `text`, in plain units (×1000 after the Hz → kHz /
/// ms → s display switch, which shows 2 decimals).
fn displayed_resolution(p: &ParamInfo, text: &str) -> f64 {
    let number = text.split_whitespace().next().unwrap_or("");
    let decimals = number.split_once('.').map_or(0, |(_, frac)| {
        frac.chars().take_while(char::is_ascii_digit).count()
    });
    let switched = text.ends_with(" kHz") || (p.unit == Unit::Ms && text.ends_with(" s"));
    let scale = if switched { 1000.0 } else { 1.0 };
    scale * 10f64.powi(-(decimals as i32))
}

fn assert_exact_roundtrip(p: &ParamInfo) {
    p.validate().unwrap();
    for v in legal_values(p) {
        let text = p.value_to_text(v);
        let back = p.text_to_value(&text);
        assert_eq!(
            back.map(f64::to_bits),
            Some(p.clamp_quantize(v).to_bits()),
            "`{}` {v} → {text:?} → {back:?}",
            p.key
        );
    }
}

#[test]
fn step_half_with_zero_decimals() {
    let p = stepped(Unit::None, 0.0, 10.0, 0.5, 0);
    assert_eq!(p.value_to_text(2.5), "2.5");
    assert_exact_roundtrip(&p);
}

#[test]
fn step_quarter_from_minus_one() {
    assert_exact_roundtrip(&stepped(Unit::None, -1.0, 3.0, 0.25, 0));
}

#[test]
fn step_one_hz_above_1000() {
    let p = stepped(Unit::Hz, 20.0, 20_000.0, 1.0, 0);
    assert_eq!(p.value_to_text(1251.0), "1.251 kHz");
    assert_exact_roundtrip(&p);
}

#[test]
fn step_tenth_ms_above_1000() {
    let p = stepped(Unit::Ms, 0.0, 5000.0, 0.1, 1);
    assert_exact_roundtrip(&p);
}

#[test]
fn builtin_parameters_round_trip() {
    for f in builtin_factories() {
        let m = f.create().unwrap();
        for p in m.params() {
            if p.is_stepped() {
                assert_exact_roundtrip(p);
            } else {
                let mut rng = TestRng::new(0xC0);
                for _ in 0..10_000 {
                    let v = p.from_normalized(rng.unit_f64());
                    let text = p.value_to_text(v);
                    let back = p.text_to_value(&text).unwrap();
                    // Half a unit of the last displayed digit (kHz / s after the unit switch).
                    let half = 0.5 * displayed_resolution(p, &text) * (1.0 + 1e-9);
                    assert!(
                        (back - v).abs() <= half || (v <= p.min && back <= p.min),
                        "`{}` {v} → {text:?} → {back}",
                        p.key
                    );
                }
            }
        }
    }
}
