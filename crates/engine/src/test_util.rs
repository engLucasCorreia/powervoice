//! Test support (feature `test-util`, H-53): the T-304 loopback rig, shared so both this crate's
//! own `tests/` (`punch.rs`) and `src-tauri`'s tests can drive a record op or a calibration run
//! through the fake backend without copying the setup. Follows the pattern
//! [`vox_module_api::test_util`] already uses: a module gated behind this crate's own
//! `test-util` feature (enabled here via a self dev-dependency), not a private `tests/`-only copy.
//!
//! - [`plug_loopback_devices`]: the fake hardware — a `"DAC"` output and a `"Mic"` input matching
//!   [`Loopback::new`]'s keys ([`dac_key`], [`mic_key`]) — the part every caller needs, including
//!   one (like a job-service test) that drives a real [`crate::Engine`] + `FakeDriver` rather than
//!   a [`ManualEngine`].
//! - [`Rig`] / [`rig`] / [`rig_with`] / [`rig_full`] / [`talent_rig`]: this crate's own
//!   `ManualEngine` + [`vox_project::Session`] harness, built on [`plug_loopback_devices`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::backend::StreamId;
use crate::backend::fake::{
    AlignedTalent, CallbackSizes, FakeBackend, FakeDevice, FakeDirection, Loopback,
};
use crate::record::RecordingResult;
use crate::record_op::{RecordOpKind, RecordPlan, RecordPrefs};
use crate::{
    DeviceKey, DevicePrefs, Direction, EngineConfig, EngineEvent, HostId, ManualEngine, PlaybackDoc,
};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_project::{
    FixedFreeSpace, HistoryStep, Session, SessionConfig, StoreOptions, TakeMode, TakeParams,
    TakeWriterOptions,
};
use vox_rack::Registry;

const MS: u64 = 1_000_000;
/// The rig's default document rate.
pub const RATE: u32 = 48_000;
/// The rig's default document length (10 s at 48 kHz).
pub const L: usize = 480_000;

/// Deterministic seeded noise, for a rig's document or its talent script.
pub fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = vox_testkit::prng::Pcg32::new(seed, 7);
    (0..n).map(|_| (rng.next_signed() * 0.3) as f32).collect()
}

/// The rig's output device key, matching [`plug_loopback_devices`] (`Loopback::new(dac_key(),
/// mic_key())`).
pub fn dac_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "DAC")
}

/// The rig's input device key (see [`dac_key`]).
pub fn mic_key() -> DeviceKey {
    DeviceKey::new(HostId::Alsa, "Mic")
}

/// The rig's fake hardware: a `"DAC"` output (2 ch, 7 ms latency, recorded) and a `"Mic"` input
/// (1 ch, 5 ms latency, random callback sizes unless `mic_source` overrides the signal), with
/// `loopback` wired in. Shared by [`rig_full`] and by callers (e.g. src-tauri's calibration test,
/// H-53) that drive their own [`crate::Engine`] rather than a [`ManualEngine`].
pub fn plug_loopback_devices(
    fake: &FakeBackend,
    mic_source: Option<fn(u64) -> f32>,
    loopback: Option<Loopback>,
    mic_skew_ppm: f64,
) {
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
        .latency_ns(5 * MS)
        .skew_ppm(mic_skew_ppm);
    if let Some(s) = mic_source {
        mic = mic.source(move |f, _, _| s(f));
    }
    fake.plug(HostId::Alsa, FakeDevice::new("Mic").with_input(mic));
    fake.set_loopback(loopback);
}

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

/// A [`ManualEngine`] + [`vox_project::Session`] wired to [`plug_loopback_devices`]'s fake
/// hardware, ticked by hand ([`Rig::run_ms`]) — the T-304 loopback rig.
pub struct Rig {
    pub fake: FakeBackend,
    pub eng: ManualEngine,
    pub phases: Arc<Mutex<Vec<(crate::record::RecordPhaseInfo, u64)>>>,
    pub windows: Arc<Mutex<Vec<(u32, u64)>>>,
    pub done: Arc<Mutex<Option<RecordingResult>>>,
    pub session: Session,
    pub a: Vec<f32>,
    /// Fake time the input stream opened (its frame 0).
    pub t_open: u64,
    _dir: TempDir,
}

/// The §5 fake backend: one clock, reported input latency 5 ms and output latency 7 ms, random
/// input callback sizes; a 10 s noise document; empty rack; monitoring Off.
pub fn rig(mic_source: Option<fn(u64) -> f32>, loopback: Option<Loopback>) -> Rig {
    rig_with(noise(1, L), mic_source, loopback, 0.0)
}

/// [`rig`] with the document `a` and the microphone's clock skew (ppm, H-21).
pub fn rig_with(
    a: Vec<f32>,
    mic_source: Option<fn(u64) -> f32>,
    loopback: Option<Loopback>,
    mic_skew_ppm: f64,
) -> Rig {
    let empty = vox_rack::RackModel { slots: Vec::new() };
    rig_full(a, mic_source, loopback, mic_skew_ppm, empty)
}

/// `TestDelay(480)`: a pure-delay rack module reporting 10 ms of latency (AC-6, T-401).
struct DelayFactory(vox_module_api::ModuleDescriptor);

impl vox_module_api::ModuleFactory for DelayFactory {
    fn descriptor(&self) -> &vox_module_api::ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn vox_module_api::Module>, vox_module_api::ModuleError> {
        Ok(Box::new(vox_module_api::test_util::TestDelay::new(480)))
    }
}

/// [`rig_with`] playing through `rack` (the registry also holds `TestDelay(480)`).
pub fn rig_full(
    a: Vec<f32>,
    mic_source: Option<fn(u64) -> f32>,
    loopback: Option<Loopback>,
    mic_skew_ppm: f64,
    rack: vox_rack::RackModel,
) -> Rig {
    assert!(alloc_checks_active());
    let fake = FakeBackend::new(7);
    plug_loopback_devices(&fake, mic_source, loopback, mic_skew_ppm);
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let delay: Arc<dyn vox_module_api::ModuleFactory> = Arc::new(DelayFactory(
        vox_module_api::Module::descriptor(&vox_module_api::test_util::TestDelay::new(480)).clone(),
    ));
    let registry = Arc::new(
        Registry::with_factories(vox_modules::builtin_factories().into_iter().chain([delay]))
            .unwrap(),
    );
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack;
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

/// A rig whose microphone carries the perfectly timed talent `x` (and nothing else, H-21 SPEC-022
/// §4.8 `AlignedTalent`).
pub fn talent_rig(a: Vec<f32>, x: &[f32], mic_skew_ppm: f64) -> Rig {
    let r = rig_with(a, None, None, mic_skew_ppm);
    r.fake.set_talent(Some(AlignedTalent {
        output: dac_key(),
        input: mic_key(),
        reference: Arc::from(r.a.clone()),
        script: Arc::from(x.to_vec()),
    }));
    r
}

/// A default [`RecordPrefs`] (1 s pre-roll, 0.5 s post-roll — short enough for a quick test run).
pub fn prefs() -> RecordPrefs {
    RecordPrefs {
        preroll_s: 1.0,
        postroll_s: 0.5,
        ..RecordPrefs::default()
    }
}

impl Rig {
    pub fn run_ms(&mut self, ms: u64) {
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

    pub fn arm(&mut self) {
        self.t_open = self.fake.now_ns();
        let st = self.eng.set_armed(true);
        assert!(st.input_open, "{st:?}");
    }

    /// Record pressed with `selection`: resolve, begin the matching take, start.
    pub fn start(&mut self, selection: Option<(u64, u64)>, prefs: RecordPrefs) -> RecordPlan {
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

    pub fn result(&mut self) -> RecordingResult {
        for _ in 0..20_000 {
            if let Some(r) = self.done.lock().unwrap().take() {
                return r;
            }
            self.run_ms(1);
        }
        panic!("the operation did not finish");
    }

    /// What the app does with the result: commit the window, or cancel.
    pub fn commit(&mut self, r: &RecordingResult) -> Option<HistoryStep> {
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

    pub fn doc(&self) -> Vec<f32> {
        let snap = self.session.current();
        let mut out = vec![0.0; snap.len_samples as usize];
        self.session.store().read(&snap, 0, &mut out).unwrap();
        out
    }

    pub fn output_stream(&self) -> StreamId {
        self.fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Output)
            .map(|s| s.info.id)
            .unwrap()
    }

    pub fn output(&self) -> Vec<f32> {
        self.fake
            .recorded_output(self.output_stream())
            .unwrap()
            .samples
    }

    /// Output frame index where document position `q` (in an unfaded, audible stretch) is
    /// heard — located by an exact match of 64 samples.
    pub fn heard_frame(&self, out: &[f32], q: usize) -> usize {
        let pat = &self.a[q..q + 64];
        out.windows(64)
            .position(|w| w.iter().zip(pat).all(|(x, y)| x.to_bits() == y.to_bits()))
            .expect("document position not heard")
    }

    pub fn input_stream(&self) -> StreamId {
        self.fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Input)
            .map(|s| s.info.id)
            .unwrap()
    }

    /// The document position truly heard at app time `t_ns` (an audible, unfaded stretch near
    /// `near`): the output frame playing at `t_ns`, matched against the document.
    pub fn true_heard(&self, t_ns: u64, near: u64) -> u64 {
        let id = self.output_stream();
        let out = self.output();
        let mut g = out.len() - 64;
        while self.fake.frame_time_ns(id, g as u64).unwrap() > t_ns {
            g -= 1;
        }
        let pat = &out[g..g + 32];
        let lo = near.saturating_sub(4_000) as usize;
        let hi = (near as usize + 4_000).min(self.a.len() - 32);
        (lo..hi)
            .find(|&q| {
                self.a[q..q + 32]
                    .iter()
                    .zip(pat)
                    .all(|(x, y)| x.to_bits() == y.to_bits())
            })
            .map(|q| q as u64)
            .expect("the heard audio matches the document near the expected position")
    }
}
