//! SPEC-012 §4.3 harness calibration (AC-5): TestGain (5 ms linear) passes; TestHardStep and
//! TestStair64 fail. Plus sanity checks of the analysis itself and of the other test modules.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
use vox_module_api::test_util::{
    ModuleTestHost, TestDelay, TestGain, TestHardStep, TestNaN, TestReporter, TestRestart,
    TestStair64, ZIPPER_CHANGE_AT, ZIPPER_SAMPLE_RATE, ZipperMode, ZipperMove, ZipperTest,
    ZipperWindow, analyze_zipper, zipper_signal,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, OutputEvents, ParamEvent, ProcessContext,
    ProcessMode, Transport,
};

vox_module_api::install_test_allocator!();

const SEED: u64 = 0x0043_0003;

fn print(label: &str, reports: &[(String, vox_module_api::test_util::ZipperReport)]) {
    for (l, r) in reports {
        println!("{label} {l}: {r}");
    }
}

#[test]
fn test_gain_passes_every_run() {
    let t = ZipperTest::new(|| Box::new(TestGain::new()), TestGain::GAIN_DB);
    let reports = t.assert_passes(SEED);
    print("TestGain", &reports);
}

#[test]
fn hard_step_fails() {
    let t = ZipperTest::new(TestHardStep::new, TestGain::GAIN_DB);
    for mode in [ZipperMode::Realtime { seed: SEED }, ZipperMode::Offline] {
        for mv in [ZipperMove::Up, ZipperMove::Down] {
            let r = t.run(mv, mode).unwrap();
            println!("TestHardStep {mv:?} {mode:?}: {r}");
            assert!(!r.pass, "{mv:?} {mode:?}: {r}");
        }
    }
}

#[test]
fn stair64_fails() {
    let t = ZipperTest::new(TestStair64::new, TestGain::GAIN_DB);
    for mode in [ZipperMode::Realtime { seed: SEED }, ZipperMode::Offline] {
        for mv in [ZipperMove::Up, ZipperMove::Down] {
            let r = t.run(mv, mode).unwrap();
            println!("TestStair64 {mv:?} {mode:?}: {r}");
            assert!(!r.pass, "{mv:?} {mode:?}: {r}");
        }
    }
}

/// An unchanged tone has no transition energy: passes, and the band levels are far below −90.
#[test]
fn untouched_tone_passes_with_margin() {
    let x = zipper_signal();
    let r = analyze_zipper(
        &x,
        0,
        ZIPPER_SAMPLE_RATE,
        ZipperWindow::step(ZIPPER_CHANGE_AT, 15.0),
    )
    .unwrap();
    println!("tone: {r}");
    assert!(r.pass, "{r}");
    assert!(r.band_levels_dbfs.iter().all(|&l| l < -120.0), "{r}");
}

/// A 15 ms linear crossfade −12 → 0 dB passes (calibration table, last row), and a click
/// (one sample set to 0.1) fails.
#[test]
fn crossfade_passes_and_click_fails() {
    let x = zipper_signal();
    let g = 10f32.powf(-12.0 / 20.0);
    let len = 720;
    let y: Vec<f32> = x
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let a = if i < ZIPPER_CHANGE_AT {
                0.0
            } else {
                (((i - ZIPPER_CHANGE_AT) + 1) as f32 / len as f32).min(1.0)
            };
            s * ((1.0 - a) * g + a)
        })
        .collect();
    let r = analyze_zipper(
        &y,
        0,
        ZIPPER_SAMPLE_RATE,
        ZipperWindow::step(ZIPPER_CHANGE_AT, 15.0),
    )
    .unwrap();
    println!("15 ms crossfade -12 -> 0 dB: {r}");
    assert!(r.pass, "{r}");

    let mut z = x.clone();
    z[ZIPPER_CHANGE_AT + 3] += 0.3;
    let r = analyze_zipper(
        &z,
        0,
        ZIPPER_SAMPLE_RATE,
        ZipperWindow::step(ZIPPER_CHANGE_AT, 15.0),
    )
    .unwrap();
    assert!(!r.pass, "{r}");
}

/// The latency shift is honoured: a delayed hard step is judged at S + latency.
#[test]
fn analysis_shifts_by_latency() {
    let x = zipper_signal();
    let mut y = vec![0.0f32; 480];
    y.extend_from_slice(&x[..x.len() - 480]);
    let r = analyze_zipper(
        &y,
        480,
        ZIPPER_SAMPLE_RATE,
        ZipperWindow::step(ZIPPER_CHANGE_AT, 15.0),
    )
    .unwrap();
    assert!(r.pass, "{r}");
    assert!(analyze_zipper(&y, 480, ZIPPER_SAMPLE_RATE, ZipperWindow::step(1000, 15.0)).is_err());
}

fn cfg() -> ActivateConfig {
    ActivateConfig {
        sample_rate: 48_000.0,
        max_block: 1024,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn run_block(
    m: &mut dyn Module,
    at: u64,
    input: &[f32],
    events: &[ParamEvent],
) -> (Vec<f32>, Vec<ParamEvent>, bool) {
    let mut out = vec![0.0; input.len()];
    let mut oe = OutputEvents::with_capacity(64);
    let mut ctx = ProcessContext::new(
        input.len() as u32,
        at,
        Transport::default(),
        events,
        &mut oe,
    );
    m.process(&mut ctx, &[input], &mut [&mut out]);
    let restart = ctx.requested(HostRequest::Restart);
    (out, oe.as_slice().to_vec(), restart)
}

#[test]
fn test_delay_is_exact() {
    let mut d = TestDelay::new(5);
    d.activate(&cfg()).unwrap();
    assert_eq!(d.latency_samples(), 5);
    let x: Vec<f32> = (1..=12).map(|i| i as f32).collect();
    let (y, _, _) = run_block(&mut d, 0, &x, &[]);
    assert_eq!(&y[..5], &[0.0; 5]);
    assert_eq!(&y[5..], &x[..7]);
    ModuleTestHost::new(|| Box::new(TestDelay::new(64)))
        .blocks(32)
        .assert_passes();
}

#[test]
fn test_restart_requests_restart_and_keeps_its_latency() {
    let mut m = TestRestart::new(480);
    m.activate(&cfg()).unwrap();
    assert_eq!(m.latency_samples(), 480);
    let x = vec![0.5f32; 16];
    let (_, _, restart) = run_block(&mut m, 0, &x, &[]);
    assert!(!restart);
    let ev = ParamEvent {
        offset: 3,
        id: TestRestart::LATENCY,
        value: 960.0,
    };
    let (_, _, restart) = run_block(&mut m, 16, &x, &[ev]);
    assert!(restart);
    assert_eq!(
        m.latency_samples(),
        480,
        "latency only changes on re-activation"
    );
    assert_eq!(m.param_value(TestRestart::LATENCY), Some(960.0));
    let state = m.save_state().unwrap();
    let mut fresh = TestRestart::new(0);
    fresh.load_state(&state).unwrap();
    fresh.activate(&cfg()).unwrap();
    assert_eq!(fresh.latency_samples(), 960);
}

#[test]
fn test_nan_and_reporter() {
    let mut n = TestNaN::new(10);
    n.activate(&cfg()).unwrap();
    let (y, _, _) = run_block(&mut n, 5, &[0.25; 8], &[]);
    assert!(y[..5].iter().all(|v| *v == 0.25));
    assert!(y[5..].iter().all(|v| v.is_nan()));

    let mut r = TestReporter::new(100);
    r.activate(&cfg()).unwrap();
    let (_, ev, _) = run_block(&mut r, 0, &[0.0; 250], &[]);
    let got: Vec<(u32, f64)> = ev.iter().map(|e| (e.offset, e.value)).collect();
    assert_eq!(got, vec![(100, 1.0), (200, 2.0)]);
    let (_, ev, _) = run_block(&mut r, 250, &[0.0; 50], &[]);
    assert!(ev.is_empty(), "300 is outside [250, 300)");
    let (_, ev, _) = run_block(&mut r, 300, &[0.0; 10], &[]);
    let got: Vec<(u32, f64)> = ev.iter().map(|e| (e.offset, e.value)).collect();
    assert_eq!(got, vec![(0, 3.0)]);
}
