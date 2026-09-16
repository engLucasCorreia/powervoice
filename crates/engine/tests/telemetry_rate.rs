//! H-16 (`Settings.telemetry_rate_hz`, SPEC-003 §3 / ADR-009): the 30 Hz setting halves the
//! publish rate of every control-thread publisher — `VXTM` (telemetry), `VXMT` (module
//! telemetry) and `VXSA` (the live analyzer) — over a simulated second of control ticks, without
//! changing the fixed 60 Hz control tick itself.
//!
//! H-43: an idle engine (stopped, nothing monitored) only publishes what changed, so these
//! cadence checks run while a document plays — the state in which every due frame goes out.
//! `tests/idle_telemetry.rs` covers the idle side.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{
    AnalyzerFrame, AnalyzerResponse, EngineConfig, HostId, ManualEngine, ModuleTelemetryFrame,
    PlaybackDoc, RackCommand, TelemetryFrame, TransportCommand,
};
use vox_modules::TruePeakLimiter;
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::Registry;

const RATE: u32 = 48_000;

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[RATE], RATE).default_buffer(256)
}

/// A control-tick == one `eng.tick()` call: `ManualEngine::tick` runs one whole control tick per
/// call regardless of wall/fake clock time (see `rack_api.rs`'s `module_telemetry_frames_...`
/// test), so "a simulated second" at the nominal 60 Hz control-tick rate is exactly 60 calls.
const TICKS_PER_SECOND: u32 = 60;
/// Each tick also advances the fake clock by one real control period, so the output callbacks
/// (and with them the analyzer tap) keep running while the document plays.
const TICK_NS: u64 = 16_666_667;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-rate-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    _dir: TempDir,
}

impl Rig {
    fn tick(&mut self) {
        self.fake.advance_by(TICK_NS);
        self.eng.tick();
    }

    fn second(&mut self) {
        for _ in 0..TICKS_PER_SECOND {
            self.tick();
        }
    }
}

/// An engine playing a 30 s, −12 dBFS 440 Hz sine.
fn rig() -> Rig {
    let fake = FakeBackend::new(3);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let dir = TempDir::new();
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(64 << 20)).unwrap();
    let mut w = ChunkWriter::new(store.clone());
    let amp = 10f32.powf(-12.0 / 20.0);
    let samples: Vec<f32> = (0..RATE as usize * 30)
        .map(|i| amp * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin())
        .collect();
    w.append(&samples).unwrap();
    let audio = w.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(RATE, audio.pieces, Vec::new()));
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    let mut rig = Rig {
        fake,
        eng,
        _dir: dir,
    };
    rig.tick();
    rig.eng.transport(TransportCommand::Play);
    // Let the first audio reach the analyzer tap before counting.
    for _ in 0..10 {
        rig.tick();
    }
    rig
}

#[test]
fn telemetry_rate_halves_vxtm_frame_count() {
    let mut rig = rig();
    let frames: Arc<Mutex<Vec<TelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng
        .set_telemetry_sink(Some(Box::new(move |f: &TelemetryFrame| {
            sink.lock().unwrap().push(*f);
        })));

    rig.second();
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXTM frame per control tick"
    );

    frames.lock().unwrap().clear();
    rig.eng.set_telemetry_rate_hz(30);
    rig.second();
    assert_eq!(
        frames.lock().unwrap().len(),
        30,
        "30 Hz halves the VXTM frame count over the same number of control ticks"
    );
}

#[test]
fn telemetry_rate_halves_vxmt_frame_count() {
    let mut rig = rig();
    let frames: Arc<Mutex<Vec<ModuleTelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng
        .set_module_telemetry_sink(Some(Box::new(move |f: &ModuleTelemetryFrame| {
            sink.lock().unwrap().push(f.clone());
        })));
    // A slot with a `Telemetry` extension (H-03): the limiter's gain-reduction meter.
    rig.eng
        .rack_command(RackCommand::Add {
            module_id: TruePeakLimiter::ID.into(),
            index: 0,
        })
        .unwrap();

    rig.second();
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXMT frame per control tick"
    );

    frames.lock().unwrap().clear();
    rig.eng.set_telemetry_rate_hz(30);
    rig.second();
    assert_eq!(
        frames.lock().unwrap().len(),
        30,
        "30 Hz halves the VXMT frame count over the same number of control ticks"
    );
}

#[test]
fn telemetry_rate_halves_vxsa_frame_count() {
    let mut rig = rig();
    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| sink.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );

    rig.second();
    assert_eq!(
        frames.lock().unwrap().len(),
        60,
        "default 60 Hz: one VXSA frame per control tick"
    );

    frames.lock().unwrap().clear();
    rig.eng.set_telemetry_rate_hz(30);
    rig.second();
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
    let mut rig = rig();
    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| sink.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );
    rig.eng.set_telemetry_rate_hz(30);
    rig.tick();
    assert!(
        frames.lock().unwrap().is_empty(),
        "1 of 2 ticks since the rate change: not due yet"
    );
    rig.tick();
    assert_eq!(
        frames.lock().unwrap().len(),
        1,
        "2 of 2 ticks since the rate change: due"
    );
}
