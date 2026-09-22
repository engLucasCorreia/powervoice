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
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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

/// H-120: the deadline every blocking wait on the child in this file uses (same value and
/// rationale as `crates/project/tests/crash.rs`'s `CHILD_TIMEOUT`, H-119) — generous enough to
/// absorb a machine under heavy parallel load, while still turning a genuinely stuck child into a
/// fast, readable test failure instead of a hang that eats the whole gate's ceiling.
const CHILD_TIMEOUT: Duration = Duration::from_secs(20);

/// Reads a child's stdout on a background thread and hands lines to the caller with a bounded
/// wait, so a stalled child fails the read in seconds instead of blocking the test thread forever
/// (H-119's pattern, copied from `crates/project/tests/crash.rs`).
struct LineReader {
    rx: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl LineReader {
    fn spawn(stdout: std::process::ChildStdout) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("punch-crash-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the child's stdout reader thread");
        LineReader {
            rx,
            seen: Vec::new(),
        }
    }

    /// The next line within `timeout`. Panics naming `context` and every line seen from the child
    /// so far, if the deadline passes or the child's output ends before a line arrives.
    fn next_line(&mut self, timeout: Duration, context: &str) -> String {
        match self.rx.recv_timeout(timeout) {
            Ok(line) => {
                self.seen.push(line.clone());
                line
            }
            Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                "{context}: the child printed nothing for {timeout:?} (stalled or deadlocked); \
                 lines seen from it so far: {:?}",
                self.seen
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                "{context}: the child's output ended before the expected line arrived; \
                 lines seen from it so far: {:?}",
                self.seen
            ),
        }
    }
}

/// SIGKILLs `child` and waits up to `timeout` for it to be reaped. Panics naming `context` if it
/// doesn't exit in time — `SIGKILL` can't be blocked, but a stuck OS wait shouldn't hang either.
fn kill_and_reap(child: &mut Child, timeout: Duration, context: &str) {
    let _ = child.kill();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() >= deadline => {
                panic!("{context}: the child didn't exit within {timeout:?} of SIGKILL")
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("{context}: lost the child process while reaping it: {e}"),
        }
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
/// directory and `K`. Every wait on the child — for the next line, and for it to exit — is
/// bounded by [`CHILD_TIMEOUT`] (H-120: this used to be an unbounded `lines.next()`/`wait()`, the
/// hang class H-119 fixed in `crates/project/tests/crash.rs`), so a stalled child fails the test
/// fast with a diagnostic instead of hanging the whole gate until its outer ceiling.
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
    let mut reader = LineReader::spawn(child.0.stdout.take().unwrap());
    let context = format!("punch crash child (phase {phase})");
    let mut dir = None;
    loop {
        let line = reader.next_line(CHILD_TIMEOUT, &context);
        // libtest prints `test name ... ` without a newline first: match markers anywhere.
        if let Some(i) = line.find("DIR=") {
            dir = Some(PathBuf::from(line[i + 4..].trim()));
        }
        if let Some(i) = line.find("READY K=") {
            let k = line[i + 8..].trim().parse().unwrap();
            kill_and_reap(&mut child.0, CHILD_TIMEOUT, &context);
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

// --- H-120: the deadline itself ----------------------------------------------------------------

/// Set to run this process as [`punch_crash_stall_child`] instead of a normal test binary
/// invocation.
const STALL_CHILD_ENV: &str = "VOX_PUNCH_CRASH_STALL";

/// A child that never prints anything — used to prove the deadline in [`LineReader`]/
/// [`crash_child`] actually bounds the wait instead of hanging (H-119's stall pattern, applied
/// here to this file's own child-reading loop).
#[test]
fn punch_crash_stall_child() {
    if std::env::var_os(STALL_CHILD_ENV).is_none() {
        return;
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// H-120: a child that never prints `DIR=`/`READY K=` must fail [`crash_child`]'s wait within its
/// timeout, with a message naming what it was waiting on and showing what the child had printed
/// so far (nothing) — not hang until an outer ceiling (e.g. the gate's 50-minute one) kills the
/// whole run.
#[test]
fn a_child_that_never_answers_fails_fast_with_a_diagnostic_message() {
    if std::env::var_os(CHILD_DIR).is_some() || std::env::var_os(STALL_CHILD_ENV).is_some() {
        return;
    }
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "punch_crash_stall_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(STALL_CHILD_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut reader = LineReader::spawn(child.0.stdout.take().unwrap());
    let timeout = Duration::from_millis(200);
    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // libtest itself prints a couple of header lines (a leading blank line, "running 1
        // test") before the child ever would — skip past those and wait for a marker the child
        // never prints, the same way `crash_child`'s loop searches for its own markers.
        loop {
            let line = reader.next_line(timeout, "stall test");
            if line.contains("READY") {
                break line;
            }
        }
    }));
    let elapsed = start.elapsed();
    // Whether or not the read above reaped the child, make sure it's gone: it's only sleeping, so
    // `SIGKILL` always lands instantly.
    kill_and_reap(&mut child.0, CHILD_TIMEOUT, "stall test cleanup");

    let payload = result.expect_err("a child that never answers must fail the test, not hang");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .expect("panic payload is a string");

    assert!(
        elapsed < Duration::from_secs(10),
        "took {elapsed:?} to fail — the deadline isn't bounding the wait (timeout was {timeout:?})"
    );
    assert!(
        message.contains("stall test"),
        "message doesn't name the wait it stalled on: {message:?}"
    );
    assert!(
        message.contains("stalled") || message.contains("printed nothing"),
        "message doesn't say the child stalled: {message:?}"
    );
}
