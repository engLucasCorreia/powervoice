//! SPEC-004 AC-12, M1 part: session cleanup at close and at start-up.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::*;
use vox_project::gc::{
    DELETING_SUFFIX, SessionClass, classify_session, collect_garbage, discard_session,
};
use vox_project::journal::{Journal, Record};
use vox_project::{
    ProjectError, Session, SessionConfig, StoreOptions, TakeMode, TakeWriterOptions,
};

fn new_session(sessions: &Path) -> Session {
    let mut config = SessionConfig::new(RATE);
    config.store = StoreOptions::with_memory_budget(512 * MIB);
    Session::create(sessions, config).unwrap()
}

fn record(session: &mut Session, n: usize) {
    let mut capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&noise(50, n)).unwrap();
    session.commit_take(&capture.finish(), &[]).unwrap();
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn session_dir_is_gone_within_5_s_after_save_and_close() {
    let tmp = TempDir::new("gc-close");
    let mut session = new_session(tmp.path());
    record(&mut session, 5 * RATE as usize);
    session
        .mark_saved(Path::new("/tmp/take.wav"), "wav24")
        .unwrap();
    let dir = session.dir().to_path_buf();
    let t0 = Instant::now();
    session.close().unwrap();
    let elapsed = t0.elapsed();
    assert!(!dir.exists());
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert!(entries(tmp.path()).is_empty());
}

#[test]
fn twenty_cycles_leave_sessions_empty() {
    let tmp = TempDir::new("gc-cycles");
    for i in 0..20 {
        let mut session = new_session(tmp.path());
        record(&mut session, 10_000 + i * 1000);
        session
            .mark_saved(Path::new("/tmp/x.wav"), "wav24")
            .unwrap();
        session.close().unwrap();
    }
    assert!(entries(tmp.path()).is_empty());
}

#[test]
fn refused_close_hands_the_session_back() {
    let tmp = TempDir::new("gc-refused-close");
    let mut session = new_session(tmp.path());
    let dir = session.dir().to_path_buf();
    let journal = session.journal_path().to_path_buf();

    // Refused while recording.
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let err = session.close().unwrap_err();
    assert!(matches!(err.error, ProjectError::NotWhileRecording));
    let mut session = *err.session.expect("a refusal returns the session");
    session.commit_take(&capture.finish(), &[]).unwrap();

    // Refused while a reader/writer shares the store.
    let store = std::sync::Arc::clone(session.store());
    let journal_len = std::fs::metadata(&journal).unwrap().len();
    let err = session.close().unwrap_err();
    assert!(matches!(err.error, ProjectError::StoreInUse));
    assert_eq!(
        std::fs::metadata(&journal).unwrap().len(),
        journal_len,
        "a refused close writes nothing"
    );
    let session = *err.session.expect("a refusal returns the session");

    // Retry once the store is released.
    drop(store);
    session.close().unwrap();
    assert!(!dir.exists());
    assert!(collect_garbage(tmp.path()).deleted.is_empty());
}

#[test]
fn interrupted_deletions_are_completed_at_next_start() {
    let tmp = TempDir::new("gc-interrupted");
    // (a) Crash after the tombstone rename.
    let tomb = tmp.path().join(format!("123-4-5{DELETING_SUFFIX}"));
    std::fs::create_dir_all(tomb.join("takes")).unwrap();
    std::fs::write(tomb.join("chunks.0.f32"), [0u8; 4096]).unwrap();

    // (b) Crash after `close` was journaled but before the directory was removed.
    let mut closed = new_session(tmp.path());
    record(&mut closed, 20_000);
    let closed_dir = closed.dir().to_path_buf();
    let journal = closed.journal_path().to_path_buf();
    drop(closed);
    Journal::open_append(&journal)
        .unwrap()
        .append(&[Record::Close])
        .unwrap();

    // (c) An in-place deletion that removed everything but the journal (which ends in close).
    let mut partial = new_session(tmp.path());
    record(&mut partial, 20_000);
    let partial_dir = partial.dir().to_path_buf();
    let journal = partial.journal_path().to_path_buf();
    drop(partial);
    Journal::open_append(&journal)
        .unwrap()
        .append(&[Record::Close])
        .unwrap();
    for entry in std::fs::read_dir(&partial_dir).unwrap() {
        let path = entry.unwrap().path();
        if path != journal {
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                std::fs::remove_file(path).unwrap();
            }
        }
    }

    let report = collect_garbage(tmp.path());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.deleted.len(), 3, "{:?}", report.deleted);
    assert!(!tomb.exists() && !closed_dir.exists() && !partial_dir.exists());
    assert!(entries(tmp.path()).is_empty());
}

#[test]
fn locked_sessions_are_untouched() {
    let tmp = TempDir::new("gc-locked");
    let mut dirty = new_session(tmp.path());
    record(&mut dirty, 30_000);
    let clean = new_session(tmp.path());
    let before_dirty = entries(dirty.dir());
    let before_clean = entries(clean.dir());

    let report = collect_garbage(tmp.path());
    let mut locked = report.locked.clone();
    locked.sort();
    let mut expected = vec![dirty.dir().to_path_buf(), clean.dir().to_path_buf()];
    expected.sort();
    assert_eq!(locked, expected);
    assert!(report.deleted.is_empty() && report.recoverable.is_empty());
    assert_eq!(entries(dirty.dir()), before_dirty);
    assert_eq!(entries(clean.dir()), before_clean);
    assert!(matches!(
        discard_session(clean.dir()),
        Err(ProjectError::SessionLocked)
    ));
    assert!(clean.dir().exists());
    // Still fully usable.
    record(&mut dirty, 1000);
}

#[test]
fn recoverable_sessions_are_never_deleted_silently() {
    let tmp = TempDir::new("gc-recoverable");
    // Unsaved edit, process "crashed" (dropped without close).
    let mut unsaved = new_session(tmp.path());
    record(&mut unsaved, 30_000);
    let unsaved_dir = unsaved.dir().to_path_buf();
    drop(unsaved);
    // An interrupted take: begun and captured, never committed.
    let mut interrupted = new_session(tmp.path());
    let mut capture = interrupted
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&noise(51, 12_345)).unwrap();
    let interrupted_dir = interrupted.dir().to_path_buf();
    drop(capture);
    drop(interrupted);
    // Clean without close: nothing ever changed, or everything saved.
    let untouched_dir = new_session(tmp.path()).dir().to_path_buf();
    let mut saved = new_session(tmp.path());
    record(&mut saved, 1000);
    saved.mark_saved(Path::new("/tmp/s.wav"), "wav24").unwrap();
    let saved_dir = saved.dir().to_path_buf();
    drop(saved);

    for restart in 0..5 {
        let report = collect_garbage(tmp.path());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut recoverable: Vec<_> = report.recoverable.iter().map(|r| r.dir.clone()).collect();
        recoverable.sort();
        let mut expected = vec![unsaved_dir.clone(), interrupted_dir.clone()];
        expected.sort();
        assert_eq!(recoverable, expected, "restart {restart}");
        if restart == 0 {
            let mut deleted = report.deleted.clone();
            deleted.sort();
            let mut expected = vec![untouched_dir.clone(), saved_dir.clone()];
            expected.sort();
            assert_eq!(deleted, expected);
            let take = report
                .recoverable
                .iter()
                .find(|r| r.dir == interrupted_dir)
                .unwrap();
            assert_eq!(take.open_take, Some(1));
            assert_eq!(take.open_take_samples, 12_345);
            assert!(!take.unsaved_changes);
            let edit = report
                .recoverable
                .iter()
                .find(|r| r.dir == unsaved_dir)
                .unwrap();
            assert!(edit.unsaved_changes);
            assert_eq!(edit.open_take, None);
            assert_eq!(edit.sample_rate_hz, Some(RATE));
            assert!(edit.size_bytes > 0);
        } else {
            assert!(report.deleted.is_empty());
        }
    }
    assert!(unsaved_dir.exists() && interrupted_dir.exists());
    assert!(matches!(
        classify_session(&unsaved_dir).unwrap(),
        SessionClass::Recoverable(_)
    ));
    // Discard (recovery dialog / Settings → Clear) deletes exactly the chosen session.
    discard_session(&unsaved_dir).unwrap();
    assert!(!unsaved_dir.exists() && interrupted_dir.exists());
}

/// A torn take part (crash during rollover, power loss) never hides a take: the parts that parse
/// are summed, and a take whose part 0 exists always keeps its session.
#[test]
fn torn_take_parts_never_hide_a_take() {
    use vox_project::take::take_part_path;
    let tmp = TempDir::new("gc-torn-part");

    // Three parts (1 000 + 1 000 + 500 samples), part 1 torn inside its header.
    let mut session = new_session(tmp.path());
    let options = TakeWriterOptions {
        max_data_bytes: 4000,
        ..TakeWriterOptions::default()
    };
    let mut capture = session.begin_take(TakeMode::New, options).unwrap();
    capture.append(&noise(52, 2500)).unwrap();
    let takes = session.takes_dir();
    let dir = session.dir().to_path_buf();
    drop(capture);
    drop(session);
    std::fs::OpenOptions::new()
        .write(true)
        .open(take_part_path(&takes, 1, 1))
        .unwrap()
        .set_len(10)
        .unwrap();

    // A take whose only part is torn: no readable samples, still a take.
    let mut other = new_session(tmp.path());
    let capture = other
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let other_dir = other.dir().to_path_buf();
    let other_part = take_part_path(&other.takes_dir(), 1, 0);
    drop(capture);
    drop(other);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&other_part)
        .unwrap()
        .set_len(20)
        .unwrap();

    let report = collect_garbage(tmp.path());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.deleted.is_empty(), "{:?}", report.deleted);
    let torn = report.recoverable.iter().find(|r| r.dir == dir).unwrap();
    assert_eq!(torn.open_take, Some(1));
    assert_eq!(torn.open_take_samples, 1500, "parts 0 and 2 still count");
    let empty = report
        .recoverable
        .iter()
        .find(|r| r.dir == other_dir)
        .unwrap();
    assert_eq!(empty.open_take_samples, 0);
    assert!(dir.exists() && other_dir.exists());
}
