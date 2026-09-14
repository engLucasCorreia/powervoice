//! T-304 (SPEC-022): record operations and latency calibration end-to-end through the fake
//! backend — sample-exact alignment of a punch with a known (hidden) latency and offset, free-start
//! Insert, what the talent hears, stopping in each phase, phase events, and a loopback calibration
//! run. Every input and output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{CallbackSizes, FakeBackend, FakeDevice, FakeDirection, Loopback};
use vox_engine::record::{
    CalibrationError, CalibrationStatus, CancelReason, MonitorMode, RecordPhase, RecordPhaseInfo,
    RecordingResult,
};
use vox_engine::record_op::{CursorRecordMode, RecordOpKind, RecordPlan, RecordPrefs};
use vox_engine::{
    DeviceKey, DevicePrefs, Direction, EngineConfig, EngineEvent, HostId, ManualEngine,
    PlaybackDoc, TransportCommand,
};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_project::{
    FixedFreeSpace, HistoryStep, PUNCH_LABEL_KEY, Session, SessionConfig, StoreOptions, TakeMode,
    TakeParams, TakeWriterOptions,
};
use vox_rack::Registry;
use vox_testkit::prng::Pcg32;

const MS: u64 = 1_000_000;
const RATE: u32 = 48_000;
/// A 10 s document (the §5 fixture's punch range, shorter tail).
const L: usize = 480_000;
const S: u64 = 240_000;
const E: u64 = 384_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-punch-{tag}-{}-{}",
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
    let mut rng = Pcg32::new(seed, 7);
    (0..n).map(|_| (rng.next_signed() * 0.3) as f32).collect()
}

/// Deterministic mic input: a pure function of the input frame index.
fn src(frame: u64) -> f32 {
    let mut z = frame.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (1 << 56);
    z ^= z >> 31;
    z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 29;
    ((z >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.8
}

fn dac_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "DAC")
}

fn mic_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "Mic")
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    phases: Arc<Mutex<Vec<(RecordPhaseInfo, u64)>>>,
    windows: Arc<Mutex<Vec<(u32, u64)>>>,
    done: Arc<Mutex<Option<RecordingResult>>>,
    session: Session,
    a: Vec<f32>,
    /// Fake time the input stream opened (its frame 0).
    t_open: u64,
    _dir: TempDir,
}

/// The §5 fake backend: one clock, reported input latency 5 ms and output latency 7 ms, random
/// input callback sizes; a 10 s noise document; empty rack; monitoring Off.
fn rig(mic_source: Option<fn(u64) -> f32>, loopback: Option<Loopback>) -> Rig {
    assert!(alloc_checks_active());
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(
            FakeDirection::new(2, &[48_000], 48_000)
                .default_buffer(256)
                .latency_ns(7 * MS)
                .record_output(),
        ),
    );
    let mut mic = FakeDirection::new(1, &[48_000], 48_000)
        .callback_sizes(CallbackSizes::Random { min: 32, max: 512 })
        .latency_ns(5 * MS);
    if let Some(s) = mic_source {
        mic = mic.source(move |f, _, _| s(f));
    }
    fake.plug(HostId::Alsa, FakeDevice::new("Mic").with_input(mic));
    fake.set_loopback(loopback);
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs = DevicePrefs {
        input_device: Some("Mic".to_owned()),
        input_channel: 1,
        ..DevicePrefs::default()
    };
    let phases = Arc::new(Mutex::new(Vec::new()));
    let windows = Arc::new(Mutex::new(Vec::new()));
    let (ph, win, clock) = (phases.clone(), windows.clone(), fake.clone());
    cfg.events = Arc::new(move |e| match e {
        EngineEvent::RecordPhase(info) => ph.lock().unwrap().push((info, clock.now_ns())),
        EngineEvent::RecordWindow { take, k_start } => win.lock().unwrap().push((take, k_start)),
        _ => {}
    });
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let dir = TempDir::new("rig");
    cfg.disk_space = Arc::new(FixedFreeSpace::new(100 << 30));
    cfg.record_volume = dir.0.clone();
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    let mut session = Session::create(
        &dir.0,
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: StoreOptions::with_memory_budget(128 << 20),
        },
    )
    .unwrap();
    let a = noise(1, L);
    let mut writer = session.chunk_writer();
    writer.append(&a).unwrap();
    let audio = writer.finish().unwrap();
    session.set_floor(&audio, Vec::new()).unwrap();
    eng.set_document(Some(PlaybackDoc {
        store: session.store().clone(),
        snapshot: session.current(),
    }));
    Rig {
        fake,
        eng,
        phases,
        windows,
        done: Arc::new(Mutex::new(None)),
        session,
        a,
        t_open: 0,
        _dir: dir,
    }
}

fn prefs() -> RecordPrefs {
    RecordPrefs {
        preroll_s: 1.0,
        postroll_s: 0.5,
        ..RecordPrefs::default()
    }
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.fake.advance_by(MS);
            self.eng.tick();
            // The app's job (DocumentService): journal each opened window.
            let opened: Vec<(u32, u64)> = std::mem::take(&mut *self.windows.lock().unwrap());
            for (take, k) in opened {
                self.session
                    .note_take_window(vox_project::TakeId(take), k)
                    .unwrap();
            }
        }
    }

    fn arm(&mut self) {
        self.t_open = self.fake.now_ns();
        let st = self.eng.set_armed(true);
        assert!(st.input_open, "{st:?}");
    }

    /// Record pressed with `selection`: resolve, begin the matching take, start.
    fn start(&mut self, selection: Option<(u64, u64)>, prefs: RecordPrefs) -> RecordPlan {
        let plan = self.eng.record_prepare(selection, prefs).unwrap();
        let mode = match plan.kind {
            RecordOpKind::Punch => TakeMode::Punch {
                start_samples: plan.at_samples,
                end_samples: plan.end_samples.unwrap(),
            },
            RecordOpKind::Insert => TakeMode::Insert {
                at_samples: plan.at_samples,
            },
            RecordOpKind::Overwrite => TakeMode::Overwrite {
                at_samples: plan.at_samples,
            },
            RecordOpKind::New => TakeMode::New,
        };
        let capture = self
            .session
            .begin_take_with(
                mode,
                TakeParams {
                    xfade_samples: plan.xfade_samples,
                    offset_ns: plan.offset_ns,
                    aligned: plan.aligned,
                },
                TakeWriterOptions::default(),
            )
            .unwrap();
        let done = self.done.clone();
        self.eng
            .record_start_op(
                plan,
                capture,
                Box::new(move |r| *done.lock().unwrap() = Some(r)),
            )
            .unwrap();
        plan
    }

    fn result(&mut self) -> RecordingResult {
        for _ in 0..20_000 {
            if let Some(r) = self.done.lock().unwrap().take() {
                return r;
            }
            self.run_ms(1);
        }
        panic!("the operation did not finish");
    }

    /// What the app does with the result: commit the window, or cancel.
    fn commit(&mut self, r: &RecordingResult) -> Option<HistoryStep> {
        let op = r.op.expect("an operation result");
        match op.window {
            Some(window) => self
                .session
                .commit_take_window(&r.finished, window, &[])
                .unwrap(),
            None => {
                self.session.cancel_take(r.finished.take).unwrap();
                None
            }
        }
    }

    fn doc(&self) -> Vec<f32> {
        let snap = self.session.current();
        let mut out = vec![0.0; snap.len_samples as usize];
        self.session.store().read(&snap, 0, &mut out).unwrap();
        out
    }

    fn output_stream(&self) -> vox_engine::backend::StreamId {
        self.fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Output)
            .map(|s| s.info.id)
            .unwrap()
    }

    fn output(&self) -> Vec<f32> {
        self.fake
            .recorded_output(self.output_stream())
            .unwrap()
            .samples
    }

    /// Output frame index where document position `q` (in an unfaded, audible stretch) is
    /// heard — located by an exact match of 64 samples.
    fn heard_frame(&self, out: &[f32], q: usize) -> usize {
        let pat = &self.a[q..q + 64];
        out.windows(64)
            .position(|w| w.iter().zip(pat).all(|(x, y)| x.to_bits() == y.to_bits()))
            .expect("document position not heard")
    }
}

fn assert_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    if let Some(i) = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!("{what}: sample {i} differs: {} vs {}", got[i], want[i]);
    }
}

/// AC-6: a 0 dB loopback with Hear original lands the punched interior on `A` bit-exactly; an
/// unreported 144-sample (3 ms) residual with δ = 0 lands it late by exactly 144; δ = +3 ms
/// compensates it bit-exactly. AC-5/AC-9 bits: outside `[S, E)` bit-identical, `L′ = L`, the
/// take holds the whole pass, one "Punch-in" entry. AC-21: no callback allocates.
#[test]
fn loopback_punch_lands_sample_exactly_after_compensation() {
    for (residual, offset_ms, shift) in [(0i64, 0.0, 0usize), (144, 0.0, 144), (144, 3.0, 0)] {
        let mut lb = Loopback::new(dac_key(), mic_key());
        lb.residual_ns = residual * 1_000_000_000 / i64::from(RATE);
        let mut r = rig(None, Some(lb));
        r.run_ms(50);
        r.arm();
        r.run_ms(100);
        let plan = r.start(
            Some((S, E)),
            RecordPrefs {
                hear_original: true,
                offset_ms,
                ..prefs()
            },
        );
        assert_eq!(plan.kind, RecordOpKind::Punch);
        assert!(plan.aligned && plan.hear_original);
        let res = r.result();
        let op = res.op.unwrap();
        assert_eq!(op.cancelled, None);
        let (k0, k1) = op.window.unwrap();
        assert_eq!(k1 - k0, E - S);
        let pass = res.finished.wav_samples;
        let min_pass = u64::from(RATE) * 3 / 2 + (E - S); // pre 1 s + window + post 0.5 s
        assert!(
            pass >= min_pass && pass <= min_pass + u64::from(RATE) / 10,
            "the take holds the whole pass: {pass} samples"
        );
        let step = r.commit(&res).expect("one edit");
        assert_eq!(&*step.label_key, PUNCH_LABEL_KEY);
        let out = r.doc();
        assert_eq!(out.len(), L);
        let (s, e) = (S as usize, E as usize);
        assert_bits(&out[..s], &r.a[..s], "A′[0, S)");
        assert_bits(&out[e..], &r.a[e..], "A′[E, L)");
        assert_bits(
            &out[s + 480..e - 480],
            &r.a[s + 480 - shift..e - 480 - shift],
            &format!("interior, residual {residual}, δ {offset_ms} ms"),
        );
        assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
    }
}

/// AC-2 (engine side): Insert at the cursor with a free start: the take is the input captured
/// from the Record command to Stop, bit-exact, spliced in at `c`; `L′ = L + n`.
#[test]
fn insert_at_the_cursor_splices_the_captured_input() {
    let mut r = rig(Some(src), None);
    r.run_ms(20);
    r.arm();
    r.run_ms(100);
    let c = 120_000u64;
    r.eng.transport(TransportCommand::Seek(c));
    let t0 = r.fake.now_ns();
    let plan = r.start(
        None,
        RecordPrefs {
            mode: CursorRecordMode::Insert,
            ..prefs()
        },
    );
    assert_eq!(
        (plan.kind, plan.at_samples, plan.aligned),
        (RecordOpKind::Insert, c, false)
    );
    r.run_ms(700);
    let t1 = r.fake.now_ns();
    r.eng.transport(TransportCommand::PlayPause); // Space stops the take
    let res = r.result();
    let op = res.op.unwrap();
    assert_eq!(op.cancelled, None);
    let n = res.finished.audio.len_samples;
    let expected = (u128::from(t1 - t0) * u128::from(RATE) / 1_000_000_000) as u64;
    assert!(n.abs_diff(expected) <= 1, "n {n}, expected {expected}");
    r.commit(&res).unwrap();
    let out = r.doc();
    assert_eq!(out.len() as u64, L as u64 + n);
    let c = c as usize;
    assert_bits(&out[..c], &r.a[..c], "A[0, c)");
    assert_bits(&out[c + n as usize..], &r.a[c..], "A[c, L)");
    let k0 = (u128::from(t0 - r.t_open) * u128::from(RATE)).div_ceil(1_000_000_000) as u64;
    let w = &out[c..c + n as usize];
    let found = (k0.saturating_sub(2)..=k0 + 2).find(|&k| {
        w.iter()
            .enumerate()
            .all(|(i, &x)| x.to_bits() == src(k + i as u64).to_bits())
    });
    assert!(
        found.is_some(),
        "W is a bit-exact run of the input near frame {k0}"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// AC-8: what the talent hears during a punch (Hear original off, no rack latency): `A[P₀, S)`
/// with a linear 5 ms fade-out ending at `S`, silence over the window, `A[E, E + post)` with a
/// 5 ms fade-in and the stop fade, then silence. Phase events arrive in order, `recording` and
/// `postroll` within one telemetry frame (≤ 17 ms) of the heard times of `S` and `E`.
#[test]
fn the_talent_hears_pre_roll_silence_and_post_roll() {
    let mut r = rig(None, None);
    r.run_ms(50);
    r.arm();
    r.run_ms(100);
    r.start(
        Some((S, E)),
        RecordPrefs {
            preroll_s: 2.0,
            postroll_s: 1.0,
            ..prefs()
        },
    );
    let res = r.result();
    r.commit(&res).unwrap();
    let out = r.output();
    let (s, e) = (S as usize, E as usize);
    let i_s = r.heard_frame(&out, s - 1_000) + 1_000;
    for j in 0..240 {
        let want = r.a[s - 240 + j] * ((240 - j) as f32 / 240.0);
        assert!((out[i_s - 240 + j] - want).abs() <= 1e-6, "fade-out {j}");
    }
    assert!(
        out[i_s..i_s + (e - s)].iter().all(|&x| x == 0.0),
        "the window is silent"
    );
    let i_e = i_s + (e - s);
    for j in 0..48_000 {
        let q = e + j;
        let mut g = 1.0f32;
        if j < 240 {
            g *= (j + 1) as f32 / 240.0;
        }
        if j >= 48_000 - 240 {
            g *= (48_000 - j) as f32 / 240.0;
        }
        assert!((out[i_e + j] - r.a[q] * g).abs() <= 1e-6, "post-roll {j}");
    }
    assert!(
        out[i_e + 48_000..].iter().all(|&x| x == 0.0),
        "silence after the run"
    );

    let phases = r.phases.lock().unwrap().clone();
    let kinds: Vec<RecordPhase> = phases.iter().map(|(p, _)| p.phase).collect();
    assert_eq!(
        kinds,
        [
            RecordPhase::PreRoll,
            RecordPhase::Recording,
            RecordPhase::PostRoll,
            RecordPhase::Committing
        ]
    );
    let out_id = r.output_stream();
    let heard_s = r.fake.frame_time_ns(out_id, i_s as u64).unwrap();
    let heard_e = r.fake.frame_time_ns(out_id, i_e as u64).unwrap();
    let (rec, post) = (&phases[1], &phases[2]);
    assert_eq!((rec.0.doc_pos_samples, post.0.doc_pos_samples), (S, E));
    assert!(
        rec.1 >= heard_s && rec.1 - heard_s <= 17 * MS,
        "recording event late"
    );
    assert!(
        post.1 >= heard_e && post.1 - heard_e <= 17 * MS,
        "post-roll event late"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// AC-9: Stop during pre-roll cancels (no edit, `rev` and undo depth unchanged); Stop during
/// the window keeps a partial punch ending at the heard position `p` (`A[p, L)` untouched);
/// Stop during post-roll equals an uninterrupted run.
#[test]
fn stopping_in_each_phase() {
    // Pre-roll: cancelled.
    let mut r = rig(None, None);
    r.run_ms(50);
    r.arm();
    r.run_ms(100);
    let rev = r.session.current().rev;
    r.start(Some((S, E)), prefs());
    r.run_ms(300);
    r.eng.transport(TransportCommand::Stop);
    let res = r.result();
    let op = res.op.unwrap();
    assert_eq!((op.window, op.cancelled), (None, Some(CancelReason::User)));
    assert!(r.commit(&res).is_none());
    assert_eq!(r.session.current().rev, rev);
    assert_eq!(r.session.history().undo_depth(), 0);

    // Recording: partial punch to the heard stop point (loopback + Hear original → A there).
    let mut r = rig(None, Some(Loopback::new(dac_key(), mic_key())));
    r.run_ms(50);
    r.arm();
    r.run_ms(100);
    r.start(
        Some((S, E)),
        RecordPrefs {
            hear_original: true,
            ..prefs()
        },
    );
    r.run_ms(2_050); // ≈ 1 s into the 3 s window
    r.eng.transport(TransportCommand::Stop);
    let res = r.result();
    let op = res.op.unwrap();
    let (k0, k1) = op.window.expect("a partial punch");
    let p = (S + (k1 - k0)) as usize;
    assert!(
        (p as u64) > S + 30_000 && (p as u64) < S + 70_000,
        "stopped about 1 s into the window: p = {p}"
    );
    let committing = r
        .phases
        .lock()
        .unwrap()
        .iter()
        .find(|(i, _)| i.phase == RecordPhase::Committing)
        .map(|(i, _)| i.doc_pos_samples);
    assert_eq!(committing, Some(p as u64), "the committing event carries p");
    r.commit(&res).unwrap();
    let out = r.doc();
    assert_eq!(out.len(), L);
    let s = S as usize;
    assert_bits(&out[p..], &r.a[p..], "A′[p, L)");
    assert_bits(&out[s + 480..p - 480], &r.a[s + 480..p - 480], "interior");

    // Post-roll: identical to letting it run out.
    let mut hashes = Vec::new();
    for stop_in_post_roll in [false, true] {
        let mut r = rig(None, Some(Loopback::new(dac_key(), mic_key())));
        r.run_ms(50);
        r.arm();
        r.run_ms(100);
        r.start(
            Some((S, E)),
            RecordPrefs {
                hear_original: true,
                ..prefs()
            },
        );
        if stop_in_post_roll {
            r.run_ms(4_200); // pre 1 s + window 3 s, into the 0.5 s post-roll
            r.eng.transport(TransportCommand::Stop);
        }
        let res = r.result();
        assert_eq!(res.op.unwrap().window.map(|(a, b)| b - a), Some(E - S));
        r.commit(&res).unwrap();
        hashes.push(vox_testkit::golden::fnv1a_hash(&r.doc()));
    }
    assert_eq!(hashes[0], hashes[1]);
}

/// AC-12 (engine side): a second Record and a calibration are refused while an operation runs.
#[test]
fn refusals_while_an_operation_runs() {
    let mut r = rig(None, None);
    r.run_ms(50);
    r.arm();
    r.run_ms(100);
    r.start(Some((S, E)), prefs());
    r.run_ms(100);
    assert!(r.eng.record_prepare(Some((S, E)), prefs()).is_err());
    assert_eq!(r.eng.calibration_start(0), Err(CalibrationError::Recording));
    r.eng.transport(TransportCommand::Stop);
    let res = r.result();
    r.commit(&res);
}

fn run_calibration(r: &mut Rig, offset_ns: i64) -> vox_dsp::calibration::CalibrationResult {
    r.eng.calibration_start(offset_ns).unwrap();
    for _ in 0..15_000 {
        r.run_ms(1);
        if let CalibrationStatus::Done(done) = r.eng.calibration_poll() {
            let cap = done.expect("the capture");
            return vox_dsp::calibration::analyze(
                &cap.recording,
                &cap.rep_starts,
                &cap.sweep,
                cap.rate_hz,
            );
        }
    }
    panic!("the calibration did not finish");
}

/// AC-16/AC-18 (engine side): a −20 dB loopback with an unreported 240-sample residual under
/// −40 dBFS noise is measured within ±1 sample; Verify with that offset applied reports a
/// residual within ±1 sample; the monitoring mode is restored; callbacks never allocate.
#[test]
fn calibration_measures_the_unreported_residual_and_verify_confirms_it() {
    let mut lb = Loopback::new(dac_key(), mic_key());
    lb.gain_db = -20.0;
    lb.residual_ns = 5 * MS as i64; // 240 samples
    lb.noise = Some((3, -40.0));
    let mut r = rig(None, Some(lb));
    r.run_ms(50);
    r.eng.set_monitor_mode(MonitorMode::Dry);
    let result = run_calibration(&mut r, 0);
    assert!(result.accepted, "{result:?}");
    assert!(
        (result.offset_samples - 240.0).abs() <= 1.0,
        "measured {}",
        result.offset_samples
    );
    assert_eq!(r.eng.record_state().monitor, MonitorMode::Dry, "restored");
    let offset_ns = (result.offset_ms * 1e6).round() as i64;
    let verify = run_calibration(&mut r, offset_ns);
    assert!(verify.accepted, "{verify:?}");
    assert!(
        verify.offset_samples.abs() <= 1.0,
        "residual {}",
        verify.offset_samples
    );
    assert_eq!(r.fake.rt_violations(), 0);
}
