//! H-21 / SPEC-022 AC-15: a punch-in interrupted by `SIGKILL`. The test binary re-runs itself as
//! a child (env-gated, the H-05 pattern) that records a punch through the fake backend into a real
//! session — loopback + Hear original, so the captured window is exactly `A` — prints `READY` once
//! `K` window samples are captured (or during pre-roll) and then stops advancing time; the parent
//! kills it and recovers the session. Process-spawning tests live in their own test binary (T-301:
//! fork copies fds, so `flock` locks would leak into children).

vox_module_api::install_test_allocator!();

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vox_engine::backend::fake::{CallbackSizes, FakeBackend, FakeDevice, FakeDirection, Loopback};
use vox_engine::record::RecordingResult;
use vox_engine::record_op::{RecordOpKind, RecordPrefs};
use vox_engine::{
    DeviceKey, DevicePrefs, EngineConfig, EngineEvent, HostId, ManualEngine, PlaybackDoc,
};
use vox_project::{
    FixedFreeSpace, PUNCH_LABEL_KEY, Session, SessionConfig, StoreOptions, TakeId, TakeMode,
    TakeParams, TakeWriterOptions,
};
use vox_rack::Registry;
use vox_testkit::prng::Pcg32;

const MS: u64 = 1_000_000;
const RATE: u32 = 48_000;
const L: usize = 480_000;
const S: u64 = 240_000;
const E: u64 = 384_000;
/// The child's sessions directory (set: this process is the child).
const CHILD_DIR: &str = "VOX_PUNCH_CRASH_DIR";
/// `window` (kill after ≥ 1.25 s of window) or `preroll`.
const CHILD_PHASE: &str = "VOX_PUNCH_CRASH_PHASE";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-punch-crash-{tag}-{}-{}",
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

/// Kills the child if the parent panics first.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = Pcg32::new(seed, 7);
    (0..n).map(|_| (rng.next_signed() * 0.3) as f32).collect()
}

/// The child: prints `DIR=<session dir>`, then `READY K=<window samples captured>` and waits for
/// `SIGKILL` without advancing (so nothing more is captured after `K`).
#[test]
fn punch_crash_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let phase = std::env::var(CHILD_PHASE).unwrap();
    let dac = DeviceKey::new(HostId::Alsa, "DAC");
    let mic = DeviceKey::new(HostId::Alsa, "Mic");
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
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("Mic").with_input(
            FakeDirection::new(1, &[48_000], 48_000)
                .callback_sizes(CallbackSizes::Random { min: 32, max: 512 })
                .latency_ns(5 * MS),
        ),
    );
    fake.set_loopback(Some(Loopback::new(dac, mic)));
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs = DevicePrefs {
        input_device: Some("Mic".to_owned()),
        input_channel: 1,
        ..DevicePrefs::default()
    };
    let windows = Arc::new(Mutex::new(Vec::new()));
    let win = windows.clone();
    cfg.events = Arc::new(move |e| {
        if let EngineEvent::RecordWindow { take, k_start } = e {
            win.lock().unwrap().push((take, k_start));
        }
    });
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    cfg.disk_space = Arc::new(FixedFreeSpace::new(100 << 30));
    cfg.record_volume = PathBuf::from(&dir);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    let mut session = Session::create(
        Path::new(&dir),
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: StoreOptions::with_memory_budget(128 << 20),
        },
    )
    .unwrap();
    let mut writer = session.chunk_writer();
    writer.append(&noise(1, L)).unwrap();
    let audio = writer.finish().unwrap();
    session.set_floor(&audio, Vec::new()).unwrap();
    eng.set_document(Some(PlaybackDoc {
        store: session.store().clone(),
        snapshot: session.current(),
    }));
    // One simulated ms per step; the app's job: journal each opened window (`take_window`).
    let step = |eng: &mut ManualEngine, session: &mut Session| {
        fake.advance_by(MS);
        eng.tick();
        for (take, k) in std::mem::take(&mut *windows.lock().unwrap()) {
            session.note_take_window(TakeId(take), k).unwrap();
        }
    };
    for _ in 0..50 {
        step(&mut eng, &mut session);
    }
    assert!(eng.set_armed(true).input_open);
    for _ in 0..100 {
        step(&mut eng, &mut session);
    }
    let prefs = RecordPrefs {
        preroll_s: 1.0,
        postroll_s: 0.5,
        hear_original: true,
        ..RecordPrefs::default()
    };
    let plan = eng.record_prepare(Some((S, E)), prefs).unwrap();
    assert_eq!(plan.kind, RecordOpKind::Punch);
    let capture = session
        .begin_take_with(
            TakeMode::Punch {
                start_samples: S,
                end_samples: E,
            },
            TakeParams {
                xfade_samples: plan.xfade_samples,
                offset_ns: plan.offset_ns,
                aligned: plan.aligned,
            },
            TakeWriterOptions::default(),
        )
        .unwrap();
    let done: Arc<Mutex<Option<RecordingResult>>> = Arc::default();
    let slot = done.clone();
    eng.record_start_op(
        plan,
        capture,
        Box::new(move |r| *slot.lock().unwrap() = Some(r)),
    )
    .unwrap();
    println!("DIR={}", session.dir().display());
    let k = if phase == "preroll" {
        for _ in 0..300 {
            step(&mut eng, &mut session);
        }
        0
    } else {
        let mut k = 0;
        for _ in 0..10_000 {
            step(&mut eng, &mut session);
            k = eng.live_take_peaks(0, 0).map_or(0, |p| p.len_samples);
            if k >= 60_000 {
                break;
            }
        }
        assert!(k >= 60_000, "the window never reached 1.25 s");
        k
    };
    println!("READY K={k}");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Runs the child to `phase`, kills it (`SIGKILL`: no destructor runs) and returns its session
/// directory and `K`.
fn crash_child(phase: &str, sessions: &Path) -> (PathBuf, u64) {
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "punch_crash_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DIR, sessions)
            .env(CHILD_PHASE, phase)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut lines = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let mut dir = None;
    loop {
        let line = lines
            .next()
            .expect("the child ended before printing READY")
            .unwrap();
        // libtest prints `test name ... ` without a newline first: match markers anywhere.
        if let Some(i) = line.find("DIR=") {
            dir = Some(PathBuf::from(line[i + 4..].trim()));
        }
        if let Some(i) = line.find("READY K=") {
            let k = line[i + 8..].trim().parse().unwrap();
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            return (dir.expect("DIR before READY"), k);
        }
    }
}

fn read_doc(session: &Session) -> Vec<f32> {
    let snap = session.current();
    let mut out = vec![0.0; snap.len_samples as usize];
    session.store().read(&snap, 0, &mut out).unwrap();
    out
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

/// AC-15: `SIGKILL` after `K` (≥ 1 s) window samples of a punch — recovery offers the interrupted
/// punch at `S`; "Apply as recorded" is a partial punch to `L_rec` with `K − 0.25·rate ≤ L_rec ≤
/// K`, its interior bit-identical to the live run's captured window (alignment preserved through
/// `take_window`), as one "Punch-in" entry.
#[test]
fn ac15_sigkill_during_the_window_recovers_a_partial_punch() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let tmp = TempDir::new("window");
    let (dir, k) = crash_child("window", &tmp.0);
    let (mut session, report) =
        Session::recover(&dir, StoreOptions::with_memory_budget(128 << 20)).unwrap();
    let info = report.open_take.expect("the interrupted punch is offered");
    assert_eq!(info.at_samples, S);
    let l_rec = info.samples;
    assert!(
        l_rec <= k && l_rec + u64::from(RATE) / 4 >= k,
        "L_rec {l_rec}, K {k}"
    );
    let step = session.apply_open_take_from_wav().unwrap().unwrap();
    assert_eq!(&*step.label_key, PUNCH_LABEL_KEY);
    assert_eq!(session.history().undo_depth(), 1);
    let out = read_doc(&session);
    let a = noise(1, L);
    let (s, p) = (S as usize, (S + l_rec) as usize);
    assert!(p < E as usize);
    assert_eq!(out.len(), L);
    assert_bits(&out[..s], &a[..s], "A′[0, S)");
    assert_bits(&out[p..], &a[p..], "A′[L_rec, L)");
    assert_bits(
        &out[s + 480..p - 480],
        &a[s + 480..p - 480],
        "the captured window",
    );
}

/// AC-15: `SIGKILL` during pre-roll — recovery lists no take and the document equals the state
/// before the operation.
#[test]
fn ac15_sigkill_during_pre_roll_leaves_the_document_unchanged() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let tmp = TempDir::new("preroll");
    let (dir, _) = crash_child("preroll", &tmp.0);
    let (session, report) =
        Session::recover(&dir, StoreOptions::with_memory_budget(128 << 20)).unwrap();
    assert!(report.open_take.is_none(), "no take is offered");
    assert!(!session.is_recording());
    assert_bits(
        &read_doc(&session),
        &noise(1, L),
        "the document before the punch",
    );
    assert_eq!(session.history().undo_depth(), 0);
}
