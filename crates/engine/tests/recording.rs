//! S1-04 recording end-to-end through the fake backend (SPEC-002 §2.1–§2.3, §2.6, §2.7 Off/Dry,
//! AC-16). Every input and output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use vox_engine::backend::fake::{
    CallbackSizes, FakeBackend, FakeDevice, FakeDirection, FakeEvent, Signal,
};
use vox_engine::devices::DeviceNotice;
use vox_engine::record::{
    DISK_FLOOR_BYTES, DropoutMark, LIVE_PEAKS_SPB, MonitorMode, RECORD_BYTES_PER_SAMPLE,
    RecordError, RecordState, RecordingResult, StopReason,
};
use vox_engine::telemetry::vxtm_flags;
use vox_engine::{
    DeviceKey, DevicePrefs, Direction, Engine, EngineConfig, EngineEvent, HostId, ManualEngine,
    PlaybackDoc, TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_project::gc::{SessionClass, classify_session};
use vox_project::session::TAKES_DIR_NAME;
use vox_project::take::{read_take_samples, recover_take, take_part_path};
use vox_project::{
    DocSnapshot, FixedFreeSpace, Session, SessionConfig, StoreOptions, TAKE_LABEL_KEY, TakeMode,
    TakeWriterOptions,
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

/// H-06: a mono mic that only ever offers 48 kHz (unlike [`mic`]'s 44.1/48 kHz pair), carrying a
/// steady tone at `freq_hz`/`amplitude` — for AC-4-style "input rate != document rate" tests,
/// where the capture-writer must resample.
fn mic_48k_only_tone(freq_hz: f64, amplitude: f32) -> FakeDevice {
    FakeDevice::new("Mic").with_input(
        FakeDirection::new(1, &[48_000], 48_000)
            .callback_sizes(CallbackSizes::FULL_RANDOM)
            .latency_ns(5 * MS)
            .signals(vec![Signal::Sine { freq_hz, amplitude }]),
    )
}

/// H-10 item 4: like [`mic`], but fixed-size callbacks — a controlled, small period estimate so
/// an injected [`FakeEvent::InputDropout`] reliably clears the period-based dropout threshold
/// (SPEC-002 §4.3), unlike [`mic`]'s `FULL_RANDOM` sizes (up to 4096 frames, sometimes bigger
/// than the gap itself).
fn mic_fixed(source: fn(u64, usize) -> f32, frames: u32) -> FakeDevice {
    FakeDevice::new("Mic").with_input(
        FakeDirection::new(2, &[44_100, 48_000], 48_000)
            .callback_sizes(CallbackSizes::Fixed(frames))
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
    /// H-11: the injected free-space provider (`EngineConfig::disk_space`), defaulting to a
    /// generous value so unrelated tests never trip the disk floor by accident.
    disk: FixedFreeSpace,
}

/// H-11: a generous default so tests that don't care about disk space never trip the floor.
const PLENTY_OF_DISK: u64 = 100 << 30;

/// A manual engine with a 2-channel "Mic" (recording `channel`, `None` = no input device) and,
/// optionally, a "DAC" output.
fn rig(source: fn(u64, usize) -> f32, with_output: bool, channel: Option<u16>) -> Rig {
    rig_with_mic(mic(source), with_output, channel)
}

/// [`rig`], but with a caller-supplied mic device (H-06: a mic offering a different rate than
/// [`mic`]'s 44.1/48 kHz pair).
fn rig_with_mic(mic: FakeDevice, with_output: bool, channel: Option<u16>) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(11);
    if with_output {
        fake.plug(HostId::Alsa, dac());
    }
    fake.plug(HostId::Alsa, mic);
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
    let dir = TempDir::new("take");
    let disk = FixedFreeSpace::new(PLENTY_OF_DISK);
    cfg.disk_space = Arc::new(disk.clone());
    cfg.record_volume = dir.0.clone();
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
        dir,
        t_open: 0,
        disk,
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

/// Mean frequency from rising zero crossings (linearly interpolated) — H-06's resampling test.
fn frequency_hz(x: &[f32], rate_hz: u32) -> f64 {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        let (a, b) = (f64::from(x[i - 1]), f64::from(x[i]));
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f64 + a / (a - b));
        }
    }
    let n = crossings.len() - 1;
    let span = crossings[n] - crossings[0];
    n as f64 * f64::from(rate_hz) / span
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

/// H-10 item 4 (SPEC-002 §2.4/§4.3, AC-7): a genuine input dropout — the fake backend really
/// skips source frames, `FakeEvent::InputDropout` — continues the take, fills the gap with
/// silence, keeps every later sample at its correct (post-gap) source position, and reports
/// exactly one `DropoutMark`; the live `dropout_count` reflects it before Stop. No allocation in
/// any callback.
#[test]
fn input_dropout_is_filled_with_silence_and_reported() {
    let mut r = rig_with_mic(mic_fixed(src, 64), false, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(200);

    let input_id = r
        .fake
        .streams()
        .iter()
        .rev()
        .find(|s| s.info.direction == Direction::Input)
        .map(|s| s.info.id)
        .expect("an input stream");
    // A 10 ms dropout (480 frames @ 48 kHz); 64-frame callbacks (≈1.33 ms) keep the period-based
    // threshold (≈0.67 ms) far below it.
    r.fake
        .schedule(r.fake.now_ns(), FakeEvent::InputDropout(input_id, 480));
    r.run_ms(200);

    // Live before Stop (SPEC-002 §2.1's amber counter): `record_state` already reports it.
    let live = r.eng.record_state();
    assert_eq!(live.dropout_count, 1, "{live:?}");

    let t_stop = r.stop();
    let res = r.result();
    assert_eq!(res.reason, StopReason::User);
    assert!(res.finished.error.is_none(), "{:?}", res.finished.error);
    assert_eq!(res.dropouts.len(), 1, "{:?}", res.dropouts);
    assert_eq!(res.dropouts[0].len_samples, 480);

    let step = session
        .commit_take(&res.finished, &[])
        .unwrap()
        .expect("one undoable edit");
    let take = take_samples(&session, &step.snapshot);
    let pos = res.dropouts[0].pos_samples as usize;
    let len = res.dropouts[0].len_samples as usize;
    assert_eq!(
        res.dropouts[0],
        DropoutMark {
            pos_samples: res.dropouts[0].pos_samples,
            len_samples: 480,
        }
    );

    // Before the gap: a bit-exact run of the source (channel 0 — `input_channel: 1`).
    let (k0, k1) = (r.frame_at(t_rec), r.frame_at(t_stop));
    let before_start = locate(&take[..pos], 0, k0);
    // The gap itself is digital silence.
    assert!(
        take[pos..pos + len].iter().all(|&x| x == 0.0),
        "the dropout must be filled with exact silence"
    );
    // After the gap: the source resumes 480 frames further on (the frames the dropout skipped),
    // not where an uninterrupted recording would have been — later audio stays at its spoken
    // (post-gap) position, SPEC-002 §2.4.
    let after_start = locate(&take[pos + len..], 0, before_start + pos as u64 + 480);
    assert_eq!(after_start, before_start + pos as u64 + 480);
    // The fake dropout reduces the *real* samples the ring actually receives over [t_rec, t_stop)
    // by ~480 (a genuine loss, unlike a mere timestamp fudge); the inserted silence restores the
    // take to its expected real-time length, so it still covers ≈ [t_rec, t_stop) (SPEC-002 §2.4:
    // "later audio stays where it was spoken").
    assert!(
        (take.len() as u64).abs_diff(k1 - k0) <= 2,
        "the take must still cover the real recording span: len={}, k1-k0={}",
        take.len(),
        k1 - k0
    );
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// A 997 Hz tone at −20 dBFS peak on channel 1 (channel 2 silent).
fn tone_997(frame: u64, ch: usize) -> f32 {
    if ch == 0 {
        (0.1 * (std::f64::consts::TAU * 997.0 * frame as f64 / 48_000.0).sin()) as f32
    } else {
        0.0
    }
}

/// SPEC-002 §2.7 (Dry): the input is heard after the rack at unity gain only while armed; Off is
/// silent; disarming fades out. T-107: the monitor path always runs through the drift-corrected
/// resampler (ADR-002 §6 — not bit-transparent), so unity gain is judged by level: the −20 dBFS
/// peak tone comes out at −23.01 dB RMS.
#[test]
fn dry_monitoring_is_heard_only_while_armed() {
    let mut r = rig_with_mic(mic_fixed(tone_997, 256), true, Some(1));
    r.run_ms(20);
    // Off: arming alone is silent.
    r.arm();
    r.run_ms(200);
    assert!(r.output().iter().all(|&x| x == 0.0), "Off is silent");
    assert!(!r.eng.record_state().monitoring);

    // Dry while armed: the output carries the input channel at unity gain (after the fade-in).
    let st = r.eng.set_monitor_mode(MonitorMode::Dry);
    assert!(st.monitoring, "{st:?}");
    r.run_ms(300);
    assert!(r.last_frame().flags & vxtm_flags::MONITORING != 0);
    let out = r.output();
    let win = &out[out.len() - 9_600..];
    let rms_db =
        10.0 * (win.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / win.len() as f64).log10();
    assert!(
        (rms_db + 23.01).abs() < 0.05,
        "the monitored output is the input channel at unity gain: {rms_db} dB"
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

    // The input opens at the preferred record rate; a document at another rate now resamples
    // instead of being refused (H-06 — see `resamples_when_the_input_rate_differs_from_the_document_rate`
    // for the full frequency/level/length checks).
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    let mut session = r.session();
    {
        let cap = session
            .begin_take(TakeMode::New, TakeWriterOptions::default())
            .unwrap();
        let resampled_id = cap.id();
        let done: DoneSlot = Arc::new(Mutex::new(None));
        let done2 = done.clone();
        let st = r
            .eng
            .record_start(
                44_100,
                cap,
                Box::new(move |res| *done2.lock().unwrap() = Some(res)),
            )
            .expect("H-06: a rate mismatch resamples instead of being refused");
        assert!(st.recording, "{st:?}");
        r.run_ms(50);
        r.eng.record_stop();
        let mut resampled = None;
        for _ in 0..2_000 {
            if let Some(res) = done.lock().unwrap().take() {
                resampled = Some(res);
                break;
            }
            r.run_ms(1);
        }
        let resampled = resampled.expect("the resampled take finished");
        assert_eq!(
            resampled.sample_rate_hz, 44_100,
            "the take is at the document rate"
        );
        // Discarded rather than committed: `session.current()` stays empty for the checks below
        // (the take was still finished — its data written — so `discard_take` closes it out the
        // same way a refused take does).
        session.discard_take(resampled_id).unwrap();
    }

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

/// H-06 (SPEC-002 §2.2, ticket H-06 test plan): a fake input that only ever offers 48 kHz records
/// a 1 kHz, -20 dBFS tone into a 44.1 kHz document. The capture-writer resamples on its own
/// thread (never the RT input callback — no allocation there either way): the tone keeps its
/// frequency within 0.05 % and its level within 0.1 dB, and the take length matches wall time
/// within one processing block (the resampler's own priming delay is compensated, so the take's
/// start isn't shifted).
#[test]
fn resamples_when_the_input_rate_differs_from_the_document_rate() {
    const DOC_RATE: u32 = 44_100;
    const SECS: f64 = 3.0;
    let mic = mic_48k_only_tone(1_000.0, 10f32.powf(-20.0 / 20.0));
    let mut r = rig_with_mic(mic, false, Some(1));
    r.run_ms(20);
    let st = r.eng.set_armed(true);
    assert!(st.armed && st.input_open, "{st:?}");
    assert_eq!(
        st.input_rate_hz,
        Some(48_000),
        "the only rate this fake mic offers"
    );
    r.run_ms(50);

    let mut session = Session::create(
        &r.dir.0,
        SessionConfig {
            sample_rate_hz: DOC_RATE,
            source: None,
            store: StoreOptions::with_memory_budget(64 << 20),
        },
    )
    .unwrap();
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let done = r.done.clone();
    let st = r
        .eng
        .record_start(
            DOC_RATE,
            capture,
            Box::new(move |res| *done.lock().unwrap() = Some(res)),
        )
        .expect("a rate mismatch resamples instead of being refused (H-06)");
    assert!(st.recording, "{st:?}");
    r.run_ms((SECS * 1000.0) as u64);
    r.eng.record_stop();
    let res = r.result();
    assert_eq!(res.reason, StopReason::User);
    assert!(res.finished.error.is_none(), "{:?}", res.finished.error);
    assert!(res.write_error.is_none());
    assert_eq!(
        res.sample_rate_hz, DOC_RATE,
        "the take is at the document rate, not the input's"
    );

    let step = session
        .commit_take(&res.finished, &[])
        .unwrap()
        .expect("one undoable edit");
    let take = take_samples(&session, &step.snapshot);

    // Length matches wall time within one processing block (`CaptureResampler::CHUNK_IN` device
    // frames converted to document-rate frames).
    let expected_len = (SECS * f64::from(DOC_RATE)).round() as usize;
    let block = (1024.0 * f64::from(DOC_RATE) / 48_000.0).ceil() as usize;
    assert!(
        take.len().abs_diff(expected_len) <= block,
        "got {}, expected {expected_len} +/- {block}",
        take.len()
    );

    // The tone keeps its frequency and level, measured on the back half and clear of the very
    // end: `finish()`'s zero-padded flush of the last partial chunk isn't real signal, so the
    // last `block`-ish samples are excluded (same reasoning as the length tolerance above).
    let tail = &take[take.len() / 2..take.len().saturating_sub(block)];
    let f = frequency_hz(tail, DOC_RATE);
    let rel_err = (f - 1_000.0).abs() / 1_000.0;
    assert!(
        rel_err <= 0.0005,
        "measured {f} Hz ({}% off)",
        rel_err * 100.0
    );
    let peak = tail.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let level_dbfs = 20.0 * f64::from(peak).log10();
    assert!(
        (level_dbfs - (-20.0)).abs() <= 0.1,
        "level {level_dbfs} dBFS"
    );

    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
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

/// H-07: `EngineHandle::live_take_peaks` mirrors the take's growing content — `None` before a
/// take starts and once it is finished, then `(min, max)` buckets of exactly the samples appended
/// so far, computed off the capture-writer thread (never the RT input callback: the `rt_violations`
/// assertions elsewhere in this file already guard that; this test only checks correctness).
#[test]
fn live_take_peaks_matches_the_appended_samples() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    assert!(r.eng.live_take_peaks(0, 8).is_none(), "no take yet");
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    r.start(&mut session);
    r.run_ms(300);

    let snap = r
        .eng
        .live_take_peaks(0, 1_000_000)
        .expect("a take is being captured");
    assert_eq!(snap.sample_rate_hz, RATE);
    assert_eq!(snap.spb, LIVE_PEAKS_SPB);
    assert!(snap.len_samples > 0);
    // Paging: the first two buckets of a wider request equal a narrower one starting at 0.
    let page = r.eng.live_take_peaks(0, 2).unwrap();
    assert_eq!(&page.buckets[..], &snap.buckets[..2]);

    r.stop();
    let res = r.result();
    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);

    let spb = snap.spb as usize;
    let full_buckets = (snap.len_samples as usize) / spb;
    assert!(
        full_buckets >= 4,
        "the take ran long enough for several full buckets"
    );
    for (b, &(mn, mx)) in snap.buckets.iter().take(full_buckets).enumerate() {
        let (want_mn, want_mx) = take[b * spb..(b + 1) * spb]
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &s| {
                (mn.min(s), mx.max(s))
            });
        assert_eq!((mn, mx), (want_mn, want_mx), "bucket {b}");
    }

    // Once the take is finished, there is no recording to query anymore.
    assert!(r.eng.live_take_peaks(0, 8).is_none());
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// H-05 (SPEC-002 §2.4, AC-8): when the capture-writer falls a whole ring (10 s) behind, the
/// recording stops by itself and the take is kept as a contiguous, bit-exact run of the source up
/// to the overflow — never a take with the lost samples silently spliced out.
#[test]
fn capture_ring_overflow_stops_and_keeps_the_take() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(100);
    // Stall the writer: fake time runs 11 s without a control tick (the manual engine's inline
    // writer drains only in `tick`), so the input callback overflows the 10 s capture ring.
    r.fake.advance_by(11_000 * MS);
    r.run_ms(2);
    let res = r
        .done
        .lock()
        .unwrap()
        .take()
        .expect("the take finished by itself, without Stop");
    let st = r.eng.record_state();
    assert!(!st.recording && !st.finishing, "{st:?}");
    assert_eq!(res.reason, StopReason::Overflow);
    assert!(res.overflow_samples > 0);
    assert!(res.write_error.is_none() && res.finished.error.is_none());

    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    let ring = u64::from(RATE) * 10;
    let len = take.len() as u64;
    assert!(
        (ring..=ring + u64::from(RATE) / 5).contains(&len),
        "kept {len} samples: the ~100 ms drained before the stall + one full ring"
    );
    let k = locate(&take, 0, r.frame_at(t_rec));
    assert!(k.abs_diff(r.frame_at(t_rec)) <= 1);
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// H-05 (SPEC-002 §2.5, ADR-004 §7.4): when appending to the take fails mid-take, the recording
/// stops by itself (it used to keep "recording" and drop everything until the user pressed Stop)
/// and the take is kept up to the last good sample.
#[test]
fn write_error_stops_the_recording_and_keeps_the_take() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    // Parts of 0.5 s; the second part's file already exists, so the rollover fails.
    let takes = session.takes_dir();
    std::fs::create_dir_all(&takes).unwrap();
    std::fs::write(take_part_path(&takes, 1, 1), b"").unwrap();
    let capture = session
        .begin_take(
            TakeMode::New,
            TakeWriterOptions {
                max_data_bytes: u64::from(RATE) * 2,
                ..TakeWriterOptions::default()
            },
        )
        .unwrap();
    assert_eq!(capture.id().0, 1);
    let t_rec = r.fake.now_ns();
    let done = r.done.clone();
    r.eng
        .record_start(
            RATE,
            capture,
            Box::new(move |res| *done.lock().unwrap() = Some(res)),
        )
        .unwrap();
    r.run_ms(1_000);
    let res = r
        .done
        .lock()
        .unwrap()
        .take()
        .expect("the take finished by itself, without Stop");
    let st = r.eng.record_state();
    assert!(!st.recording && !st.finishing && st.armed, "{st:?}");
    assert_eq!(res.reason, StopReason::WriteError);
    assert!(res.write_error.is_some());
    assert_eq!(res.finished.wav_samples, u64::from(RATE) / 2);

    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    assert!(!take.is_empty() && take.len() as u64 <= u64::from(RATE) / 2);
    let k = locate(&take, 0, r.frame_at(t_rec));
    assert!(k.abs_diff(r.frame_at(t_rec)) <= 1);

    // The input is free again: the next take records normally.
    r.start(&mut session);
    r.run_ms(200);
    r.stop();
    let res = r.result();
    assert_eq!(res.reason, StopReason::User);
    assert!(res.write_error.is_none() && res.finished.audio.len_samples > 9_000);
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// H-11 (SPEC-002 §2.5, AC-13): the remaining-time formula while idle/armed, and the injected
/// free-space provider dropping below the hard floor mid-take stops the recording gracefully and
/// keeps the take, like a write error.
#[test]
fn disk_floor_stops_and_keeps_the_take() {
    let mut r = rig(src, true, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);

    // Remaining time before any take: (free - floor) / (8 * rate), at the armed input's rate.
    let want = (PLENTY_OF_DISK - DISK_FLOOR_BYTES) / (RECORD_BYTES_PER_SAMPLE * u64::from(RATE));
    let st = r.eng.record_state();
    assert_eq!(st.disk_remaining_s, Some(want), "{st:?}");

    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(100);
    // Free space drops below the 512 MiB floor.
    r.disk.set(300 << 20);
    // The next ~1 s poll (SPEC-002 §2.5) picks it up and stops within 1 s (AC-13).
    r.run_ms(1_000);
    let res = r
        .done
        .lock()
        .unwrap()
        .take()
        .expect("the take finished by itself, without Stop");
    let st = r.eng.record_state();
    assert!(!st.recording && !st.finishing, "{st:?}");
    assert_eq!(res.reason, StopReason::DiskFull);
    assert!(res.finished.error.is_none(), "{:?}", res.finished.error);

    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    assert!(!take.is_empty());
    let k = locate(&take, 0, r.frame_at(t_rec));
    assert!(k.abs_diff(r.frame_at(t_rec)) <= 1, "bit-exact from t_rec");
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// H-11 item 4 (SPEC-002 §2.4 second row, A-011): an input gap over the 2 s fill cap is handled
/// like device loss end-to-end — the recording stops, the take is kept up to the gap, and it is
/// *not* padded with seconds of silence (unlike a fillable dropout, H-10).
#[test]
fn input_gap_over_two_seconds_is_device_loss_not_a_fillable_dropout() {
    let mut r = rig_with_mic(mic_fixed(src, 64), false, Some(1));
    r.run_ms(20);
    r.arm();
    r.run_ms(50);
    let mut session = r.session();
    let t_rec = r.start(&mut session);
    r.run_ms(200);

    let input_id = r
        .fake
        .streams()
        .iter()
        .rev()
        .find(|s| s.info.direction == Direction::Input)
        .map(|s| s.info.id)
        .expect("an input stream");
    // A 2.1 s gap: past the fill cap, so it cuts over to device loss instead of being filled.
    // The fake backend "loses" wall-clock time along with the skipped frames (`Slot::reschedule`
    // computes the next callback's time from the jumped `frame_pos`), so the callback carrying
    // the gap doesn't arrive until fake time has advanced by roughly the gap's own duration too.
    r.fake.schedule(
        r.fake.now_ns(),
        FakeEvent::InputDropout(input_id, u64::from(RATE) * 21 / 10),
    );
    r.run_ms(2_200);

    let res = r
        .done
        .lock()
        .unwrap()
        .take()
        .expect("the take finished by itself, like device loss");
    assert_eq!(res.reason, StopReason::InputLost);
    assert!(
        res.dropouts.is_empty(),
        "not a fillable dropout: {:?}",
        res.dropouts
    );
    assert_eq!(r.eng.record_state().dropout_count, 0);

    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    // Kept up to the gap (≈200 ms + the run before the schedule took effect), nowhere near the
    // ~2.1 s that a fill would have added.
    assert!(
        take.len() < (u64::from(RATE) as usize),
        "kept only up to the gap, got {} samples",
        take.len()
    );
    let k = locate(&take, 0, r.frame_at(t_rec));
    assert!(k.abs_diff(r.frame_at(t_rec)) <= 1);
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// H-11 item 5 (SPEC-002 §2.4/§4.3): a dropout on the resampled capture path (H-06, input rate ≠
/// document rate) is still filled with silence and marked, converted to document-rate samples by
/// the same resampler real audio goes through — not silently dropped as H-10 left it.
#[test]
fn resampled_take_still_fills_and_marks_dropouts() {
    const DOC_RATE: u32 = 44_100;
    let mut r = rig_with_mic(mic_fixed(src, 64), false, Some(1));
    r.run_ms(20);
    let st = r.eng.set_armed(true);
    assert!(st.armed && st.input_open, "{st:?}");
    assert_eq!(st.input_rate_hz, Some(48_000));
    r.run_ms(50);

    let mut session = Session::create(
        &r.dir.0,
        SessionConfig {
            sample_rate_hz: DOC_RATE,
            source: None,
            store: StoreOptions::with_memory_budget(64 << 20),
        },
    )
    .unwrap();
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let done = r.done.clone();
    r.eng
        .record_start(
            DOC_RATE,
            capture,
            Box::new(move |res| *done.lock().unwrap() = Some(res)),
        )
        .expect("a rate mismatch resamples instead of being refused (H-06)");
    r.run_ms(200);

    let input_id = r
        .fake
        .streams()
        .iter()
        .rev()
        .find(|s| s.info.direction == Direction::Input)
        .map(|s| s.info.id)
        .expect("an input stream");
    // A 20 ms dropout at the input's 48 kHz rate.
    r.fake
        .schedule(r.fake.now_ns(), FakeEvent::InputDropout(input_id, 960));
    r.run_ms(200);

    r.eng.record_stop();
    let res = r.result();
    assert_eq!(res.reason, StopReason::User);
    assert!(res.finished.error.is_none(), "{:?}", res.finished.error);
    assert_eq!(
        res.dropouts.len(),
        1,
        "the resampled path must still splice and mark it: {:?}",
        res.dropouts
    );
    // 20 ms of input silence resamples to ≈20 ms of document-rate silence; the resampler
    // processes in fixed 1024-input-frame chunks, so allow one chunk's worth of slack (H-06's own
    // resampling test uses the same tolerance style).
    let want_len = (0.020 * f64::from(DOC_RATE)).round() as u64;
    let block = (1024.0 * f64::from(DOC_RATE) / 48_000.0).ceil() as u64;
    assert!(
        res.dropouts[0].len_samples.abs_diff(want_len) <= block,
        "got {}, expected {want_len} +/- {block}",
        res.dropouts[0].len_samples
    );

    let step = session.commit_take(&res.finished, &[]).unwrap().unwrap();
    let take = take_samples(&session, &step.snapshot);
    let pos = res.dropouts[0].pos_samples as usize;
    let len = res.dropouts[0].len_samples as usize;
    assert!(pos + len <= take.len());
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

/// Set for the child half of [`kill_9_mid_take_recovers_every_appended_sample`]: the session
/// directory to record into. Unset, [`kill9_child_records_until_killed`] does nothing.
const KILL9_CHILD_DIR: &str = "VOX_ENGINE_KILL9_CHILD_DIR";

/// Kills (SIGKILL) and reaps the child on every exit path.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Child half of [`kill_9_mid_take_recovers_every_appended_sample`] (a no-op in a normal run):
/// records through the threaded engine and prints `KILL9 <appended> <captured>` every few ms —
/// samples the capture-writer handed to the take (`live_take_peaks`) and samples the input
/// callback captured (telemetry) — until the parent kills it.
#[test]
fn kill9_child_records_until_killed() {
    let Some(dir) = std::env::var_os(KILL9_CHILD_DIR) else {
        return;
    };
    let fake = FakeBackend::new(7);
    fake.plug(HostId::Alsa, mic(src)); // no output device needed (D-020)
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
    let _driver = fake.spawn_driver(Duration::from_millis(1));
    let captured = Arc::new(AtomicU64::new(0));
    let c = captured.clone();
    h.set_telemetry_sink(Some(Box::new(move |f: &TelemetryFrame| {
        if f.flags & vxtm_flags::RECORDING != 0 {
            c.store(f.playhead_sample, Ordering::Relaxed);
        }
    })));
    let opened = (0..300).any(|_| {
        let open = h.set_armed(true).is_some_and(|s| s.input_open);
        if !open {
            std::thread::sleep(Duration::from_millis(10));
        }
        open
    });
    assert!(opened, "the input stream opened");
    let mut session = Session::create(Path::new(&dir), SessionConfig::new(RATE)).unwrap();
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    h.record_start(RATE, capture, Box::new(|_| {}))
        .expect("record_start");
    let mut out = std::io::stdout().lock();
    loop {
        std::thread::sleep(Duration::from_millis(2));
        let appended = h.live_take_peaks(0, 0).map_or(0, |p| p.len_samples);
        let captured = captured.load(Ordering::Relaxed);
        writeln!(out, "KILL9 {appended} {captured}").unwrap();
        out.flush().unwrap();
    }
}

/// H-05 / SPEC-002 §2.3, AC-6: `kill -9` mid-take. A child process records through the threaded
/// engine (real capture-writer + sync threads) and is killed with SIGKILL once ≥ 1.5 s is in the
/// take (past at least one header patch + `fdatasync`). The session is then recoverable, and its
/// take WAV holds every sample the writer had appended (nothing is buffered in user space), within
/// 250 ms of what the input had captured, bit-identical to the source.
#[test]
fn kill_9_mid_take_recovers_every_appended_sample() {
    if std::env::var_os(KILL9_CHILD_DIR).is_some() {
        return;
    }
    let dir = TempDir::new("kill9");
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "kill9_child_records_until_killed",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(KILL9_CHILD_DIR, &dir.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = BufReader::new(child.0.stdout.take().unwrap());
    let target = u64::from(RATE) * 3 / 2;
    let mut last = None;
    for line in stdout.lines() {
        let line = line.unwrap();
        let Some(rest) = line.strip_prefix("KILL9 ") else {
            continue;
        };
        let v: Vec<u64> = rest.split(' ').map(|x| x.parse().unwrap()).collect();
        if v[0] >= target {
            last = Some((v[0], v[1]));
            break;
        }
    }
    child.0.kill().unwrap(); // SIGKILL: no destructor, no final patch or sync
    child.0.wait().unwrap();
    let (appended, captured) = last.expect("the child recorded 1.5 s");

    let sessions: Vec<PathBuf> = std::fs::read_dir(&dir.0)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    match classify_session(&sessions[0]).unwrap() {
        SessionClass::Recoverable(s) => {
            assert_eq!(s.open_take, Some(1));
            assert!(s.open_take_samples >= appended);
        }
        SessionClass::Clean => panic!("a killed take must be offered for recovery"),
    }
    let parts = recover_take(&sessions[0].join(TAKES_DIR_NAME), 1).unwrap();
    assert_eq!(parts.len(), 1);
    let len = parts[0].samples;
    assert!(len >= appended, "recovered {len} < {appended} appended");
    assert!(
        len + u64::from(RATE) / 4 >= captured,
        "recovered {len}, but {captured} were captured (> 250 ms lost)"
    );
    let mut take = vec![0.0f32; len as usize];
    assert_eq!(
        read_take_samples(&parts[0], 0, &mut take).unwrap(),
        take.len()
    );
    let k = (0..u64::from(RATE) * 30)
        .find(|&k| {
            take[..64]
                .iter()
                .enumerate()
                .all(|(i, &x)| x.to_bits() == src(k + i as u64, 0).to_bits())
        })
        .expect("the take start is in the source");
    assert_eq!(locate(&take, 0, k), k, "bit-identical to the source");
}

/// A 2-channel mic carrying `signals` per channel, for input-meter tests (SPEC-002 AC-1).
fn mic_signals(signals: Vec<Signal>) -> FakeDevice {
    FakeDevice::new("Mic").with_input(
        FakeDirection::new(2, &[48_000], 48_000)
            .callback_sizes(CallbackSizes::FULL_RANDOM)
            .latency_ns(5 * MS)
            .signals(signals),
    )
}

// `-inf` is exact, so comparing against it is not the approximate-equality mistake the lint is
// about (the same reason `telemetry.rs`'s own meter tests do it).
#[allow(clippy::float_cmp)]
/// SPEC-002 AC-1: the input meter's accuracy as the UI actually receives it — in the `VXTM`
/// frames, not just in `InputMeter`'s own unit test. A 997 Hz sine at −12.00 dBFS peak on the
/// selected channel reads −12.00 ± 0.05 dB peak / −15.01 ± 0.05 dB RMS in every frame after the
/// first 300 ms; digital silence reads `-inf` for both (JSON `null`).
#[test]
fn ac1_input_meter_reports_peak_and_rms_within_tolerance() {
    // −12 dBFS peak.
    let amplitude = 10f32.powf(-12.0 / 20.0);
    let mut r = rig_with_mic(
        mic_signals(vec![
            Signal::Silence,
            Signal::Sine {
                freq_hz: 997.0,
                amplitude,
            },
        ]),
        false,
        Some(2),
    );
    r.run_ms(20);
    r.arm();
    r.run_ms(1000);

    let frames = r.frames.lock().unwrap().clone();
    // One control tick per millisecond of fake time in this rig, so frames past 300 are past the
    // meter's settling window.
    let settled: Vec<_> = frames.iter().skip(320).collect();
    assert!(settled.len() > 100, "{} frames", settled.len());
    for f in &settled {
        assert!(
            (f.in_peak_dbfs - (-12.00)).abs() <= 0.05,
            "peak {} dBFS",
            f.in_peak_dbfs
        );
        assert!(
            (f.in_rms_dbfs - (-15.01)).abs() <= 0.05,
            "rms {} dBFS",
            f.in_rms_dbfs
        );
    }
    assert_eq!(r.fake.rt_violations(), 0);

    // The unselected channel carries the tone, so selecting channel 1 must read digital silence.
    let mut quiet = rig_with_mic(
        mic_signals(vec![
            Signal::Silence,
            Signal::Sine {
                freq_hz: 997.0,
                amplitude,
            },
        ]),
        false,
        Some(1),
    );
    quiet.run_ms(20);
    quiet.arm();
    quiet.run_ms(500);
    let f = quiet.last_frame();
    assert!(
        f.in_peak_dbfs == f32::NEG_INFINITY,
        "peak on a silent channel: {}",
        f.in_peak_dbfs
    );
    assert!(
        f.in_rms_dbfs == f32::NEG_INFINITY,
        "rms on a silent channel: {}",
        f.in_rms_dbfs
    );
    assert_eq!(quiet.fake.rt_violations(), 0);
}
