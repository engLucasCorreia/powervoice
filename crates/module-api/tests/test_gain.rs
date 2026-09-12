//! T-005 acceptance tests for the reference module and the Module API surface.

use std::sync::Arc;

use vox_module_api::test_util::{ModuleTestHost, TestGain, TestGainFactory, TestRng, no_alloc};
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventList, EventListError, LocalizedText, Module, ModuleFactory,
    ModuleState, OutputEvents, ParamEvent, ParamFlags, ParamId, ParamInfo, ProcessContext,
    ProcessMode, StateError, Taper, Transport, Unit, noise_profile, prepare_state, response_curve,
    telemetry,
};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;

fn cfg(max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: SR,
        max_block,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn active_gain(state: Option<&ModuleState>) -> TestGain {
    let mut m = TestGain::new();
    if let Some(s) = state {
        m.load_state(&prepare_state(&m, s.clone()).unwrap())
            .unwrap();
    }
    m.activate(&cfg(1024)).unwrap();
    m
}

/// Processes one block under the allocation checker.
fn process(m: &mut dyn Module, input: &[f32], events: &[ParamEvent], steady_time: u64) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    let mut out_events = OutputEvents::with_capacity(16);
    no_alloc(|| {
        let mut ctx = ProcessContext::new(
            input.len() as u32,
            steady_time,
            Transport::default(),
            events,
            &mut out_events,
        );
        m.process(&mut ctx, &[input], &mut [&mut out[..]]);
    })
    .expect("process allocated");
    out
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = TestRng::new(seed);
    (0..n).map(|_| 0.8 * rng.bipolar_f32()).collect()
}

fn gain_event(offset: u32, db: f64) -> ParamEvent {
    ParamEvent {
        offset,
        id: TestGain::GAIN_DB,
        value: db,
    }
}

#[test]
fn test_gain_passes_module_test_host() {
    let report = ModuleTestHost::new(|| Box::new(TestGain::new())).assert_passes();
    assert!(report.alloc_checked);
    assert!(
        report.zero_length_blocks > 0,
        "random blocks include 0-length ones"
    );
    assert!(report.events > 100);
    assert!(report.resets > 0);
    assert!(
        report
            .first_effect
            .iter()
            .all(|p| p.first_difference == Some(p.offset)),
        "{:?}",
        report.first_effect
    );
}

#[test]
fn test_gain_factory_passes_with_small_blocks() {
    let factory: Arc<dyn ModuleFactory> = Arc::new(TestGainFactory::new());
    ModuleTestHost::from_factory(factory)
        .max_block(7)
        .seed(42)
        .assert_passes();
}

#[test]
fn cloned_event_lists_keep_their_capacity() {
    let mut list = EventList::with_capacity(512);
    list.push(gain_event(3, -1.0)).unwrap();
    let mut out = OutputEvents::with_capacity(512);
    out.try_push(gain_event(0, 1.0)).unwrap();
    let (mut list2, mut out2) = (list.clone(), out.clone());
    assert_eq!(list2.as_slice(), list.as_slice());
    assert_eq!(out2.as_slice(), out.as_slice());
    assert_eq!(list2.capacity(), 512);
    no_alloc(|| {
        for i in 1..512u32 {
            list2.push(gain_event(3 + i, 0.0)).unwrap();
            out2.try_push(gain_event(0, 0.0)).unwrap();
        }
    })
    .expect("pushing into a clone up to its capacity allocated");
    assert_eq!(list2.len(), 512);
    assert_eq!(list2.push(gain_event(600, 0.0)), Err(EventListError::Full));
    assert!(out2.try_push(gain_event(0, 0.0)).is_err());
}

#[test]
fn event_at_offset_k_takes_effect_from_sample_k() {
    let ramp = TestGain::ramp_samples(SR) as usize;
    assert_eq!(ramp, 240, "5 ms at 48 kHz");
    let target = TestGain::db_to_gain(-6.0);
    let n = 1024;
    let ones = vec![1.0_f32; n];
    for k in [0usize, 1, 100, 511, 1023] {
        let mut m = active_gain(None);
        let out = process(&mut m, &ones, &[gain_event(k as u32, -6.0)], 0);
        assert!(
            out[..k].iter().all(|&x| x.to_bits() == 1.0_f32.to_bits()),
            "k={k}: unchanged before k"
        );
        assert!(out[k] < 1.0, "k={k}: changes at sample k");
        let step = (target - 1.0) / ramp as f32;
        assert!(
            (out[k] - (1.0 + step)).abs() < 1e-6,
            "k={k}: first ramp step at k"
        );
        // Monotonic ramp, exactly the target from sample k + ramp - 1 on.
        let settled = k + ramp - 1;
        for i in k + 1..n.min(settled) {
            assert!(out[i] < out[i - 1], "k={k}: ramp decreasing at {i}");
        }
        for &x in out.iter().skip(settled) {
            assert_eq!(x.to_bits(), target.to_bits(), "k={k}: settled");
        }
        assert_eq!(
            m.param_value(TestGain::GAIN_DB).map(f64::to_bits),
            Some((-6.0_f64).to_bits())
        );
    }
}

#[test]
fn block_splitting_does_not_change_output() {
    let x = noise(1, 1000);
    let events = [
        gain_event(0, -3.0),
        gain_event(10, -20.0),
        gain_event(500, 6.0),
        gain_event(999, -60.0),
    ];
    let mut whole = active_gain(None);
    let expected = process(&mut whole, &x, &events, 0);

    // Same events, delivered as block-relative offsets of arbitrary sub-blocks (incl. empty ones).
    let mut split = active_gain(None);
    let mut got = Vec::new();
    let bounds = [0, 0, 10, 11, 11, 499, 500, 777, 999, 1000];
    for w in bounds.windows(2) {
        let (s, e) = (w[0], w[1]);
        let sub: Vec<_> = events
            .iter()
            .filter(|ev| (ev.offset as usize) >= s && (ev.offset as usize) < e)
            .map(|ev| ParamEvent {
                offset: ev.offset - s as u32,
                ..*ev
            })
            .collect();
        got.extend(process(&mut split, &x[s..e], &sub, s as u64));
    }
    assert_eq!(got.len(), expected.len());
    assert!(
        got.iter()
            .zip(&expected)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn neg_inf_at_min_is_silence() {
    let mut m = active_gain(Some(&TestGain::state_with_gain_db(-60.0)));
    let out = process(&mut m, &noise(2, 64), &[], 0);
    assert!(out.iter().all(|&x| x == 0.0));
    assert_eq!(m.params()[0].value_to_text(-60.0), "-inf dB");
}

#[test]
fn state_round_trip_is_bit_exact() {
    // Original: non-default gain via events, plus an opaque blob.
    let mut state = TestGain::state_with_gain_db(-12.5);
    state.blob = Some(vec![1, 2, 3, 254, 255]);
    let mut a = active_gain(Some(&state));
    process(
        &mut a,
        &noise(3, 700),
        &[gain_event(3, -7.25), gain_event(650, 3.5)],
        0,
    );
    let saved = a.save_state().unwrap();
    assert_eq!(saved.params["gain_db"].to_bits(), 3.5_f64.to_bits());
    assert_eq!(saved.blob.as_deref(), Some(&[1, 2, 3, 254, 255][..]));

    // Through JSON (the sidecar form), into a fresh instance.
    let json = serde_json::to_string(&saved).unwrap();
    let loaded: ModuleState = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, saved);
    let mut b = active_gain(Some(&loaded));
    assert_eq!(b.save_state().unwrap(), saved);

    a.deactivate();
    a.activate(&cfg(1024)).unwrap();
    let x = noise(4, 2048);
    let events = [gain_event(100, -1.0), gain_event(1500, -30.0)];
    let out_a = process(&mut a, &x, &events, 0);
    let out_b = process(&mut b, &x, &events, 0);
    assert!(
        out_a
            .iter()
            .zip(&out_b)
            .all(|(p, q)| p.to_bits() == q.to_bits())
    );
}

#[test]
fn prepare_state_pipeline() {
    let m = TestGain::new();
    // Unknown keys dropped, missing keys added at default, values clamped.
    let mut s = ModuleState::new(TestGain::STATE_FORMAT_VERSION);
    s.params.insert("bogus".into(), 1.0);
    let p = prepare_state(&m, s).unwrap();
    assert_eq!(p.params.len(), 1);
    assert_eq!(p.params["gain_db"].to_bits(), 0.0_f64.to_bits());
    let p = prepare_state(&m, TestGain::state_with_gain_db(99.0)).unwrap();
    assert_eq!(p.params["gain_db"].to_bits(), 24.0_f64.to_bits());
    // Too new.
    let err = prepare_state(&m, ModuleState::new(TestGain::STATE_FORMAT_VERSION + 1)).unwrap_err();
    assert!(
        matches!(
            err,
            StateError::TooNew {
                found: 2,
                supported: 1
            }
        ),
        "{err}"
    );
    // Older: default migrate_state bumps the version.
    let mut old = TestGain::state_with_gain_db(-3.0);
    old.format_version = 0;
    let p = prepare_state(&m, old).unwrap();
    assert_eq!(p.format_version, TestGain::STATE_FORMAT_VERSION);
    assert_eq!(p.params["gain_db"].to_bits(), (-3.0_f64).to_bits());
}

#[test]
fn factory_and_presets() {
    let f = TestGainFactory::new();
    assert_eq!(f.descriptor().id, TestGain::ID);
    let presets = f.presets();
    assert_eq!(presets.len(), 1);
    let mut m = f.create().unwrap();
    m.load_state(&prepare_state(&*m, presets[0].state.clone()).unwrap())
        .unwrap();
    assert_eq!(
        m.param_value(TestGain::GAIN_DB).map(f64::to_bits),
        Some((-6.0_f64).to_bits())
    );
    assert_eq!(m.param_value(ParamId(99)), None);
}

#[test]
fn extensions_are_typed() {
    let mut m = active_gain(Some(&TestGain::state_with_gain_db(-6.0)));
    let t = telemetry(&m).expect("telemetry");
    let curve = response_curve(&m).expect("response curve");
    assert!(noise_profile(&m).is_none());
    process(&mut m, &noise(5, 32), &[], 0);
    assert!((t.read(0) + 6.0).abs() < 1e-4, "applied gain {}", t.read(0));
    assert_eq!(t.channels()[0].key, "gain_db");
    let mut out = [0.0; 3];
    curve.magnitude_db(&[-6.0], SR, &[100.0, 1000.0, 10_000.0], &mut out);
    assert!(out.iter().all(|&db| (db + 6.0).abs() < 1e-12));
    // Handles stay valid after the instance is dropped.
    drop(m);
    assert!((t.read(0) + 6.0).abs() < 1e-4);
}

fn p(key: &str, unit: Unit, min: f64, max: f64, taper: Taper, decimals: u8) -> ParamInfo {
    ParamInfo {
        id: ParamId(0),
        key: key.into(),
        name: LocalizedText::plain(key),
        group: None,
        unit,
        min,
        max,
        default: min,
        taper,
        step: None,
        enum_labels: Vec::new(),
        decimals,
        smoothing_ms: 0.0,
        flags: ParamFlags::AUTOMATABLE,
    }
}

/// Half a unit of the last shown decimal, per the display rules (kHz / s switch at 1000).
fn half_unit(p: &ParamInfo, v: f64) -> f64 {
    let unit = 10f64.powi(-i32::from(p.decimals));
    let big = matches!(p.unit, Unit::Hz | Unit::Ms) && (v / unit).round() * unit >= 1000.0;
    0.5 * if big { 10.0 } else { unit }
}

#[test]
fn text_round_trips_for_linear_log_and_db_tapers() {
    let continuous = [
        p("mix", Unit::Percent, 0.0, 100.0, Taper::Linear, 0),
        p("width", Unit::None, -1.0, 1.0, Taper::Linear, 3),
        p("hold_s", Unit::Seconds, 0.0, 10.0, Taper::Linear, 2),
        p("target_lufs", Unit::Lufs, -40.0, -5.0, Taper::Linear, 1),
        p("freq_hz", Unit::Hz, 20.0, 20_000.0, Taper::Log, 0),
        p("attack_ms", Unit::Ms, 0.1, 5000.0, Taper::Log, 1),
        p("ratio", Unit::Ratio, 1.0, 20.0, Taper::Log, 1),
        p("q", Unit::None, 0.1, 18.0, Taper::Log, 2),
        p(
            "gain_db",
            Unit::Db,
            -60.0,
            24.0,
            Taper::Db {
                neg_inf_at_min: true,
            },
            1,
        ),
        p(
            "ceiling_dbtp",
            Unit::Dbtp,
            -12.0,
            0.0,
            Taper::Db {
                neg_inf_at_min: false,
            },
            1,
        ),
        p(
            "threshold_dbfs",
            Unit::Dbfs,
            -80.0,
            0.0,
            Taper::Db {
                neg_inf_at_min: false,
            },
            2,
        ),
    ];
    let mut rng = TestRng::new(7);
    for param in &continuous {
        param.validate().unwrap();
        for i in 0..2000 {
            let t = if i < 3 {
                f64::from(i) / 2.0
            } else {
                rng.unit_f64()
            };
            let v = param.from_normalized(t);
            let text = param.value_to_text(v);
            let back = param
                .text_to_value(&text)
                .unwrap_or_else(|| panic!("{text:?}"));
            let tol = half_unit(param, v) * (1.0 + 1e-9) + 1e-9;
            assert!(
                (back - param.clamp_quantize(v)).abs() <= tol,
                "{}: {v} → {text:?} → {back}",
                param.key
            );
        }
    }

    // Stepped (linear, log, dB), enum and bool: exact.
    let mut smp = p("delay_smp", Unit::Samples, 0.0, 4800.0, Taper::Linear, 0);
    smp.step = Some(1.0);
    let mut hz = p("fft_hz", Unit::Hz, 10.0, 900.0, Taper::Log, 0);
    hz.step = Some(10.0);
    let mut db = p(
        "range_db",
        Unit::Db,
        -90.0,
        0.0,
        Taper::Db {
            neg_inf_at_min: true,
        },
        1,
    );
    db.step = Some(0.5);
    let mut on = p("enabled", Unit::None, 0.0, 1.0, Taper::Linear, 0);
    on.step = Some(1.0);
    on.flags |= ParamFlags::BOOL;
    let mut mode = p("mode", Unit::None, 0.0, 2.0, Taper::Linear, 0);
    mode.step = Some(1.0);
    mode.enum_labels = ["Peak", "RMS", "Hybrid"].map(LocalizedText::plain).to_vec();
    for mut param in [smp, hz, db, on, mode] {
        param.flags |= ParamFlags::STEPPED;
        param.validate().unwrap();
        for i in 0..500 {
            let v = param.min + (param.max - param.min) * f64::from(i) / 499.0;
            let q = param.clamp_quantize(v);
            let text = param.value_to_text(v);
            let back = param.text_to_value(&text).unwrap();
            assert_eq!(
                back.to_bits(),
                q.to_bits(),
                "{}: {v} → {text:?} → {back}",
                param.key
            );
        }
    }
}
