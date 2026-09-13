//! Built-in Gain: `ModuleTestHost` without opt-outs, SPEC-012 AC-5 (§4.3 in both directions and
//! the drag variant, realtime and offline) and AC-6 (event timing and smoothing).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
use vox_module_api::test_util::{ModuleTestHost, ZipperTest};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleFactory, OutputEvents, ParamEvent, ProcessContext,
    ProcessMode, Taper, Transport, Unit, prepare_state,
};
use vox_modules::{Gain, GainFactory, builtin_factories};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;

#[test]
fn gain_passes_module_test_host_without_opt_outs() {
    let report = ModuleTestHost::new(|| Box::new(Gain::new())).assert_passes();
    assert!(report.alloc_checked);
    assert!(report.zero_length_blocks > 0);
    // Every event-timing probe saw the change exactly at its offset (latency 0).
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
fn gain_schema_and_identity() {
    let f = GainFactory::new();
    let d = f.descriptor();
    assert_eq!(d.id, "org.powervoice.gain");
    assert_eq!(d.version.to_string(), "1.0.0");
    let m = f.create().unwrap();
    let p = &m.params()[0];
    assert_eq!(p.key, "gain_db");
    assert_eq!((p.min, p.max, p.default), (-60.0, 24.0, 0.0));
    assert_eq!(
        p.taper,
        Taper::Db {
            neg_inf_at_min: true
        }
    );
    assert_eq!(p.unit, Unit::Db);
    assert_eq!(p.step, None);
    assert_eq!(p.decimals, 1);
    assert_eq!(p.smoothing_ms, 20.0);
    assert_eq!(p.value_to_text(-60.0), "-inf dB");
    assert_eq!(p.value_to_text(-6.0), "-6.0 dB");
    let gain = builtin_factories()
        .into_iter()
        .find(|f| f.descriptor().id == Gain::ID)
        .expect("Gain is a built-in");
    ModuleTestHost::from_factory(gain)
        .blocks(16)
        .check_schema()
        .unwrap();
}

#[test]
fn gain_passes_zipper_in_every_run() {
    let reports =
        ZipperTest::new(|| Box::new(Gain::new()), Gain::GAIN_DB).assert_passes(0x6A1_0005);
    for (label, r) in &reports {
        println!("Gain {label}: {r}");
    }
}

fn activated(db: f64) -> Box<dyn Module> {
    let mut m: Box<dyn Module> = Box::new(Gain::new());
    let s = prepare_state(&*m, Gain::state_with_gain_db(db)).unwrap();
    m.load_state(&s).unwrap();
    m.activate(&ActivateConfig {
        sample_rate: SR,
        max_block: 4096,
        mode: ProcessMode::Offline,
        layout: ChannelLayout::MONO,
    })
    .unwrap();
    m
}

fn render(m: &mut dyn Module, x: &[f32], events: &[ParamEvent]) -> Vec<f32> {
    let mut y = vec![0.0; x.len()];
    let mut oe = OutputEvents::with_capacity(8);
    let mut ctx = ProcessContext::new(x.len() as u32, 0, Transport::default(), events, &mut oe);
    m.process(&mut ctx, &[x], &mut [&mut y]);
    y
}

/// AC-6: step 0 → −6 dB at block offset k.
#[test]
fn ac6_event_timing_and_smoothing() {
    let x: Vec<f32> = (0..4000)
        .map(|i| 0.5 + 0.25 * (i as f32 * 0.01).sin())
        .collect();
    let ramp = Gain::ramp_samples(SR) as usize;
    assert_eq!(ramp, 960);
    for k in [0usize, 1, 377, 2000] {
        let ev = ParamEvent {
            offset: k as u32,
            id: Gain::GAIN_DB,
            value: -6.0,
        };
        let base = render(&mut *activated(0.0), &x, &[]);
        let y = render(&mut *activated(0.0), &x, &[ev]);
        for i in 0..k {
            assert_eq!(
                y[i].to_bits(),
                base[i].to_bits(),
                "k {k}: sample {i} changed early"
            );
        }
        assert_ne!(y[k].to_bits(), base[k].to_bits(), "k {k}: no change at k");
        let target = Gain::db_to_gain(-6.0);
        for i in k + ramp..x.len() {
            let g = f64::from(y[i]) / f64::from(x[i]);
            assert!(
                (g / target - 1.0).abs() <= 1e-6,
                "k {k}: sample {i}: gain {g} vs target {target}"
            );
        }
        // Still ramping one sample before.
        let g = f64::from(y[k + ramp - 2]) / f64::from(x[k + ramp - 2]);
        assert!((g / target - 1.0).abs() > 1e-6, "k {k}: ramp too short");
    }
}

#[test]
fn minimum_is_silence_and_ramp_is_linear() {
    let x = vec![1.0f32; 2000];
    let ev = ParamEvent {
        offset: 0,
        id: Gain::GAIN_DB,
        value: -60.0,
    };
    let y = render(&mut *activated(0.0), &x, &[ev]);
    assert!(y[960..].iter().all(|&s| s == 0.0));
    // Linear in linear gain: equal steps.
    let d1 = y[10] - y[11];
    let d2 = y[500] - y[501];
    assert!((d1 - d2).abs() < 1e-6, "{d1} vs {d2}");
    assert!((y[479] - 0.5).abs() < 2e-3, "{}", y[479]);
}
