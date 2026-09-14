//! T-301: process-crash tests (SPEC-004 AC-9 kill points, AC-12 dead-pid temp files). They spawn
//! child processes, so they live in their own test binary: between `fork` and `exec` a child holds
//! a copy of every open file descriptor, including other tests' session locks (`flock` is per open
//! file description), which would make concurrent recovery tests see sessions "in use".

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use common::script::*;
use common::*;
use vox_project::gc::{SessionClass, classify_session, remove_stale_temp_files};
use vox_project::{Session, TAKE_LABEL_KEY, TakeMode, TakeWriterOptions};

// --- AC-9: SIGKILL right after a command reported success --------------------------------------

const CHILD_DIR: &str = "VOX_PROJECT_CRASH_DIR";
const CHILD_SEED: &str = "VOX_PROJECT_CRASH_SEED";
const CHILD_MODE: &str = "VOX_PROJECT_CRASH_MODE";

/// Kills (SIGKILL) and reaps the child on every exit path.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
/// Returns its last `prefix` line (without the prefix).
fn run_and_kill(child: &mut ChildGuard, prefix: &str, steps: usize) -> String {
    let mut stdin = child.0.stdin.take().unwrap();
    let mut lines = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let mut last = String::new();
    for _ in 0..steps {
        writeln!(stdin, "go").unwrap();
        stdin.flush().unwrap();
        last = loop {
            let line = lines.next().expect("the child ended early").unwrap();
            // Not `strip_prefix`: libtest prints "test crash_child ... " without a newline before
            // running the test, so the child's first line shares that line.
            if let Some(i) = line.find(prefix) {
                break line[i + prefix.len()..].to_owned();
            }
        };
    }
    child.0.kill().unwrap(); // SIGKILL: no destructor runs
    child.0.wait().unwrap();
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
        let expected = run_and_kill(&mut child, "STATE ", steps);
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
        let last = run_and_kill(&mut child, "TAKE ", steps);
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
