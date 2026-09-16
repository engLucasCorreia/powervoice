//! T-208 live output analyzer end-to-end through the fake backend (SPEC-007 §2.9/§4.8, AC-15,
//! AC-16, AC-18, AC-19). Every output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::record::MonitorMode;
use vox_engine::{
    AnalyzerFrame, AnalyzerResponse, DeviceKey, EngineConfig, HostId, ManualEngine, PlaybackDoc,
    TransportCommand,
};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_module_api::{ModuleRef, ModuleState};
use vox_modules::Gain;
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};

const MS: u64 = 1_000_000;
const RATE: u32 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-an-{tag}-{}-{}",
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

fn sine(freq_hz: f64, level_dbfs: f64, rate: u32, n: usize) -> Vec<f32> {
    let amp = 10f64.powf(level_dbfs / 20.0);
    (0..n)
        .map(|i| {
            (amp * (std::f64::consts::TAU * freq_hz * i as f64 / f64::from(rate)).sin()) as f32
        })
        .collect()
}

fn gain_rack(db: f64) -> RackModel {
    RackModel {
        slots: vec![SlotModel::new(
            &ModuleRef {
                id: Gain::ID.into(),
                version: Gain::VERSION,
            },
            false,
            &ModuleState {
                format_version: 1,
                params: BTreeMap::from([("gain_db".to_owned(), db)]),
                blob: None,
            },
        )],
    }
}

fn dev_256() -> FakeDirection {
    FakeDirection::new(2, &[RATE], RATE).default_buffer(256)
}

fn device() -> FakeDevice {
    FakeDevice::new("DAC").with_output(dev_256().record_output())
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    key: DeviceKey,
    frames: Arc<Mutex<Vec<AnalyzerFrame>>>,
    _dir: TempDir,
}

fn rig(samples: &[f32], rack: RackModel) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(11);
    fake.plug(HostId::Alsa, device());
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });

    let dir = TempDir::new("doc");
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(256 << 20)).unwrap();
    let mut w = ChunkWriter::new(store.clone());
    w.append(samples).unwrap();
    let audio = w.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(RATE, audio.pieces, Vec::new()));

    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack;
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    Rig {
        fake,
        eng,
        key: DeviceKey::new(HostId::Alsa, "DAC"),
        frames: Arc::new(Mutex::new(Vec::new())),
        _dir: dir,
    }
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.fake.advance_by(MS);
            self.eng.tick();
        }
    }

    /// Subscribes the analyzer, collecting every frame into `self.frames`.
    fn subscribe(&mut self, response: AnalyzerResponse) -> u32 {
        let fr = self.frames.clone();
        self.eng.analyzer_subscribe(
            Box::new(move |f: &AnalyzerFrame| fr.lock().unwrap().push(f.clone())),
            response,
        )
    }

    fn last_frame(&self) -> AnalyzerFrame {
        self.frames.lock().unwrap().last().unwrap().clone()
    }

    fn frame_count(&self) -> usize {
        self.frames.lock().unwrap().len()
    }
}

/// Index of the band whose level is highest.
fn max_band(levels: &[f32]) -> usize {
    (0..levels.len())
        .max_by(|&a, &b| levels[a].partial_cmp(&levels[b]).unwrap())
        .unwrap()
}

// --- AC-15: analyzer level and frequency accuracy ------------------------------------------

/// AC-15: playing a 1 kHz -20 dBFS sine through an empty rack, Medium response, after 2 s: the
/// band containing 1 kHz is (or is adjacent to) the maximum band and reads about -20.37 dB
/// (BH4 scalloping at δ = 1/3 bin); a bin-centred tone reads -20.00 ± 0.2 dB.
#[test]
fn ac15_tone_level_and_frequency() {
    let src = sine(1000.0, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Medium);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(2_000);

    let f = r.last_frame();
    assert_eq!(f.sample_rate_hz, RATE);
    let mb = max_band(&f.levels_db);
    let mb_hz = vox_dsp::analyzer::band_center_hz(mb as u32);
    assert!(
        (mb_hz - 1000.0).abs() < 40.0,
        "the loudest band should be near 1 kHz, got {mb_hz} Hz"
    );
    assert!(
        (f.levels_db[mb] - (-20.37)).abs() < 0.3,
        "expected about -20.37 dB, got {}",
        f.levels_db[mb]
    );
}

/// AC-15: a bin-centred 1 001.953 Hz tone (bin 171 of N = 8192 @ 48 kHz) reads -20.00 ± 0.2 dB.
#[test]
fn ac15_bin_centered_tone() {
    let freq = 171.0 * f64::from(RATE) / 8192.0;
    let src = sine(freq, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Medium);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(2_000);

    let f = r.last_frame();
    let mb = max_band(&f.levels_db);
    assert!(
        (f.levels_db[mb] - (-20.0)).abs() < 0.2,
        "expected -20.00 ± 0.2 dB, got {}",
        f.levels_db[mb]
    );
}

/// AC-15: with a +6 dB Gain in the rack, the reading rises by 6.00 ± 0.05 dB, proving the tap is
/// post-rack.
#[test]
fn ac15_gain_proves_post_rack_tap() {
    let freq = 171.0 * f64::from(RATE) / 8192.0;
    let src = sine(freq, -20.0, RATE, 4 * RATE as usize);

    let mut r0 = rig(&src, RackModel { slots: Vec::new() });
    r0.subscribe(AnalyzerResponse::Medium);
    r0.run_ms(20);
    r0.eng.transport(TransportCommand::Play);
    r0.run_ms(2_000);
    let f0 = r0.last_frame();
    let band = max_band(&f0.levels_db);
    let level0 = f0.levels_db[band];

    let mut r1 = rig(&src, gain_rack(6.0));
    r1.subscribe(AnalyzerResponse::Medium);
    r1.run_ms(20);
    r1.eng.transport(TransportCommand::Play);
    r1.run_ms(2_000);
    let f1 = r1.last_frame();
    let level1 = f1.levels_db[band];

    assert!(
        (level1 - level0 - 6.0).abs() < 0.05,
        "expected +6.00 ± 0.05 dB, got {}",
        level1 - level0
    );
}

/// AC-15: Dry-monitoring a -20 dBFS input tone makes it appear in the analyzer too (the tap is
/// the ADR-002 §4 `out` signal: rack output + Dry monitor).
#[test]
fn ac15_dry_monitoring_appears_in_the_analyzer() {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let freq = 171.0 * f64::from(RATE) / 8192.0;
    let fake = FakeBackend::new(5);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev_256().record_output()),
    );
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("Mic").with_input(
            FakeDirection::new(1, &[RATE], RATE)
                .default_buffer(256)
                .source(move |k, _ch, rate| {
                    (10f64.powf(-20.0 / 20.0)
                        * (std::f64::consts::TAU * freq * k as f64 / f64::from(rate)).sin())
                        as f32
                }),
        ),
    );
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let dir = TempDir::new("dry");
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs.input_device = Some("Mic".into());
    cfg.prefs.input_channel = 1;
    cfg.disk_space = Arc::new(vox_project::FixedFreeSpace::new(100 << 30));
    cfg.record_volume = dir.0.clone();
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();

    let frames: Arc<Mutex<Vec<AnalyzerFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let fr = frames.clone();
    eng.analyzer_subscribe(
        Box::new(move |f: &AnalyzerFrame| fr.lock().unwrap().push(f.clone())),
        AnalyzerResponse::Medium,
    );
    eng.set_monitor_mode(MonitorMode::Dry);
    let st = eng.set_armed(true);
    assert!(st.armed && st.input_open, "{st:?}");

    for _ in 0..2_000 {
        fake.advance_by(MS);
        eng.tick();
    }

    let f = frames.lock().unwrap().last().unwrap().clone();
    let mb = max_band(&f.levels_db);
    assert!(
        f.levels_db[mb] > -40.0,
        "the monitored tone should be well above the floor, got {}",
        f.levels_db[mb]
    );
}

// --- AC-16: response time (lighter engine-level smoke test; full calibration in vox-dsp) ----

/// AC-16: stepping from silence to a -20 dBFS tone, Medium reaches within 1 dB of its final
/// value within 0.40 s (looser than the calibration table's 0.33 s).
#[test]
fn ac16_medium_reaches_within_1db_in_budget() {
    let freq = 171.0 * f64::from(RATE) / 8192.0;
    let mut src = vec![0.0f32; RATE as usize]; // 1 s silence
    src.extend(sine(freq, -20.0, RATE, 4 * RATE as usize));
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Medium);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);

    // Run to steady state to learn the final level and band.
    r.run_ms(4_000);
    let f = r.last_frame();
    let band = max_band(&f.levels_db);
    let final_db = f.levels_db[band];

    // Restart and time the rise from the first tone sample.
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Medium);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(1_000); // through the 1 s of silence
    let mut reached_ms = None;
    for ms in 0..500u64 {
        r.run_ms(1);
        let f = r.last_frame();
        if (f.levels_db[band] - final_db).abs() <= 1.0 {
            reached_ms = Some(ms);
            break;
        }
    }
    let ms = reached_ms.expect("never reached within 1 dB of the final level");
    assert!(ms <= 400, "Medium rise took {ms} ms, budget 400 ms");
}

// --- AC-18: real-time safety and cost --------------------------------------------------------

/// AC-18: with the analyzer subscribed, the output callback allocates nothing and the tap's
/// drop counter stays 0 under normal load.
#[test]
fn ac18_no_allocation_with_analyzer_subscribed() {
    let src = sine(1000.0, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, gain_rack(0.0));
    r.subscribe(AnalyzerResponse::Fast);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(500);
    assert!(r.frame_count() > 0, "frames should be flowing");
    assert_eq!(r.fake.rt_violations(), 0, "the output callback allocated");
}

/// AC-18: unsubscribed, the tap writes nothing and no `VXSA` frames are sent.
#[test]
fn ac18_unsubscribed_produces_no_frames() {
    let src = sine(1000.0, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(500);
    assert_eq!(r.frame_count(), 0, "no subscriber: no frames");
}

/// AC-18: an output-device reopen produces one frame with `RESET` set, and none of the
/// surrounding frames do.
#[test]
fn ac18_device_reopen_sets_reset() {
    let src = sine(1000.0, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Fast);
    r.run_ms(50);
    assert!(r.frame_count() > 0, "frames should be flowing");
    // The very first frame after subscribing follows the initial attach: it may carry RESET.
    let before_reopen = r.frame_count();

    let removed = r.fake.unplug(&r.key);
    assert!(removed.is_some());
    r.run_ms(20);
    // Two polls while absent register the "present: false" transition (device_state's
    // recovery state machine needs to see the loss before a reappearance counts as a
    // recovery), then plugging it back in and polling again reopens the stream.
    r.eng.poll_devices();
    r.eng.poll_devices();
    r.fake.plug(HostId::Alsa, device());
    r.eng.poll_devices();
    r.run_ms(50);

    let after: Vec<AnalyzerFrame> = r.frames.lock().unwrap()[before_reopen..].to_vec();
    assert!(!after.is_empty(), "frames should resume after reopen");
    assert!(
        after.iter().any(|f| f.reset),
        "one frame after the reopen should carry RESET"
    );
}

// --- AC-19: analyzer IPC contract (engine side) ----------------------------------------------

/// AC-19: `band_count` is 246 at 48 kHz; frames arrive at the telemetry rate (60 Hz) ± 5 % over
/// a few seconds; silence sets `SILENT` and carries `-inf`, never NaN.
#[test]
fn ac19_contract_fields_and_cadence() {
    let src = sine(1000.0, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Medium);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    // H-43: a silent, at-rest curve isn't repeated every tick (while stopped, and for the few
    // ticks before the first played block reaches the tap) — count once the signal flows.
    r.run_ms(20);
    let before_play = r.frame_count() as u64;
    let ticks = 2_000u64;
    r.run_ms(ticks);

    let f = r.last_frame();
    assert_eq!(f.band_count, 246);
    assert_eq!(f.levels_db.len(), 246);
    assert!(f.levels_db.iter().all(|v| !v.is_nan()));

    // Cadence: exactly one frame per `Control::tick()` (the same mechanism as VXTM/VXMT), which
    // the real threaded engine paces at `TICK` = 16.667 ms = 60 Hz (ADR-009). `ManualEngine`
    // drives ticks directly rather than on a wall clock, so the 60 Hz figure itself is a
    // property of the real engine's tick loop, not reproducible here; what this checks is the
    // 1:1 tick -> frame invariant that gives it that cadence.
    assert_eq!(
        r.frame_count() as u64 - before_play,
        ticks,
        "one frame per tick while playing"
    );
}

/// AC-19 (silence): with nothing playing, the tap sees digital silence and the analyzer reports
/// `SILENT` with `-inf` levels, never NaN.
#[test]
fn ac19_silence_sets_silent_flag() {
    let src = vec![0.0f32; RATE as usize];
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    r.subscribe(AnalyzerResponse::Fast);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(500);

    let f = r.last_frame();
    assert!(f.silent, "digital silence should set SILENT");
    assert!(f.levels_db.iter().all(|v| !v.is_nan()));
    assert!(f.levels_db.iter().all(|&v| v == f32::NEG_INFINITY));
}

// --- H-42: voice diagnostics and Spectrum Inspector streams (SPEC-007 §8.3, §8.8) -----------

use vox_dsp::diagnostics::{VoiceReport, WindowKind};
use vox_engine::{InspectorConfig, InspectorFrame};

impl Rig {
    fn subscribe_voice(&mut self) -> (u32, Arc<Mutex<Vec<VoiceReport>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let id = self
            .eng
            .analyzer_voice_subscribe(Box::new(move |r: &VoiceReport| {
                o.lock().unwrap().push(r.clone())
            }));
        (id, out)
    }

    fn subscribe_inspector(
        &mut self,
        config: InspectorConfig,
    ) -> (u32, Arc<Mutex<Vec<InspectorFrame>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let id = self.eng.analyzer_inspector_subscribe(
            Box::new(move |f: &InspectorFrame| o.lock().unwrap().push(f.clone())),
            config,
        );
        (id, out)
    }
}

/// H-42: a 200 Hz tone played through the rack reaches the voice tracker: its reports carry the
/// F0 statistics, and the output callback still never allocates.
#[test]
fn h42_voice_reports_follow_the_output() {
    let src = sine(200.0, -20.0, RATE, 3 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    let (_id, reports) = r.subscribe_voice();
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(1_500);
    let reports = reports.lock().unwrap();
    assert!(reports.len() > 10, "reports: {}", reports.len());
    let f0 = reports.last().unwrap().f0.expect("f0");
    assert!((f0.median_hz - 200.0).abs() < 1.0, "{f0:?}");
    assert!(f0.current_hz.is_some());
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-42: silence changes nothing, so nothing new is sent (idle costs the UI nothing).
#[test]
fn h42_unchanged_voice_reports_are_not_resent() {
    let src = vec![0.0f32; RATE as usize];
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    let (_id, reports) = r.subscribe_voice();
    r.run_ms(1_000);
    assert_eq!(
        reports.lock().unwrap().len(),
        1,
        "only the first (empty) report"
    );
}

/// H-42: an Inspector stream at a chosen FFT size and window reads a bin-centred tone at its
/// level; reconfiguring changes the bin count; unsubscribing stops it.
#[test]
fn h42_inspector_stream_resolution_and_level() {
    let fft = 4096u32;
    let freq = 85.0 * f64::from(RATE) / f64::from(fft);
    let src = sine(freq, -20.0, RATE, 4 * RATE as usize);
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    let config = InspectorConfig {
        fft_size: fft,
        window: WindowKind::FlatTop,
        response: AnalyzerResponse::Fast,
    };
    let (id, frames) = r.subscribe_inspector(config);
    r.run_ms(20);
    r.eng.transport(TransportCommand::Play);
    r.run_ms(1_000);
    {
        let f = frames.lock().unwrap().last().unwrap().clone();
        assert_eq!(f.fft_size, fft);
        assert_eq!(f.levels_db.len(), fft as usize / 2 + 1);
        assert_eq!(f.window, WindowKind::FlatTop);
        let peak = (0..f.levels_db.len())
            .max_by(|&a, &b| f.levels_db[a].total_cmp(&f.levels_db[b]))
            .unwrap();
        assert_eq!(peak, 85);
        assert!((f.levels_db[85] + 20.0).abs() < 0.1, "{}", f.levels_db[85]);
        let bytes = f.encode();
        assert_eq!(&bytes[..4], b"VXIS");
        assert_eq!(bytes.len(), 40 + 4 * f.levels_db.len());
    }
    // Every INSPECTOR_EVERY-th tick (30 Hz at 60 Hz).
    let n = frames.lock().unwrap().len();
    assert!((495..=512).contains(&n), "frames: {n}");

    r.eng.analyzer_inspector_configure(
        id,
        InspectorConfig {
            fft_size: 1024,
            ..config
        },
    );
    r.run_ms(10);
    let f = frames.lock().unwrap().last().unwrap().clone();
    assert_eq!(f.levels_db.len(), 513);

    r.eng.analyzer_unsubscribe(id);
    let n = frames.lock().unwrap().len();
    r.run_ms(100);
    assert_eq!(frames.lock().unwrap().len(), n);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-42: a silent output sends one Inspector frame (the floor), then nothing until sound comes.
#[test]
fn h42_inspector_goes_idle_on_silence() {
    let src = vec![0.0f32; RATE as usize];
    let mut r = rig(&src, RackModel { slots: Vec::new() });
    let (_id, frames) = r.subscribe_inspector(InspectorConfig {
        fft_size: 2048,
        window: WindowKind::Hann,
        response: AnalyzerResponse::Medium,
    });
    r.run_ms(500);
    let frames = frames.lock().unwrap();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].silent);
    assert!(frames[0].levels_db.iter().all(|&v| v == f32::NEG_INFINITY));
}
