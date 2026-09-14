//! H-16 (`Settings.telemetry_rate_hz`, SPEC-003 §3 / ADR-009): the 30 Hz setting halves the
//! publish rate of every control-thread publisher — `VXTM` (telemetry), `VXMT` (module
//! telemetry) and `VXSA` (the live analyzer) — over a simulated second of control ticks, without
//! changing the fixed 60 Hz control tick itself.

vox_module_api::install_test_allocator!();

use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{
    AnalyzerFrame, AnalyzerResponse, EngineConfig, HostId, ManualEngine, ModuleTelemetryFrame,
    RackCommand, TelemetryFrame,
};
use vox_modules::TruePeakLimiter;
use vox_rack::Registry;

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

/// A control-tick == one `eng.tick()` call: `ManualEngine::tick` runs one whole control tick per
/// call regardless of wall/fake clock time (see `rack_api.rs`'s `module_telemetry_frames_...`
/// test), so "a simulated second" at the nominal 60 Hz control-tick rate is exactly 60 calls.
const TICKS_PER_SECOND: u32 = 60;

fn rig() -> ManualEngine {
    let fake = FakeBackend::new(3);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    eng
}

#[test]
fn telemetry_rate_halves_vxtm_frame_count() {
    let mut eng = rig();
    let frames: Arc<Mutex<Vec<TelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f: &TelemetryFrame| {
        sink.lock().unwrap().push(*f);
    })));

    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXTM frame per control tick"
    );

    frames.lock().unwrap().clear();
    eng.set_telemetry_rate_hz(30);
    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        30,
        "30 Hz halves the VXTM frame count over the same number of control ticks"
    );
}

#[test]
fn telemetry_rate_halves_vxmt_frame_count() {
    let mut eng = rig();
    let frames: Arc<Mutex<Vec<ModuleTelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.set_module_telemetry_sink(Some(Box::new(move |f: &ModuleTelemetryFrame| {
        sink.lock().unwrap().push(f.clone());
    })));
    // A slot with a `Telemetry` extension (H-03): the limiter's gain-reduction meter.
    eng.rack_command(RackCommand::Add {
        module_id: TruePeakLimiter::ID.into(),
        index: 0,
    })
    .unwrap();

    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXMT frame per control tick"
    );

    frames.lock().unwrap().clear();
    eng.set_telemetry_rate_hz(30);
    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        30,
        "30 Hz halves the VXMT frame count over the same number of control ticks"
    );
}

#[test]
fn telemetry_rate_halves_vxsa_frame_count() {
    let mut eng = rig();
    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| sink.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );

    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXSA frame per control tick"
    );

    frames.lock().unwrap().clear();
    eng.set_telemetry_rate_hz(30);
    for _ in 0..TICKS_PER_SECOND {
        eng.tick();
    }
    assert_eq!(
        frames.lock().unwrap().len(),
        30,
        "30 Hz halves the VXSA frame count over the same number of control ticks"
    );
}

/// Switching rates resets the gate's phase (`TelemetryRateGate::set_rate_hz`), so the new 30 Hz
/// cadence starts counting from zero instead of publishing on whatever offset the old 60 Hz rate
/// happened to leave it at: the very next control tick is *not* due (1 of 2), the one after it is.
#[test]
fn telemetry_rate_change_resets_the_gates_phase() {
    let mut eng = rig();
    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| sink.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );
    eng.set_telemetry_rate_hz(30);
    eng.tick();
    assert!(
        frames.lock().unwrap().is_empty(),
        "1 of 2 ticks since the rate change: not due yet"
    );
    eng.tick();
    assert_eq!(
        frames.lock().unwrap().len(),
        1,
        "2 of 2 ticks since the rate change: due"
    );
}
