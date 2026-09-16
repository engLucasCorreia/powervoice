//! H-43 (idle CPU): while the transport is stopped and nothing is monitored, the control thread
//! stops streaming telemetry at the full rate — `VXTM` only sends what changed (and falls silent
//! once the meters rest at the floor), `VXMT` skips frames that repeat the last one, and `VXSA`
//! sends one at-rest frame and then nothing. Playing still publishes every due frame.

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

/// One control tick of simulated wall time (60 Hz).
const TICK_NS: u64 = 16_666_667;
/// A simulated second of control ticks.
const TICKS_PER_SECOND: u32 = 60;
const RATE: u32 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-idle-{tag}-{}-{}",
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
    /// A stopped engine with an open output and a 10 s, −12 dBFS sine document.
    fn new() -> Self {
        let fake = FakeBackend::new(7);
        fake.plug(
            HostId::Alsa,
            FakeDevice::new("DAC").with_output(
                FakeDirection::new(2, &[RATE], RATE)
                    .default_buffer(256)
                    .record_output(),
            ),
        );
        let dir = TempDir::new("doc");
        let store =
            ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(64 << 20)).unwrap();
        let mut w = ChunkWriter::new(store.clone());
        let amp = 10f32.powf(-12.0 / 20.0);
        let samples: Vec<f32> = (0..RATE as usize * 10)
            .map(|i| amp * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin())
            .collect();
        w.append(&samples).unwrap();
        let audio = w.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(RATE, audio.pieces, Vec::new()));
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
        let clock = fake.clone();
        cfg.clock = Arc::new(move || clock.now_ns());
        let mut eng = ManualEngine::new(cfg);
        eng.set_document(Some(PlaybackDoc { store, snapshot }));
        eng.poll_devices();
        Rig {
            fake,
            eng,
            _dir: dir,
        }
    }

    /// `n` control ticks of simulated real time (the output callbacks run in between).
    fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.fake.advance_by(TICK_NS);
            self.eng.tick();
        }
    }
}

fn vxtm_sink(eng: &mut ManualEngine) -> Arc<Mutex<Vec<TelemetryFrame>>> {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f: &TelemetryFrame| {
        sink.lock().unwrap().push(*f)
    })));
    frames
}

fn take<T>(frames: &Arc<Mutex<Vec<T>>>) -> Vec<T> {
    std::mem::take(&mut *frames.lock().unwrap())
}

#[test]
fn stopped_vxtm_sends_the_state_once_then_falls_silent() {
    let mut rig = Rig::new();
    let frames = vxtm_sink(&mut rig.eng);
    rig.ticks(TICKS_PER_SECOND);
    let first = take(&frames);
    assert!(
        !first.is_empty() && first.len() <= 3,
        "the current state goes out once (got {} frames)",
        first.len()
    );
    rig.ticks(10 * TICKS_PER_SECOND);
    assert_eq!(
        take(&frames).len(),
        0,
        "ten idle seconds with the meters at the floor send nothing"
    );
}

#[test]
fn a_seek_while_stopped_sends_one_frame_at_once() {
    let mut rig = Rig::new();
    let frames = vxtm_sink(&mut rig.eng);
    rig.ticks(TICKS_PER_SECOND);
    take(&frames);
    rig.eng
        .transport(TransportCommand::Seek(u64::from(RATE) * 3));
    rig.ticks(1);
    let after = take(&frames);
    assert_eq!(after.len(), 1, "the seek is published on the next tick");
    assert_eq!(after[0].playhead_sample, u64::from(RATE) * 3);
    rig.ticks(TICKS_PER_SECOND);
    assert_eq!(take(&frames).len(), 0);
}

#[test]
fn playing_publishes_every_due_frame_and_stop_decays_to_silence() {
    let mut rig = Rig::new();
    let frames = vxtm_sink(&mut rig.eng);
    rig.ticks(10);
    take(&frames);
    rig.eng.transport(TransportCommand::Play);
    rig.ticks(TICKS_PER_SECOND);
    let playing = take(&frames);
    assert_eq!(playing.len(), 60, "full 60 Hz while playing");
    assert!(
        playing.last().unwrap().out_peak_dbfs > -20.0,
        "the sine reaches the meter"
    );

    rig.eng.set_telemetry_rate_hz(30);
    rig.ticks(TICKS_PER_SECOND);
    assert_eq!(
        take(&frames).len(),
        30,
        "the 30 Hz setting still applies while playing"
    );

    rig.eng.transport(TransportCommand::Stop);
    rig.ticks(TICKS_PER_SECOND);
    let stopping = take(&frames);
    assert!(
        !stopping.is_empty(),
        "Stop is published, with the RMS window emptying"
    );
    let last = stopping.last().unwrap();
    assert!(
        last.out_peak_dbfs <= -120.0 && last.out_rms_dbfs <= -120.0,
        "{last:?}"
    );
    rig.ticks(5 * TICKS_PER_SECOND);
    assert_eq!(take(&frames).len(), 0, "silent once the meters rest");
}

#[test]
fn stopped_vxmt_repeats_nothing_but_playing_publishes_every_frame() {
    let mut rig = Rig::new();
    let frames: Arc<Mutex<Vec<ModuleTelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng
        .set_module_telemetry_sink(Some(Box::new(move |f: &ModuleTelemetryFrame| {
            sink.lock().unwrap().push(f.clone())
        })));
    rig.eng
        .rack_command(RackCommand::Add {
            module_id: TruePeakLimiter::ID.into(),
            index: 0,
        })
        .unwrap();
    rig.ticks(TICKS_PER_SECOND);
    assert!(!take(&frames).is_empty(), "the slot's meter shows up once");
    rig.ticks(5 * TICKS_PER_SECOND);
    assert_eq!(
        take(&frames).len(),
        0,
        "an idle, settled rack sends nothing"
    );

    rig.eng.transport(TransportCommand::Play);
    rig.ticks(TICKS_PER_SECOND);
    assert_eq!(take(&frames).len(), 60, "full rate while playing");
}

#[test]
fn stopped_vxsa_sends_one_frame_at_rest_then_nothing_until_the_signal_returns() {
    let mut rig = Rig::new();
    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    rig.eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| sink.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );
    rig.ticks(TICKS_PER_SECOND);
    let idle = take(&frames);
    // The output's first frame carries RESET (a fresh ring), then one at-rest frame.
    assert!(
        (1..=2).contains(&idle.len()),
        "one at-rest frame (got {})",
        idle.len()
    );
    assert!(idle.iter().all(|f| f.silent));
    rig.ticks(5 * TICKS_PER_SECOND);
    assert_eq!(take(&frames).len(), 0);

    rig.eng.transport(TransportCommand::Play);
    rig.ticks(TICKS_PER_SECOND);
    let playing = take(&frames);
    assert!(
        playing.len() >= 58,
        "every frame while the signal plays (got {})",
        playing.len()
    );

    // After Stop the curve falls (frames keep coming while it does), then rests.
    rig.eng.transport(TransportCommand::Stop);
    rig.ticks(20 * TICKS_PER_SECOND);
    let falling = take(&frames);
    assert!(falling.len() > 1, "the decay is published");
    assert!(falling.last().unwrap().silent);
    rig.ticks(5 * TICKS_PER_SECOND);
    assert_eq!(take(&frames).len(), 0, "and then nothing");
}
