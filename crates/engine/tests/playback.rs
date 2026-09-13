//! S1-01 playback end-to-end through the fake backend (SPEC-003 §2.1–§2.3, SPEC-001 §2.4,
//! ADR-002 §4–§8). Every output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{CallbackSizes, FakeBackend, FakeDevice, FakeDirection};
use vox_engine::devices::DeviceNotice;
use vox_engine::{
    DeviceKey, Direction, EngineConfig, EngineEvent, HostId, ManualEngine, PlaybackDoc,
    TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{
    TestDelay, ZIPPER_FREQ_HZ, ZIPPER_LEVEL_DBFS, ZipperWindow, alloc_checks_active,
    analyze_zipper, no_alloc,
};
use vox_module_api::{
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, Version,
};
use vox_modules::Gain;
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};

const MS: u64 = 1_000_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-{tag}-{}-{}",
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

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((x >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
        })
        .collect()
}

fn sine(freq_hz: f64, rate: u32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            (0.5 * (std::f64::consts::TAU * freq_hz * i as f64 / f64::from(rate)).sin()) as f32
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

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    events: Arc<Mutex<Vec<EngineEvent>>>,
    frames: Arc<Mutex<Vec<TelemetryFrame>>>,
    key: DeviceKey,
    make_dev: fn() -> FakeDirection,
    _dir: TempDir,
}

fn device(make_dev: fn() -> FakeDirection) -> FakeDevice {
    FakeDevice::new("DAC").with_output(make_dev().record_output())
}

fn rig(make_dev: fn() -> FakeDirection, samples: &[f32], doc_rate: u32, rack: RackModel) -> Rig {
    rig_with(make_dev, samples, doc_rate, rack, |_, _| {})
}

/// [`rig`] with `setup` run on the backend and the config before the engine starts.
fn rig_with(
    make_dev: fn() -> FakeDirection,
    samples: &[f32],
    doc_rate: u32,
    rack: RackModel,
    setup: impl FnOnce(&FakeBackend, &mut EngineConfig),
) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(42);
    fake.plug(HostId::Alsa, device(make_dev));
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });

    let dir = TempDir::new("doc");
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(256 << 20)).unwrap();
    let mut w = ChunkWriter::new(store.clone());
    w.append(samples).unwrap();
    let audio = w.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(doc_rate, audio.pieces, Vec::new()));

    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack;
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    setup(&fake, &mut cfg);
    let mut eng = ManualEngine::new(cfg);
    let fr = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f| fr.lock().unwrap().push(*f))));
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    Rig {
        fake,
        eng,
        events,
        frames,
        key: DeviceKey::new(HostId::Alsa, "DAC"),
        make_dev,
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

    fn output(&self) -> vox_engine::backend::fake::RecordedOutput {
        let id = self
            .fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Output)
            .map(|s| s.info.id)
            .expect("an output stream");
        self.fake.recorded_output(id).unwrap()
    }

    fn recorded(&self) -> Vec<f32> {
        self.output().samples
    }

    /// Output callbacks run so far on the latest output stream.
    fn blocks(&self) -> usize {
        self.output().blocks.len()
    }

    fn last_frame(&self) -> TelemetryFrame {
        *self.frames.lock().unwrap().last().unwrap()
    }

    fn cmd(&mut self, c: TransportCommand) -> vox_engine::TransportState {
        self.eng.transport(c)
    }
}

fn dev_random() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).callback_sizes(CallbackSizes::FULL_RANDOM)
}

fn dev_256() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn dev_256_latency() -> FakeDirection {
    dev_256().latency_ns(10 * MS)
}

/// Offset of `src[from..]` in `rec` (first match of a 64-sample window).
fn align(rec: &[f32], src: &[f32], from: usize) -> usize {
    (0..rec.len() - src.len())
        .find(|&k| (from..from + 64).all(|j| (rec[k + j] - src[j]).abs() <= 1e-6))
        .expect("source not found in the output")
}

/// Rising zero crossings → mean frequency.
fn frequency_hz(x: &[f32], rate: u32) -> f64 {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        let (a, b) = (f64::from(x[i - 1]), f64::from(x[i]));
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f64 + a / (a - b));
        }
    }
    let n = crossings.len() - 1;
    n as f64 * f64::from(rate) / (crossings[n] - crossings[0])
}

/// A known snapshot through [Gain 0 dB] reaches the device unchanged (≤ 1e-6 after alignment,
/// outside the 5 ms fade-in), with random callback sizes 1…4096, and no allocation in callbacks.
#[test]
fn plays_the_document_through_gain_unchanged() {
    let src = noise(1, 48_000);
    let mut r = rig(dev_random, &src, 48_000, gain_rack(0.0));
    r.run_ms(50);
    assert!(r.cmd(TransportCommand::Play).playing);
    r.run_ms(1_400);
    let st = r.eng.transport_state();
    assert!(!st.playing, "stops at the document end");
    assert_eq!(
        st.playhead_samples,
        src.len() as u64,
        "end = Pause semantics"
    );

    let rec = r.recorded();
    let fade = 240;
    let k = align(&rec, &src, fade);
    for i in fade..src.len() {
        assert!(
            (rec[k + i] - src[i]).abs() <= 1e-6,
            "sample {i}: {} vs {}",
            rec[k + i],
            src[i]
        );
    }
    assert!(
        rec[k + src.len()..].iter().all(|&x| x.to_bits() == 0),
        "silence after the end"
    );
    assert_eq!(r.fake.rt_violations(), 0, "the output callback allocated");
}

/// SPEC-003 §2.3: from the Play command to the first non-silent sample being heard < 50 ms.
#[test]
fn start_latency_is_under_50_ms() {
    let src = vec![0.5f32; 48_000];
    let mut r = rig(
        dev_256_latency,
        &src,
        48_000,
        RackModel { slots: Vec::new() },
    );
    r.run_ms(100);
    let t0 = r.fake.now_ns();
    r.cmd(TransportCommand::Play);
    r.run_ms(100);
    let rec = r.recorded();
    let first = rec.iter().position(|&x| x != 0.0).expect("audible output");
    // The stream opened at t = 0; frame f is heard at latency + f / rate.
    let heard_ns = 10 * MS + first as u64 * 1_000_000_000 / 48_000;
    let latency_ms = (heard_ns - t0) as f64 / 1e6;
    assert!(latency_ms < 50.0, "start latency {latency_ms} ms");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// D-018 / SPEC-003 §2.1: Stop returns to the play-start position; Pause keeps the heard
/// position and a resume continues from it.
#[test]
fn stop_returns_to_play_start_and_pause_keeps_position() {
    let src = noise(2, 3 * 48_000);
    let mut r = rig(dev_256, &src, 48_000, gain_rack(0.0));
    r.run_ms(20);
    r.cmd(TransportCommand::Seek(24_000));
    let st = r.cmd(TransportCommand::Play);
    assert!(st.playing);
    assert_eq!(st.play_start_samples, 24_000);
    r.run_ms(300);
    assert!(r.last_frame().rate > 0.0, "telemetry anchors while playing");
    let st = r.cmd(TransportCommand::Stop);
    assert!(!st.playing);
    assert_eq!(st.playhead_samples, 24_000);
    r.run_ms(50);
    assert_eq!(r.eng.transport_state().playhead_samples, 24_000);
    let f = r.last_frame();
    assert_eq!(
        (f.playhead_sample, f.rate.to_bits()),
        (24_000, 0f64.to_bits())
    );

    assert!(r.cmd(TransportCommand::Play).playing);
    r.run_ms(500);
    let paused = r.cmd(TransportCommand::Pause).playhead_samples;
    assert!(
        (24_000 + 19_200..=24_000 + 24_000).contains(&paused),
        "paused at {paused}"
    );
    r.run_ms(50);
    let p = r.eng.transport_state().playhead_samples;
    assert!(
        p.abs_diff(paused) < 1_000,
        "pause keeps the heard position: {p} vs {paused}"
    );
    assert_eq!(r.last_frame().playhead_sample, p);

    let st = r.cmd(TransportCommand::PlayPause);
    assert!(st.playing);
    assert_eq!(st.play_start_samples, p, "resume from the pause position");
    r.run_ms(200);
    r.cmd(TransportCommand::PlayPause);
    r.run_ms(50);
    assert!(r.eng.transport_state().playhead_samples > p + 7_200);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 §2.4: a 44.1 kHz document on a 48 kHz-only device is resampled in the reader: a
/// 1 kHz tone stays 1 kHz ± 0.05 %, and the heard position advances at the document rate.
#[test]
fn rate_mismatch_is_resampled_in_the_reader() {
    let src = sine(1000.0, 44_100, 2 * 44_100);
    let mut r = rig(dev_256, &src, 44_100, RackModel { slots: Vec::new() });
    assert_eq!(r.eng.devices().output_rate_hz, Some(48_000));
    r.run_ms(20);
    r.cmd(TransportCommand::Play);
    r.run_ms(1_500);
    let rec = r.recorded();
    let s = rec.iter().position(|&x| x != 0.0).unwrap();
    let f = frequency_hz(&rec[s + 4_800..s + 4_800 + 48_000], 48_000);
    assert!((f - 1000.0).abs() <= 0.5, "measured {f} Hz");

    let frames = r.frames.lock().unwrap();
    let playing: Vec<&TelemetryFrame> = frames.iter().filter(|f| f.rate > 0.0).collect();
    let (a, b) = (playing[10], playing[playing.len() - 10]);
    assert_eq!(a.rate.to_bits(), 44_100f64.to_bits());
    let slope = (b.playhead_sample - a.playhead_sample) as f64 * 1e9
        / (b.playhead_time_ns - a.playhead_time_ns) as f64;
    assert!(
        (slope - 44_100.0).abs() < 2.0,
        "heard position advances at {slope}/s"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-001 §2.4: losing the output device stops playback (Pause semantics) with a notice;
/// replugging reopens the stream without resuming playback.
#[test]
fn device_loss_stops_playback_with_a_notice() {
    let src = noise(3, 3 * 48_000);
    let mut r = rig(dev_256, &src, 48_000, gain_rack(0.0));
    r.run_ms(20);
    r.cmd(TransportCommand::Play);
    r.run_ms(500);
    let removed = r.fake.unplug(&r.key);
    assert!(removed.is_some());
    r.run_ms(20);
    let st = r.eng.transport_state();
    assert!(!st.playing);
    assert!(!st.can_play);
    assert!(
        st.playhead_samples > 19_200,
        "kept the heard position: {}",
        st.playhead_samples
    );
    let lost = r.events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Notice(DeviceNotice::DeviceLost {
                direction: Direction::Output,
                playback_stopped: true,
                ..
            })
        )
    });
    assert!(lost, "device-lost notice");

    r.eng.poll_devices(); // seen absent
    r.fake.plug(HostId::Alsa, device(r.make_dev));
    r.eng.poll_devices(); // back: reopen
    r.run_ms(20);
    let st = r.eng.transport_state();
    assert!(st.can_play, "stream reopened");
    assert!(!st.playing, "no auto-resume");
    let reconnected = r.events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Notice(DeviceNotice::DeviceReconnected { .. })
        )
    });
    assert!(reconnected);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// A scripted session (play, seeks, pause/resume, play from start, return to start, stop) with
/// random callback sizes: no allocation in any callback, no underrun, sane positions.
#[test]
fn scripted_session_is_allocation_free() {
    let src = noise(4, 4 * 48_000);
    let mut r = rig(dev_random, &src, 48_000, gain_rack(-6.0));
    r.run_ms(30);
    r.cmd(TransportCommand::Play);
    r.run_ms(300);
    r.cmd(TransportCommand::Seek(96_000));
    r.run_ms(200);
    let f = r.last_frame();
    assert!(
        f.playhead_sample >= 96_000,
        "seek target reached: {}",
        f.playhead_sample
    );
    r.cmd(TransportCommand::Pause);
    r.run_ms(100);
    r.cmd(TransportCommand::Play);
    r.run_ms(150);
    r.eng.set_selection(Some((48_000, 60_000)));
    r.cmd(TransportCommand::PlayFromStart);
    r.run_ms(150);
    r.cmd(TransportCommand::ReturnToStart);
    r.run_ms(150);
    let st = r.cmd(TransportCommand::Stop);
    assert_eq!(
        st.playhead_samples, 48_000,
        "Stop returns to where Play / Play from start was last pressed (not to a seek target)"
    );
    r.run_ms(50);
    assert!(!r.eng.transport_state().playing);
    assert_eq!(r.fake.rt_violations(), 0, "the output callback allocated");
    assert!(
        r.frames
            .lock()
            .unwrap()
            .iter()
            .all(|f| f.flags & vox_engine::telemetry::vxtm_flags::XRUN == 0),
        "no underrun"
    );
}

// --- H-04: post-merge review fixes -------------------------------------------------------------

/// The 5 ms transport fade at 48 kHz.
const FADE: usize = 240;

/// The SPEC-012 §4.3 tone (997 Hz, −20 dBFS peak, 48 kHz), `n` samples long.
fn tone(n: usize) -> Vec<f32> {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let w = std::f64::consts::TAU * ZIPPER_FREQ_HZ / 48_000.0;
    (0..n)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect()
}

/// Largest sample-to-sample step the faded tone can make: the tone's own slope plus one fade
/// step, and a little rounding margin.
fn max_tone_step() -> f32 {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let slope = amp * std::f64::consts::TAU * ZIPPER_FREQ_HZ / 48_000.0;
    (slope + amp / FADE as f64 + 1e-4) as f32
}

/// A transport transition (fade-out, silence, fade-in) of the tone at or after `from` in `rec`
/// passes SPEC-012 §4.3 with T_s = the 5 ms fade (the transition spans from one fade before the
/// silent gap to its end), and no sample-to-sample step exceeds the faded tone's.
fn assert_no_click(rec: &[f32], from: usize) {
    let z0 = (from..rec.len() - 1)
        .find(|&i| rec[i] == 0.0 && rec[i + 1] == 0.0)
        .expect("a silent gap");
    let z1 = (z0..rec.len())
        .find(|&i| rec[i] != 0.0)
        .expect("audio after the gap");
    let report = analyze_zipper(
        rec,
        0,
        48_000.0,
        ZipperWindow {
            start: z0 - FADE,
            end: z1,
            t_s_ms: 5.0,
        },
    )
    .expect("enough steady tone around the transition");
    assert!(report.pass, "§4.3 at the transition {z0}..{z1}: {report}");
    let (at, step) = rec
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, (w[1] - w[0]).abs()))
        .fold((0, 0.0f32), |a, b| if b.1 > a.1 { b } else { a });
    assert!(
        step <= max_tone_step(),
        "step {step} at {at} (gap {z0}..{z1}) above {}",
        max_tone_step()
    );
}

/// Latency of the rack-drain test module.
const DELAY: u32 = 480;

/// `TestDelay(480)` for the registry.
struct DelayFactory(ModuleDescriptor);

impl ModuleFactory for DelayFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestDelay::new(DELAY)))
    }
}

fn delay_rack() -> RackModel {
    RackModel {
        slots: vec![SlotModel::new(
            &ModuleRef {
                id: TestDelay::ID.into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState::new(1),
        )],
    }
}

/// Blocking 1: a seek in the last 5 ms before the document end. The END arrives during the seek's
/// fade-out; the queued seek must still start (not leave the transport "playing" with an idle
/// callback and a frozen playhead).
#[test]
fn seek_in_the_last_5_ms_still_starts() {
    // 3 callbacks of 256 frames + 100 frames: after 3 callbacks, 100 frames (< one fade) remain.
    let src = noise(6, 3 * 256 + 100);
    let mut r = rig(dev_256, &src, 48_000, gain_rack(0.0));
    r.run_ms(20);
    let n0 = r.blocks();
    assert!(r.cmd(TransportCommand::Play).playing);
    while r.blocks() < n0 + 3 {
        r.fake.advance_by(MS / 10);
        r.eng.tick();
    }
    assert!(r.eng.transport_state().playing);
    let at = r.recorded().len();
    assert!(r.cmd(TransportCommand::Seek(0)).playing);
    r.run_ms(100);
    let st = r.eng.transport_state();
    assert!(!st.playing, "the second pass ended");
    assert_eq!(
        st.playhead_samples,
        src.len() as u64,
        "end = Pause at the end"
    );
    let rec = r.recorded();
    let k = align(&rec[at..], &src, FADE);
    assert!(
        (FADE..src.len()).all(|i| (rec[at + k + i] - src[i]).abs() <= 1e-6),
        "the seek replays the document"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Blocking 1: Pause + Play within one callback quantum: the Play waits for the Pause's fade-out
/// (no cut to silence), then fades in.
#[test]
fn pause_and_play_within_one_quantum_do_not_click() {
    let src = tone(3 * 48_000);
    let mut r = rig(dev_256, &src, 48_000, RackModel { slots: Vec::new() });
    r.run_ms(20);
    r.cmd(TransportCommand::Play);
    r.run_ms(600);
    let at = r.recorded().len();
    assert!(!r.cmd(TransportCommand::Pause).playing);
    assert!(r.cmd(TransportCommand::Play).playing);
    r.run_ms(600);
    assert!(r.eng.transport_state().playing);
    assert_no_click(&r.recorded(), at);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Blocking 1: a Pause during a seek's fade-out reports the seek target (the transport's
/// position), not the position the fade-out played.
#[test]
fn stop_during_a_seek_fade_reports_the_seek_target() {
    let src = noise(7, 3 * 48_000);
    let mut r = rig(dev_256, &src, 48_000, gain_rack(0.0));
    r.run_ms(20);
    r.cmd(TransportCommand::Play);
    r.run_ms(300);
    r.cmd(TransportCommand::Seek(96_000));
    let st = r.cmd(TransportCommand::Pause);
    assert!(!st.playing);
    assert_eq!(st.playhead_samples, 96_000);
    r.run_ms(50);
    assert_eq!(
        r.eng.transport_state().playhead_samples,
        96_000,
        "the final stop position is the seek target"
    );
    let st = r.cmd(TransportCommand::Play);
    assert_eq!(st.play_start_samples, 96_000);
    r.run_ms(200);
    assert!(r.last_frame().playhead_sample > 96_000);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Blocking 2: a host switch closes the stream while playing → the transport pauses (heard
/// position kept) and Play works again on the new host.
#[test]
fn host_switch_while_playing_pauses_the_transport() {
    let src = noise(8, 3 * 48_000);
    let mut r = rig_with(dev_256, &src, 48_000, gain_rack(0.0), |fake, cfg| {
        // The saved host is unavailable at start: the default host (ALSA) is used.
        fake.add_host(HostId::PipeWire, false);
        fake.plug(HostId::PipeWire, device(dev_256));
        cfg.prefs.host = Some(HostId::PipeWire);
    });
    assert_eq!(r.eng.devices().host, Some(HostId::Alsa));
    r.run_ms(20);
    assert!(r.cmd(TransportCommand::Play).playing);
    r.run_ms(300);
    // The saved host comes back: the engine switches to it.
    r.fake.set_host_available(HostId::PipeWire, true);
    r.eng.poll_devices();
    assert_eq!(r.eng.devices().host, Some(HostId::PipeWire));
    let st = r.eng.transport_state();
    assert!(!st.playing, "the stream closed: Pause semantics");
    assert!(
        st.playhead_samples > 9_600,
        "kept the heard position: {}",
        st.playhead_samples
    );
    r.run_ms(20);
    assert!(r.eng.transport_state().can_play, "reopened on the new host");
    let st = r.cmd(TransportCommand::Play);
    assert!(st.playing, "Play works again");
    r.run_ms(200);
    assert!(
        r.recorded().iter().any(|&x| x != 0.0),
        "audible on the new host"
    );
    assert!(r.last_frame().playhead_sample > st.play_start_samples);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Blocking 3: a seek with a 480-sample-latency module in the rack. The rack reset waits until
/// the delayed fade-out has left the rack: no cut, §4.3 passes at the transition.
#[test]
fn seek_with_rack_latency_does_not_cut_the_fade_out() {
    let src = tone(5 * 48_000);
    let mut r = rig_with(dev_256, &src, 48_000, delay_rack(), |_, cfg| {
        let mut factories = vox_modules::builtin_factories();
        factories.push(Arc::new(DelayFactory(
            TestDelay::new(DELAY).descriptor().clone(),
        )));
        cfg.registry = Arc::new(Registry::with_factories(factories).unwrap());
    });
    r.run_ms(20);
    r.cmd(TransportCommand::Play);
    r.run_ms(600);
    let at = r.recorded().len();
    r.cmd(TransportCommand::Seek(100_003));
    r.run_ms(600);
    assert!(r.eng.transport_state().playing);
    let rec = r.recorded();
    assert_no_click(&rec, at);
    // The fade-out reached the device in full: 480 samples of tone (latency) then 240 faded.
    let z0 = (at..rec.len() - 1)
        .find(|&i| rec[i] == 0.0 && rec[i + 1] == 0.0)
        .unwrap();
    assert!(
        z0 >= at + DELAY as usize + FADE - 1,
        "fade-out cut at {z0} (command at {at})"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}
