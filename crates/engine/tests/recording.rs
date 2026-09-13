//! S1-04 recording end-to-end through the fake backend (SPEC-002 §2.1–§2.3, §2.6, §2.7 Off/Dry,
//! AC-16). Every input and output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use vox_engine::backend::fake::{CallbackSizes, FakeBackend, FakeDevice, FakeDirection};
use vox_engine::devices::DeviceNotice;
use vox_engine::record::{MonitorMode, RecordError, RecordState, RecordingResult, StopReason};
use vox_engine::telemetry::vxtm_flags;
use vox_engine::{
    DeviceKey, DevicePrefs, Direction, Engine, EngineConfig, EngineEvent, HostId, ManualEngine,
    PlaybackDoc, TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_project::{
    DocSnapshot, Session, SessionConfig, StoreOptions, TAKE_LABEL_KEY, TakeMode, TakeWriterOptions,
};
use vox_rack::Registry;

const MS: u64 = 1_000_000;
const RATE: u32 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-rec-{tag}-{}-{}",
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

/// Deterministic input: a different noise per channel, a pure function of the frame index.
fn src(frame: u64, ch: usize) -> f32 {
    let mut z = frame.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((ch as u64 + 1) << 56);
    z ^= z >> 31;
    z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 29;
    ((z >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.8
}

/// Channel 1 clips for 10 ms every 0.5 s (frames `b·24 000 .. b·24 000 + 480`).
fn clipping(frame: u64, ch: usize) -> f32 {
    if ch == 0 && frame % 24_000 < 480 {
        1.0
    } else {
        src(frame, ch) * 0.5
    }
}

fn mic_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "Mic")
}

fn dac_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "DAC")
}

fn mic(source: fn(u64, usize) -> f32) -> FakeDevice {
    FakeDevice::new("Mic").with_input(
        FakeDirection::new(2, &[44_100, 48_000], 48_000)
            .callback_sizes(CallbackSizes::FULL_RANDOM)
            .latency_ns(5 * MS)
            .source(move |f, c, _| source(f, c)),
    )
}

fn dac() -> FakeDevice {
    FakeDevice::new("DAC").with_output(
        FakeDirection::new(2, &[44_100, 48_000], 48_000)
            .default_buffer(256)
            .record_output(),
    )
}

type DoneSlot = Arc<Mutex<Option<RecordingResult>>>;

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    events: Arc<Mutex<Vec<EngineEvent>>>,
    frames: Arc<Mutex<Vec<TelemetryFrame>>>,
    done: DoneSlot,
    dir: TempDir,
    /// Fake time the input stream opened (its frame 0).
    t_open: u64,
}

/// A manual engine with a 2-channel "Mic" (recording `channel`, `None` = no input device) and,
/// optionally, a "DAC" output.
fn rig(source: fn(u64, usize) -> f32, with_output: bool, channel: Option<u16>) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(11);
    if with_output {
        fake.plug(HostId::Alsa, dac());
    }
    fake.plug(HostId::Alsa, mic(source));
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs = DevicePrefs {
        input_device: channel.map(|_| "Mic".to_owned()),
        input_channel: channel.unwrap_or(1),
        ..DevicePrefs::default()
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    let frames = Arc::new(Mutex::new(Vec::new()));
    let fr = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f| fr.lock().unwrap().push(*f))));
    eng.poll_devices();
    Rig {
        fake,
        eng,
        events,
        frames,
        done: Arc::new(Mutex::new(None)),
        dir: TempDir::new("take"),
        t_open: 0,
    }
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.fake.advance_by(MS);
            self.eng.tick();
        }
    }

    fn arm(&mut self) -> RecordState {
        self.t_open = self.fake.now_ns();
        let st = self.eng.set_armed(true);
        assert!(st.armed && st.input_open, "{st:?}");
        assert_eq!(st.input_rate_hz, Some(RATE));
        st
    }

    fn session(&self) -> Session {
        Session::create(
            &self.dir.0,
            SessionConfig {
                sample_rate_hz: RATE,
                source: None,
                store: StoreOptions::with_memory_budget(64 << 20),
            },
        )
        .unwrap()
    }

    /// Begins a take and starts recording; returns the fake time of the Record command.
    fn start(&mut self, session: &mut Session) -> u64 {
        let capture = session
            .begin_take(TakeMode::New, TakeWriterOptions::default())
            .unwrap();
        let t = self.fake.now_ns();
        let done = self.done.clone();
        let st = self
            .eng
            .record_start(
                RATE,
                capture,
                Box::new(move |r| *done.lock().unwrap() = Some(r)),
            )
            .expect("record_start");
        assert!(st.recording && st.armed, "{st:?}");
        t
    }

    /// Stops; returns the fake time of the Stop command.
    fn stop(&mut self) -> u64 {
        let t = self.fake.now_ns();
        let st = self.eng.record_stop();
        assert!(!st.recording, "{st:?}");
        t
    }

    /// Runs until the capture-writer finished the take.
    fn result(&mut self) -> RecordingResult {
        for _ in 0..2_000 {
            if let Some(r) = self.done.lock().unwrap().take() {
                return r;
            }
            self.run_ms(1);
        }
        panic!("the take was not finished");
    }

    /// Input frame index of the first sample captured at or after fake time `t_ns`.
    fn frame_at(&self, t_ns: u64) -> u64 {
        (u128::from(t_ns - self.t_open) * u128::from(RATE)).div_ceil(1_000_000_000) as u64
    }

    fn last_frame(&self) -> TelemetryFrame {
        *self.frames.lock().unwrap().last().unwrap()
    }

    fn output(&self) -> Vec<f32> {
        let id = self
            .fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Output)
            .map(|s| s.info.id)
            .expect("an output stream");
        self.fake.recorded_output(id).unwrap().samples
    }

    fn notices(&self) -> Vec<DeviceNotice> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Notice(n) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }
}

fn take_samples(session: &Session, snap: &DocSnapshot) -> Vec<f32> {
    let mut buf = vec![0.0; snap.len_samples as usize];
    let n = session.store().read(snap, 0, &mut buf).unwrap();
    assert_eq!(n, buf.len());
    buf
}

/// The source frame the take starts at: the whole take must be a bit-exact run of channel `ch`
/// starting within ±2 frames of `expected`.
fn locate(take: &[f32], ch: usize, expected: u64) -> u64 {
    for k in expected.saturating_sub(2)..=expected + 2 {
        if take
            .iter()
            .enumerate()
            .all(|(i, &x)| x.to_bits() == src(k + i as u64, ch).to_bits())
        {
            return k;
        }
    }
    panic!("the take is not a bit-exact run of the source near frame {expected}");
}

/// SPEC-002 §2.2, §4.2: the take is the selected input channel, bit-exact, covering capture times
/// `[t_record, t_stop)` to ±1 sample, committed as one undoable "Record" edit that undo removes
/// and redo restores; no allocation in any callback (random callback sizes 1…4096).
#[test]
fn records_the_selected_channel_bit_exact_and_undo_removes_the_take() {
    let mut r = rig(src, true, Some(2));
    r.run_ms(20);
    r.arm();
    r.run_ms(100);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(700);
    let f = r.last_frame();
    assert!(f.flags & vxtm_flags::RECORDING != 0, "RECORDING flag");
    assert!(f.in_peak_dbfs.is_finite() && f.in_rms_dbfs.is_finite());
    assert!(f.playhead_sample > 0, "the recording anchor advances");
    let t_stop = r.stop();
    let res = r.result();
    assert_eq!(res.reason, StopReason::User);
    assert!(res.finished.error.is_none(), "{:?}", res.finished.error);
    assert!(res.write_error.is_none());
    assert_eq!((res.overflow_samples, res.sample_rate_hz), (0, RATE));
    let st = r.eng.record_state();
    assert!(!st.recording && !st.finishing && st.armed, "{st:?}");

    let step = session
        .commit_take(&res.finished, &[])
        .unwrap()
        .expect("one undoable edit");
    assert_eq!(&*step.label_key, TAKE_LABEL_KEY);
    let take = take_samples(&session, &step.snapshot);
    let (k0, k1) = (r.frame_at(t_rec), r.frame_at(t_stop));
    let k = locate(&take, 1, k0);
    assert!(
        k.abs_diff(k0) <= 1,
        "take starts at frame {k}, expected {k0}"
    );
    let end = k + take.len() as u64;
    assert!(
        end.abs_diff(k1) <= 1,
        "take ends at frame {end}, expected {k1}"
    );
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");

    // The take becomes the playable document.
    r.eng.set_document(Some(PlaybackDoc {
        store: session.store().clone(),
        snapshot: step.snapshot.clone(),
    }));
    let ts = r.eng.transport_state();
    assert!(ts.can_play);
    assert_eq!(ts.doc_len_samples, take.len() as u64);

    // Undo returns to the empty document; redo restores the take exactly.
    let undone = session.undo().unwrap().expect("undo");
    assert_eq!(undone.snapshot.len_samples, 0);
    assert!(undone.snapshot.markers.is_empty());
    let redone = session.redo().unwrap().expect("redo");
    assert_eq!(take_samples(&session, &redone.snapshot), take);
}

/// SPEC-002 §2.1: |x| ≥ 0.99990 raises `IN_CLIP`; clip runs are counted per take; with Dry
/// monitoring on, still no allocation in the input or output callback.
#[test]
fn clips_are_flagged_and_counted() {
    let mut r = rig(clipping, true, Some(1));
    r.eng.set_monitor_mode(MonitorMode::Dry);
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(1_300);
    let t_stop = r.stop();
    let res = r.result();
    let (k0, k1) = (r.frame_at(t_rec), r.frame_at(t_stop));
    // Bursts b cover frames [b·24 000, b·24 000 + 480); count those overlapping the take.
    let expected = (0..=k1 / 24_000)
        .filter(|b| {
            let (s, e) = (b * 24_000, b * 24_000 + 480);
            s < k1 && e > k0
        })
        .count() as u32;
    assert!(expected >= 2);
    assert_eq!(res.clip_events, expected);
    let flagged = r
        .frames
        .lock()
        .unwrap()
        .iter()
        .any(|f| f.flags & vxtm_flags::IN_CLIP != 0);
    assert!(flagged, "IN_CLIP telemetry flag");
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// SPEC-002 §2.7 (Dry): the input is heard after the rack at unity gain only while armed; Off is
/// silent; disarming fades out.
#[test]
fn dry_monitoring_is_heard_only_while_armed() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    // Off: arming alone is silent.
    r.arm();
    r.run_ms(200);
    assert!(r.output().iter().all(|&x| x == 0.0), "Off is silent");
    assert!(!r.eng.record_state().monitoring);

    // Dry while armed: the output carries the input channel bit-exact (after the fade-in).
    let st = r.eng.set_monitor_mode(MonitorMode::Dry);
    assert!(st.monitoring, "{st:?}");
    assert!(r.last_frame().flags & vxtm_flags::MONITORING == 0 || st.monitoring);
    r.run_ms(300);
    assert!(r.last_frame().flags & vxtm_flags::MONITORING != 0);
    let out = r.output();
    let n = out.len();
    let win = &out[n - 4_800..n - 4_736];
    let span = r.frame_at(r.fake.now_ns()) + 4_800;
    let found = (0..span).any(|k| {
        win.iter()
            .enumerate()
            .all(|(i, &x)| x.to_bits() == src(k + i as u64, 0).to_bits())
    });
    assert!(
        found,
        "the monitored output is the input channel at unity gain"
    );

    // Disarm: monitoring stops (after a ≤ 10 ms fade).
    let st = r.eng.set_armed(false);
    assert!(!st.armed && !st.input_open && !st.monitoring, "{st:?}");
    let before = r.output().len();
    r.run_ms(100);
    let out = r.output();
    assert!(
        out[before + 960..].iter().all(|&x| x == 0.0),
        "silent after disarming"
    );
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// SPEC-002 §2.6: losing the input device stops the recording and keeps the take.
#[test]
fn input_loss_stops_and_keeps_the_take() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    r.start(&mut session);
    r.run_ms(400);
    assert!(r.fake.unplug(&mic_key()).is_some());
    let res = r.result();
    assert_eq!(res.reason, StopReason::InputLost);
    // Kept up to the last delivered block (callbacks of up to 4096 frames + 5 ms latency).
    let len = res.finished.audio.len_samples;
    assert!((12_000..=19_300).contains(&len), "kept {len} samples");
    let step = session.commit_take(&res.finished, &[]).unwrap();
    assert!(step.is_some(), "the take is committed");
    let st = r.eng.record_state();
    assert!(!st.recording && !st.finishing && !st.input_open, "{st:?}");
    let lost = r.notices().iter().any(|n| {
        matches!(
            n,
            DeviceNotice::DeviceLost {
                direction: Direction::Input,
                recording_stopped: true,
                ..
            }
        )
    });
    assert!(lost, "input device-lost notice");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-002 AC-16 / D-017: losing only the output keeps the recording going; D-020: recording
/// needs no output device.
#[test]
fn output_loss_keeps_recording() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(300);
    assert!(r.fake.unplug(&dac_key()).is_some());
    r.run_ms(300);
    assert!(r.eng.record_state().recording, "still recording");
    let t_stop = r.stop();
    let res = r.result();
    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    let k0 = r.frame_at(t_rec);
    let k = locate(&take, 0, k0);
    assert!((k + take.len() as u64).abs_diff(r.frame_at(t_stop)) <= 1);
    let continues = r.notices().iter().any(|n| {
        matches!(
            n,
            DeviceNotice::DeviceLost {
                direction: Direction::Output,
                recording_continues: true,
                ..
            }
        )
    });
    assert!(
        continues,
        "output device-lost notice says recording continues"
    );

    // Without any output device at all, recording still works.
    let mut r = rig(src, false, Some(1));
    r.run_ms(20);
    r.arm();
    let mut session = r.session();
    r.start(&mut session);
    r.run_ms(200);
    r.stop();
    let res = r.result();
    assert!(res.finished.audio.len_samples > 9_000);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Refusals and transport interplay: no input device, rate mismatch, double start; Space stops.
#[test]
fn refusals_and_space_stops_the_recording() {
    // No input device: arming opens nothing and Record is refused.
    let mut r = rig(src, true, None);
    r.run_ms(20);
    let st = r.eng.set_armed(true);
    assert!(!st.input_open && st.input_device.is_none(), "{st:?}");
    let mut session = r.session();
    let cap = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let id = cap.id();
    let err = r
        .eng
        .record_start(RATE, cap, Box::new(|_| {}))
        .expect_err("no input device");
    assert_eq!(err, RecordError::NoInputDevice);
    session.discard_take(id).unwrap();

    // The input opens at the preferred record rate; a document at another rate is refused.
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    let mut session = r.session();
    let cap = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let id = cap.id();
    let err = r
        .eng
        .record_start(44_100, cap, Box::new(|_| {}))
        .expect_err("rate mismatch");
    assert_eq!(
        err,
        RecordError::RateMismatch {
            input_hz: 48_000,
            doc_hz: 44_100
        }
    );
    session.discard_take(id).unwrap();
    r.eng.set_record_rate(44_100);
    r.eng.set_armed(false);
    let st = r.eng.set_armed(true);
    assert_eq!(st.input_rate_hz, Some(44_100));
    r.eng.set_armed(false);
    r.eng.set_record_rate(RATE);
    r.arm();

    // Double start is refused; Space (play/pause) stops the recording; disarm is refused while
    // recording.
    r.start(&mut session);
    r.run_ms(100);
    let cap2 = Session::create(&r.dir.0, SessionConfig::new(RATE))
        .unwrap()
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    assert_eq!(
        r.eng.record_start(RATE, cap2, Box::new(|_| {})).err(),
        Some(RecordError::AlreadyRecording)
    );
    assert!(
        r.eng.set_armed(false).armed,
        "armed is locked while recording"
    );
    r.eng.transport(TransportCommand::PlayPause);
    assert!(!r.eng.record_state().recording, "Space stops the recording");
    let res = r.result();
    assert!(res.finished.audio.len_samples > 4_000);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// The threaded engine: real capture-writer and sync threads; the done callback runs on the
/// writer thread and may call the engine handle (no deadlock); the take is bit-exact.
#[test]
fn threaded_engine_records_through_the_writer_thread() {
    assert!(alloc_checks_active());
    let fake = FakeBackend::new(5);
    fake.plug(HostId::Alsa, dac());
    fake.plug(HostId::Alsa, mic(src));
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs = DevicePrefs {
        input_device: Some("Mic".into()),
        input_channel: 1,
        ..DevicePrefs::default()
    };
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let engine = Engine::start(cfg).unwrap();
    let h = engine.handle();
    let driver = fake.spawn_driver(Duration::from_millis(1));

    let mut opened = false;
    for _ in 0..300 {
        if h.set_armed(true).is_some_and(|s| s.input_open) {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(opened, "the input stream opened");

    let dir = TempDir::new("threaded");
    let mut session = Session::create(&dir.0, SessionConfig::new(RATE)).unwrap();
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let h2 = h.clone();
    h.record_start(
        RATE,
        capture,
        Box::new(move |res| {
            // The done callback may call the engine (it runs on the writer thread).
            let _ = h2.transport_state();
            let _ = tx.send(res);
        }),
    )
    .expect("record_start");
    std::thread::sleep(Duration::from_millis(400));
    assert!(h.record_state().is_some_and(|s| s.recording));
    h.record_stop();
    let res = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the take was finished");
    assert!(res.finished.error.is_none());
    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    assert!(take.len() > 9_600, "recorded {} samples", take.len());
    // Contiguous and bit-exact: find where it starts in the source.
    let limit = u64::from(RATE) * 30;
    let k = (0..limit)
        .find(|&k| {
            take[..64]
                .iter()
                .enumerate()
                .all(|(i, &x)| x.to_bits() == src(k + i as u64, 0).to_bits())
        })
        .expect("the take start is in the source");
    assert_eq!(locate(&take, 0, k), k);
    drop(driver);
    drop(engine);
    assert_eq!(fake.rt_violations(), 0, "a callback allocated");
}
