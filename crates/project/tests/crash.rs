//! T-301: process-crash tests (SPEC-004 AC-9 kill points, AC-12 dead-pid temp files). They spawn
//! child processes, so they live in their own test binary: between `fork` and `exec` a child holds
//! a copy of every open file descriptor, including other tests' session locks (`flock` is per open
//! file description), which would make concurrent recovery tests see sessions "in use".

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::script::*;
use common::*;
use vox_project::gc::{SessionClass, classify_session, remove_stale_temp_files};
use vox_project::{Session, TAKE_LABEL_KEY, TakeMode, TakeWriterOptions};

// --- AC-9: SIGKILL right after a command reported success --------------------------------------

const CHILD_DIR: &str = "VOX_PROJECT_CRASH_DIR";
const CHILD_SEED: &str = "VOX_PROJECT_CRASH_SEED";
const CHILD_MODE: &str = "VOX_PROJECT_CRASH_MODE";

/// H-119: the deadline every blocking wait on a child in this file uses. A normal iteration
/// answers within microseconds; this is generous enough to absorb a machine under heavy parallel
/// load without false failures, while still turning a genuinely stuck child into a fast, readable
/// test failure instead of a hang that eats the whole gate's ceiling.
const CHILD_TIMEOUT: Duration = Duration::from_secs(20);

/// Kills (SIGKILL) and reaps the child on every exit path.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads a child's stdout on a background thread and hands lines to the caller with a bounded
/// wait, so a stalled child (wedged, deadlocked, or simply never answering the next `go`) fails
/// the read in seconds instead of blocking the test thread forever.
struct LineReader {
    rx: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl LineReader {
    fn spawn(stdout: std::process::ChildStdout) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("crash-test-reader".into())
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

    /// The next line within `timeout`. Panics naming `context` (which step/iteration this was)
    /// and every line seen from the child so far, if the deadline passes or the child's output
    /// ends before the expected line arrives.
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

/// Child half of the kill tests (a no-op in a normal run). Waits for a `go` line on stdin, runs
/// one command, prints the resulting state, and waits again — so when the parent kills it, the
/// state it printed last is exactly what the journal must reproduce.
#[test]
fn crash_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let seed: u64 = std::env::var(CHILD_SEED).unwrap().parse().unwrap();
    let mode = std::env::var(CHILD_MODE).unwrap();
    let mut s = new_session(Path::new(&dir));
    set_floor(&mut s, &noise(seed, 200_000));
    let mut rng = Rng::new(seed);
    let mut clip = Vec::new();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut line = String::new();
    let wait_go = |line: &mut String| {
        line.clear();
        stdin.lock().read_line(line).unwrap() > 0
    };
    if mode == "stall" {
        // H-119: a child that accepts one `go` and then never answers again — used to prove the
        // deadline in `LineReader`/`run_and_kill` actually bounds the wait instead of hanging.
        wait_go(&mut line);
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    if mode == "take" {
        for _ in 0..3 {
            random_op(&mut s, &mut rng, &mut clip);
        }
        let base = read_all(s.store(), &s.current());
        let mut capture = s
            .begin_take(TakeMode::New, TakeWriterOptions::default())
            .unwrap();
        let mut take = Vec::new();
        while wait_go(&mut line) {
            let block = noise(seed ^ take.len() as u64, 4_800 + rng.below(20_000) as usize);
            capture.append(&block).unwrap();
            take.extend_from_slice(&block);
            let mut fnv = Fnv::new();
            fnv.update(&base);
            fnv.update(&take);
            writeln!(out, "TAKE {} {:016x}", take.len(), fnv.finish()).unwrap();
            out.flush().unwrap();
        }
        return;
    }
    while wait_go(&mut line) {
        random_op(&mut s, &mut rng, &mut clip);
        if rng.below(6) == 0 {
            s.mark_saved(Path::new("/nonexistent/doc.wav"), "wav24")
                .unwrap();
        }
        writeln!(out, "STATE {}", fingerprint(&s)).unwrap();
        out.flush().unwrap();
    }
}

fn spawn_child(dir: &Path, seed: u64, mode: &str) -> ChildGuard {
    ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
            .env(CHILD_DIR, dir)
            .env(CHILD_SEED, seed.to_string())
            .env(CHILD_MODE, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

/// Lets the child run `steps` commands, then SIGKILLs it while it waits for the next `go`.
/// Returns its last `prefix` line (without the prefix). Every wait on the child — for the next
/// line, and for it to exit — is bounded by `timeout`; `context` (e.g. "point 3 (seed …)") names
/// the run in any failure message.
fn run_and_kill(
    child: &mut ChildGuard,
    prefix: &str,
    steps: usize,
    context: &str,
    timeout: Duration,
) -> String {
    let mut stdin = child.0.stdin.take().unwrap();
    let mut reader = LineReader::spawn(child.0.stdout.take().unwrap());
    let mut last = String::new();
    for step in 0..steps {
        writeln!(stdin, "go").unwrap();
        stdin.flush().unwrap();
        let where_ = format!("{context}, step {}/{steps}", step + 1);
        last = loop {
            let line = reader.next_line(timeout, &where_);
            // Not `strip_prefix`: libtest prints "test crash_child ... " without a newline before
            // running the test, so the child's first line shares that line.
            if let Some(i) = line.find(prefix) {
                break line[i + prefix.len()..].to_owned();
            }
        };
    }
    kill_and_reap(&mut child.0, timeout, context); // SIGKILL: no destructor runs
    last
}

/// SPEC-004 AC-9 (first bullet): `SIGKILL` at 50 random points, each right after a command
/// reported success; recovery reproduces the exact audio hash, marker list, undo/redo depths,
/// entry labels and modified flag.
#[test]
fn ac9_kill_9_after_success_at_50_points_recovers_the_exact_state() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let mut rng = Rng::new(0x0005_eed5);
    for point in 0..50 {
        let dir = TempDir::new("ac9");
        let steps = 1 + rng.below(30) as usize;
        let seed = rng.next() >> 1;
        let mut child = spawn_child(dir.path(), seed, "edits");
        let context = format!("point {point} (seed {seed}, {steps} steps)");
        let expected = run_and_kill(&mut child, "STATE ", steps, &context, CHILD_TIMEOUT);
        let (session, report) = Session::recover(&only_session(dir.path()), options())
            .unwrap_or_else(|e| panic!("point {point} (seed {seed}): {e}"));
        assert_eq!(
            fingerprint(&session),
            expected,
            "point {point} (seed {seed}, {steps} steps)"
        );
        assert_eq!(report.lost_changes, 0, "point {point}");
    }
}

/// A process crash mid-take loses nothing the capture-writer appended: recovery offers the take
/// and "Apply as recorded" makes the document exactly base ++ take, as one "Record" entry.
#[test]
fn kill_9_mid_take_then_apply_as_recorded_is_exact() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let mut rng = Rng::new(0x7a4e);
    for point in 0..5 {
        let dir = TempDir::new("take-kill");
        let steps = 1 + rng.below(8) as usize;
        let seed = rng.next() >> 1;
        let mut child = spawn_child(dir.path(), seed, "take");
        let context = format!("point {point} (seed {seed}, {steps} steps)");
        let last = run_and_kill(&mut child, "TAKE ", steps, &context, CHILD_TIMEOUT);
        let (samples, hash) = last.split_once(' ').unwrap();
        let samples: u64 = samples.parse().unwrap();
        let session_dir = only_session(dir.path());
        match classify_session(&session_dir).unwrap() {
            SessionClass::Recoverable(r) => {
                assert_eq!(r.open_take_samples, samples, "point {point}");
            }
            SessionClass::Clean => panic!("an interrupted take must be recoverable"),
        }
        let (mut s, report) = Session::recover(&session_dir, options()).unwrap();
        let take = report.open_take.expect("the take is offered");
        assert_eq!(take.samples, samples, "point {point}");
        assert!(
            s.is_recording(),
            "the take stays open until the user decides"
        );
        let depth = s.history().undo_depth();
        s.apply_open_take_from_wav().unwrap().expect("one edit");
        assert!(!s.is_recording());
        assert_eq!(s.history().undo_depth(), depth + 1);
        assert_eq!(s.history().undo_label(), Some(TAKE_LABEL_KEY));
        let got = hash_of(&read_all(s.store(), &s.current()));
        assert_eq!(format!("{got:016x}"), hash, "point {point}");
    }
}

// --- H-119: the deadline itself ----------------------------------------------------------------

/// H-119: a child that accepts one `go` and then never answers again must fail `run_and_kill`
/// within its timeout, with a message naming the step and showing what the child had printed so
/// far — not hang until an outer ceiling (e.g. the gate's 50-minute one) kills the whole run.
#[test]
fn a_child_that_never_answers_fails_fast_with_a_diagnostic_message() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let dir = TempDir::new("stall");
    let mut child = spawn_child(dir.path(), 1, "stall");
    let timeout = Duration::from_millis(200);
    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_and_kill(&mut child, "STATE ", 2, "stall test", timeout)
    }));
    let elapsed = start.elapsed();
    // Whether or not `run_and_kill` reaped the child before panicking, make sure it's gone: it's
    // only sleeping, so `SIGKILL` always lands instantly.
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
        message.contains("stall test, step 1/2"),
        "message doesn't name the step it stalled on: {message:?}"
    );
    assert!(
        message.contains("stalled") || message.contains("printed nothing"),
        "message doesn't say the child stalled: {message:?}"
    );
}

// --- AC-12 -----------------------------------------------------------------------------------------

/// SPEC-004 AC-12: a stale `.powervoice-tmp-<dead pid>` is removed at the next save into that
/// folder; one whose pid is alive is not.
#[test]
fn ac12_stale_temp_files_of_dead_pids_are_removed() {
    let dir = TempDir::new("stale-tmp");
    let mut child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let stale = dir.path().join(format!(".take.wav.powervoice-tmp-{dead}"));
    let alive = dir
        .path()
        .join(format!(".take.wav.powervoice-tmp-{}", std::process::id()));
    let unrelated = dir.path().join("take.wav");
    for p in [&stale, &alive, &unrelated] {
        std::fs::write(p, b"x").unwrap();
    }
    assert_eq!(remove_stale_temp_files(dir.path()), 1);
    assert!(!stale.exists());
    assert!(alive.exists() && unrelated.exists());
}

// --- H-15: AC-6 sidecar-write crash safety (SPEC-018 §2.3/§2.8/§4.4) ---------------------------

/// AC-6 (dead-pid rule, sidecar variant): a leftover `.‹sidecar name›.powervoice-tmp-‹dead pid›`
/// is deleted at the next write into that folder; one whose pid is alive is not. Doesn't need a
/// real crash — a completed `Command::new("true")` gives a definitely-dead pid.
#[test]
fn ac6_sidecar_dead_pid_temp_files_are_swept_but_a_live_one_is_kept() {
    let dir = TempDir::new("sidecar-stale-tmp");
    let path = dir.path().join("a.wav.vo.json");

    let mut dead_child = Command::new("true").spawn().unwrap();
    let dead = dead_child.id();
    dead_child.wait().unwrap();
    let dead_tmp = dir
        .path()
        .join(format!(".a.wav.vo.json.powervoice-tmp-{dead}"));
    std::fs::write(&dead_tmp, b"leftover").unwrap();

    // A genuinely different, still-running pid (never the current process's own — that pid is
    // what `write_sidecar` itself would use as *its* temp file name below, so reusing it here
    // would make the real write legitimately consume and rename this file away, not "leave it
    // alone").
    let mut live_child = Command::new("sleep").arg("5").spawn().unwrap();
    let live = live_child.id();
    let live_tmp = dir
        .path()
        .join(format!(".a.wav.vo.json.powervoice-tmp-{live}"));
    std::fs::write(&live_tmp, b"leftover").unwrap();

    let markers = crash_markers(1);
    let input = sidecar_crash_write_input(&markers, 1);
    vox_project::write_sidecar(&path, &input, false).unwrap();

    assert!(
        !dead_tmp.exists(),
        "the dead pid's leftover temp file is swept"
    );
    assert!(live_tmp.exists(), "a live pid's temp file is left alone");
    let _ = live_child.kill();
    let _ = live_child.wait();
}

/// 10 000 seeded, position-valid markers (AC-6: "sidecar ≈ 1.5 MB").
fn crash_markers(seed: u64) -> Vec<vox_project::MarkerItemModel> {
    let mut rng = Rng::new(seed ^ 0xA5A5_A5A5_u64);
    (1..=10_000u64)
        .map(|id| vox_project::MarkerItemModel {
            id,
            pos_samples: rng.below(48_000 * 590),
            len_samples: 0,
            name: format!("Marker {id}"),
            kind: "user".to_string(),
            extra: Default::default(),
        })
        .collect()
}

/// The document identity every [`sidecar_crash_write_input`] call agrees on — a real document's
/// facts don't change between two saves of the same session, only its markers/rack (which is
/// exactly what makes S0 and the child's write differ).
fn crash_identity() -> vox_project::DocumentIdentity {
    vox_project::DocumentIdentity {
        sample_rate_hz: 48_000,
        len_samples: 48_000 * 600,
        audio_crc32: 0x1234_5678,
    }
}

fn sidecar_crash_write_input(
    markers: &[vox_project::MarkerItemModel],
    seed: u64,
) -> vox_project::WriteInput<'_> {
    let identity = crash_identity();
    vox_project::WriteInput {
        file_name: "a.wav".to_string(),
        file_size_bytes: 123,
        file_mtime: "2026-09-13T08:41:06Z".to_string(),
        sample_rate_hz: identity.sample_rate_hz,
        len_samples: identity.len_samples,
        audio_crc32: identity.audio_crc32,
        save_format: vox_project::SaveFormatModel {
            container: "wav".to_string(),
            sample_format: "pcm24".to_string(),
            dither: "tpdf".to_string(),
            extra: Default::default(),
        },
        markers,
        rack: serde_json::json!({ "slots": [] }),
        view: serde_json::json!({}),
        app_version: "0.3.0".to_string(),
        written_at: format!("2026-09-13T08:41:{:02}Z", 7 + seed % 50),
    }
}

const SIDECAR_CHILD_PATH: &str = "VOX_PROJECT_SIDECAR_CRASH_PATH";
const SIDECAR_CHILD_SEED: &str = "VOX_PROJECT_SIDECAR_CRASH_SEED";

/// Child half of the sidecar-write kill test (a no-op in a normal run): on "go", writes a large
/// sidecar once and prints "READY" immediately before the write, "DONE" immediately after — the
/// parent races a `SIGKILL` against that gap, so the kill can land while the write is actually in
/// flight, not merely between two completed commands (AC-9's pattern).
#[test]
fn sidecar_crash_child() {
    let Some(path) = std::env::var_os(SIDECAR_CHILD_PATH) else {
        return;
    };
    let seed: u64 = std::env::var(SIDECAR_CHILD_SEED).unwrap().parse().unwrap();
    let path = std::path::PathBuf::from(path);
    let markers = crash_markers(seed);
    let input = sidecar_crash_write_input(&markers, seed);

    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).unwrap() == 0 {
        return;
    }
    writeln!(out, "READY").unwrap();
    out.flush().unwrap();
    vox_project::write_sidecar(&path, &input, false).unwrap();
    writeln!(out, "DONE").unwrap();
    out.flush().unwrap();
}

/// AC-6: `SIGKILL`ed while a sidecar write is racing to completion, at 20 seeded points — the
/// sidecar on disk is always either byte-identical to the pre-existing S₀ or a complete, parseable
/// v1 sidecar of the new state, never partial or corrupt; a dead-pid temp file it left behind is
/// swept at the next write into the same folder (the case above, exercised here end to end too).
#[test]
fn ac6_sigkill_during_a_sidecar_write_never_leaves_a_partial_or_corrupt_file() {
    if std::env::var_os(SIDECAR_CHILD_PATH).is_some() {
        return;
    }
    let identity = crash_identity();
    let mut rng = Rng::new(0xC0FF_EE00);
    for point in 0..20 {
        let dir = TempDir::new("sidecar-kill");
        let path = dir.path().join("a.wav.vo.json");

        // S0: a valid, already-written v1 sidecar (seed 0, so it always differs from the child's
        // seeded content below).
        let markers0 = crash_markers(0);
        let input0 = sidecar_crash_write_input(&markers0, 0);
        vox_project::write_sidecar(&path, &input0, false).unwrap();
        let s0_bytes = std::fs::read(&path).unwrap();

        let seed = 1 + (rng.next() >> 1);
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sidecar_crash_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(SIDECAR_CHILD_PATH, &path)
                .env(SIDECAR_CHILD_SEED, seed.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut stdin = child.0.stdin.take().unwrap();
        let mut reader = LineReader::spawn(child.0.stdout.take().unwrap());
        writeln!(stdin, "go").unwrap();
        stdin.flush().unwrap();
        let context = format!("point {point} (seed {seed})");
        loop {
            let line = reader.next_line(CHILD_TIMEOUT, &context);
            if line.contains("READY") {
                break;
            }
        }
        // A small seeded delay races the kill against the write itself. Exactly where it lands is
        // up to OS scheduling either way — that unpredictability is the point: every possible
        // instant must satisfy the invariant checked below.
        std::thread::sleep(Duration::from_micros(rng.below(3_000)));
        kill_and_reap(&mut child.0, CHILD_TIMEOUT, &context);

        let bytes = std::fs::read(&path).unwrap_or_default();
        if bytes == s0_bytes {
            continue; // Killed before the rename: S0 stands, byte-identical and untouched.
        }
        // Otherwise it must be the complete, valid new state — never a torn or corrupt file.
        let load = vox_project::read_sidecar(&path, identity);
        assert!(
            load.doc.is_some() && load.notice.is_none(),
            "point {point} (seed {seed}): on-disk sidecar is neither S0 nor a complete valid v1 file"
        );
        assert_eq!(load.markers.len(), 10_000, "point {point} (seed {seed})");

        // The dead child's temp file (if the kill landed mid-write, before rename) is swept at
        // the next write into the same folder — AC-6's dead-pid rule, exercised end to end.
        let next_markers = crash_markers(999);
        let next_input = sidecar_crash_write_input(&next_markers, 999);
        vox_project::write_sidecar(&path, &next_input, false).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("powervoice-tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "point {point}: a dead-pid sidecar temp file survived a later save"
        );
    }
}
