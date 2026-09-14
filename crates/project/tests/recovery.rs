//! T-301: crash recovery (SPEC-004 §2.7, AC-9 fault injection, AC-10, AC-11, AC-12; ADR-004 §9).
//! The kill-point tests that spawn processes are in `crash.rs`.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use common::script::*;
use common::*;
use vox_project::gc::{collect_garbage, discard_session};
use vox_project::{
    Edit, Marker, MarkerOp, NormalizeResult, ProjectError, Session, SessionConfig, SourceInfo,
    TAKE_LABEL_KEY, TakeMode, TakeWriterOptions, edit, normalize_peak, validate_range,
};

// --- AC-9: journal fault injection --------------------------------------------------------------

/// SPEC-004 AC-9 (second bullet, §4): truncating the journal at every byte offset inside its
/// last three records, and separately flipping one byte per record, never panics and recovers
/// exactly the state after the last intact record — with a "could not be recovered" count
/// whenever a change was lost.
#[test]
fn ac9_journal_fault_injection_recovers_the_last_intact_state() {
    let dir = TempDir::new("fault");
    let mut s = new_session(dir.path());
    set_floor(&mut s, &noise(3, 150_000));
    let journal = s.journal_path().to_path_buf();
    let session_dir = s.dir().to_path_buf();
    let journal_len = || std::fs::metadata(&journal).unwrap().len() as usize;
    let mut states = vec![fingerprint(&s)];
    let mut ends = vec![journal_len()];
    let mut rng = Rng::new(11);
    let mut clip = Vec::new();
    let mut names = Vec::new();
    for _ in 0..14 {
        names.push(random_op(&mut s, &mut rng, &mut clip));
        states.push(fingerprint(&s));
        ends.push(journal_len());
    }
    drop(s); // crash: no `close`
    let full = std::fs::read(&journal).unwrap();
    let line_ends: Vec<usize> = full
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'\n')
        .map(|(i, _)| i + 1)
        .collect();
    let expected_at = |cut: usize| {
        let k = ends.iter().rposition(|&e| e <= cut).expect("floor intact");
        &states[k]
    };

    let tail_start = line_ends[line_ends.len() - 4];
    for cut in tail_start..=full.len() {
        std::fs::write(&journal, &full[..cut]).unwrap();
        let (r, report) =
            Session::recover(&session_dir, options()).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        assert_eq!(
            &fingerprint(&r),
            expected_at(cut),
            "cut at {cut} ({names:?})"
        );
        assert_eq!(
            report.lost_changes > 0,
            !ends.contains(&cut),
            "cut at {cut}: lost {}",
            report.lost_changes
        );
        drop(r);
    }

    // One flipped byte per record after the floor.
    let mut start = 0;
    for &end in &line_ends {
        if start >= ends[0] {
            let mut damaged = full.clone();
            damaged[start + (end - start) / 2] ^= 0x01;
            std::fs::write(&journal, &damaged).unwrap();
            let (r, report) = Session::recover(&session_dir, options())
                .unwrap_or_else(|e| panic!("flip in line at {start}: {e}"));
            assert_eq!(&fingerprint(&r), expected_at(start), "flip at {start}");
            assert!(report.lost_changes > 0, "flip at {start}");
            drop(r);
        }
        start = end;
    }
}

/// A recovered session keeps journaling: a second crash after more edits recovers those too.
#[test]
fn recovered_session_keeps_journaling_after_a_torn_tail() {
    let dir = TempDir::new("continue");
    let mut s = new_session(dir.path());
    set_floor(&mut s, &noise(5, 100_000));
    let mut rng = Rng::new(21);
    let mut clip = Vec::new();
    for _ in 0..5 {
        random_op(&mut s, &mut rng, &mut clip);
    }
    let journal = s.journal_path().to_path_buf();
    let session_dir = s.dir().to_path_buf();
    drop(s);
    let mut bytes = std::fs::read(&journal).unwrap();
    bytes.extend_from_slice(b"0badc0de\t{\"type\":\"ed");
    std::fs::write(&journal, &bytes).unwrap();

    let (mut s, report) = Session::recover(&session_dir, options()).unwrap();
    assert_eq!(report.lost_changes, 1);
    for _ in 0..5 {
        random_op(&mut s, &mut rng, &mut clip);
    }
    let expected = fingerprint(&s);
    drop(s);
    let (s, report) = Session::recover(&session_dir, options()).unwrap();
    assert_eq!(fingerprint(&s), expected);
    assert_eq!(report.lost_changes, 0);
}

/// SPEC-004 §2.7 "Damaged data": audio that fails its checksum sends recovery back to the
/// newest state whose audio is intact, with the notice; a damaged undo floor leaves only
/// Discard.
#[test]
fn damaged_audio_falls_back_to_the_newest_intact_state() {
    let dir = TempDir::new("corrupt");
    let mut s = new_session(dir.path());
    set_floor(&mut s, &noise(8, 200_000));
    let floor_chunk = s.store().locations()[0];
    let len = s.current().len_samples;
    let all = validate_range(0, len, len).unwrap();
    assert!(matches!(
        normalize_peak(&mut s, all, -1.0).unwrap(),
        NormalizeResult::Applied(_)
    ));
    let after_first = fingerprint(&s);
    let before = s.store().chunk_count();
    assert!(matches!(
        normalize_peak(&mut s, all, -3.0).unwrap(),
        NormalizeResult::Applied(_)
    ));
    let second_chunk = s.store().location(before).unwrap();
    let chunks_path = s.store().chunks_path().to_path_buf();
    let session_dir = s.dir().to_path_buf();
    drop(s);

    let flip = |offset: u64| {
        let mut bytes = std::fs::read(&chunks_path).unwrap();
        bytes[offset as usize + 8] ^= 0x40;
        std::fs::write(&chunks_path, &bytes).unwrap();
    };
    flip(second_chunk.offset);
    let (r, report) = Session::recover(&session_dir, options()).unwrap();
    assert_eq!(fingerprint(&r), after_first);
    assert_eq!(report.lost_changes, 1);
    drop(r);

    flip(floor_chunk.offset);
    assert!(matches!(
        Session::recover(&session_dir, options()),
        Err(ProjectError::NothingRecoverable)
    ));
}

// --- AC-10: the recovery flow -------------------------------------------------------------------

fn file_facts(path: &Path) -> (u64, u64) {
    let meta = std::fs::metadata(path).unwrap();
    let mtime = meta
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    (meta.len(), mtime)
}

/// A session opened from `source` with 3 unsaved edits and an interrupted 2.5 s take, crashed.
fn crashed_session_with_take(root: &Path, source: &Path) -> (PathBuf, Vec<f32>, String) {
    std::fs::write(source, b"RIFF-original").unwrap();
    let (size_bytes, mtime_unix_ms) = file_facts(source);
    let sessions = root.join("sessions");
    let mut s = Session::create(
        &sessions,
        SessionConfig {
            sample_rate_hz: RATE,
            source: Some(SourceInfo {
                path: source.to_path_buf(),
                size_bytes,
                mtime_unix_ms,
                format: "wav".into(),
            }),
            store: options(),
        },
    )
    .unwrap();
    set_floor(&mut s, &noise(1, 96_000));
    let mut rng = Rng::new(99);
    for i in 0..3 {
        let len = s.current().len_samples;
        let r = random_range(&mut rng, len).unwrap();
        let e = if i == 1 {
            edit::silence(r)
        } else {
            edit::delete(r)
        };
        s.commit_edit(e).unwrap();
    }
    let before_take = fingerprint(&s);
    let take = noise(2, 120_000); // 2.5 s at 48 kHz
    let mut capture = s
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&take).unwrap();
    capture.sync().unwrap();
    drop(capture);
    drop(s); // crash
    (sessions, take, before_take)
}

/// SPEC-004 AC-10: the dialog's facts, "Decide later", the changed-on-disk warning, Recover (3
/// undo steps, modified, bound to the original path) and "Apply as recorded" (one "Record").
#[test]
fn ac10_recovery_flow() {
    let root = TempDir::new("ac10");
    let source = root.path().join("voice.wav");
    let (sessions, take, before_take) = crashed_session_with_take(root.path(), &source);

    let listed = collect_garbage(&sessions).recoverable;
    assert_eq!(listed.len(), 1);
    let r = &listed[0];
    assert_eq!(r.path.as_deref(), Some(source.as_path()));
    assert_eq!(r.unsaved_count, 3, "\"3 unsaved changes\"");
    assert_eq!(r.open_take, Some(1));
    assert_eq!(r.open_take_samples, 120_000, "\"recording 0:02.5\"");
    assert!(r.last_modified.is_some());
    assert!(!r.source_changed && !r.source_missing);

    // "Decide later" keeps it through restarts (AC-12: 5 restarts).
    for _ in 0..5 {
        let again = collect_garbage(&sessions);
        assert!(again.deleted.is_empty());
        assert_eq!(again.recoverable.len(), 1);
    }

    // The source file changed on disk → the warning.
    std::fs::write(&source, b"RIFF-changed-elsewhere").unwrap();
    assert!(collect_garbage(&sessions).recoverable[0].source_changed);

    let (mut s, report) = Session::recover(&listed[0].dir, options()).unwrap();
    assert_eq!(report.bound_path(), Some(source.as_path()));
    assert_eq!(report.lost_changes, 0);
    assert_eq!(report.open_take.unwrap().samples, 120_000);
    let without_take = fingerprint(&s);
    assert_eq!(without_take, before_take);
    assert_eq!(s.history().undo_depth(), 3);
    assert!(s.is_dirty(), "a recovered document opens modified");

    let step = s.apply_open_take_from_wav().unwrap().unwrap();
    assert_eq!(&*step.label_key, TAKE_LABEL_KEY);
    assert_eq!(s.history().undo_depth(), 4);
    let doc = read_all(s.store(), &s.current());
    assert!(bits_equal(&doc[doc.len() - take.len()..], &take));

    // A session in use (this one, now locked) is neither listed nor touched.
    let report = collect_garbage(&sessions);
    assert!(report.recoverable.is_empty());
    assert_eq!(report.locked.len(), 1);
    assert!(matches!(
        discard_session(&listed[0].dir),
        Err(ProjectError::SessionLocked)
    ));
}

/// "Open as new document": the take becomes its own untitled document; the original session
/// keeps its edits, without the take.
#[test]
fn interrupted_take_can_open_as_a_new_document_or_be_discarded() {
    let root = TempDir::new("take-choice");
    let source = root.path().join("voice.wav");
    let (sessions, take, before_take) = crashed_session_with_take(root.path(), &source);
    let dir = collect_garbage(&sessions).recoverable[0].dir.clone();

    let (mut s, _) = Session::recover(&dir, options()).unwrap();
    let fresh = s
        .open_take_as_new_session(&sessions, options())
        .unwrap()
        .expect("a new document");
    assert!(!s.is_recording());
    assert_eq!(fingerprint(&s), before_take);
    assert_eq!(fresh.history().undo_depth(), 1);
    assert_eq!(fresh.history().undo_label(), Some(TAKE_LABEL_KEY));
    assert!(bits_equal(
        &read_all(fresh.store(), &fresh.current()),
        &take
    ));
    drop(fresh);
    drop(s);

    // The original session still has its 3 unsaved edits and no take any more.
    let listed = collect_garbage(&sessions).recoverable;
    let original = listed.iter().find(|r| r.dir == dir).unwrap();
    assert_eq!(original.open_take, None);
    assert_eq!(original.unsaved_count, 3);

    // "Discard take" on a fresh crash.
    let root = TempDir::new("take-discard");
    let source = root.path().join("voice.wav");
    let (sessions, _, before_take) = crashed_session_with_take(root.path(), &source);
    let dir = collect_garbage(&sessions).recoverable[0].dir.clone();
    let (mut s, report) = Session::recover(&dir, options()).unwrap();
    s.discard_take(report.open_take.unwrap().take).unwrap();
    assert!(!s.is_recording());
    assert_eq!(fingerprint(&s), before_take);
}

/// H-10: a take whose commit failed leaves its session (dropped, unlocked) with the take open;
/// the next start offers it and "Apply as recorded" commits it from the WAV — the chunks the
/// capture wrote but never journaled are not trusted.
#[test]
fn h10_failed_commit_session_is_recovered_from_the_wav() {
    let dir = TempDir::new("h10");
    let mut s = new_session(dir.path());
    set_floor(&mut s, &noise(4, 50_000));
    let base = read_all(s.store(), &s.current());
    let take = noise(6, 70_000);
    let mut capture = s
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&take).unwrap();
    let finished = capture.finish();
    assert!(finished.error.is_none());
    // `DocumentService::commit_take` gave up: the session is dropped with the take open.
    drop(finished);
    drop(s);

    let listed = collect_garbage(dir.path()).recoverable;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].open_take_samples, 70_000);
    let (mut s, _) = Session::recover(&listed[0].dir, options()).unwrap();
    s.apply_open_take_from_wav().unwrap().unwrap();
    let mut expected = base;
    expected.extend_from_slice(&take);
    assert!(bits_equal(&read_all(s.store(), &s.current()), &expected));
}

// --- AC-12 ---------------------------------------------------------------------------------------

/// Discard deletes the session; a session in use is refused.
#[test]
fn discard_removes_exactly_the_session() {
    let root = TempDir::new("discard");
    let source = root.path().join("voice.wav");
    let (sessions, _, _) = crashed_session_with_take(root.path(), &source);
    let dir = collect_garbage(&sessions).recoverable[0].dir.clone();
    let live = new_session(&sessions);
    discard_session(&dir).unwrap();
    assert!(!dir.exists());
    assert!(live.dir().exists());
    assert!(matches!(
        discard_session(live.dir()),
        Err(ProjectError::SessionLocked)
    ));
}

// --- AC-11: recovery time (bench) ----------------------------------------------------------------

/// SPEC-004 AC-11: a 60-min session with 1 000 journal records is editable within 5 s of
/// Recover, CRC verification of all referenced audio included. ~700 MB of scratch: run with
/// `just test-big` (or `--ignored`, `POWERVOICE_TEST_TMP` on a real disk).
#[test]
#[ignore = "bench: 60-min session (~700 MB scratch)"]
fn ac11_recovering_a_60_min_session_with_1000_records_takes_under_5_s() {
    let dir = TempDir::new("ac11");
    let mut s = new_session(dir.path());
    let minutes = 60;
    let mut writer = s.chunk_writer();
    for block in 0..(minutes * 60) {
        writer.append(&noise(block, RATE as usize)).unwrap();
    }
    let audio = writer.finish().unwrap();
    s.set_floor(&audio, Vec::new()).unwrap();
    let mut rng = Rng::new(1);
    let mut clip = Vec::new();
    let mut records = 0;
    while records < 1000 {
        let len = s.current().len_samples;
        if rng.below(4) == 0 {
            random_op(&mut s, &mut rng, &mut clip);
        } else {
            let id = s.new_marker_id();
            s.commit_edit(
                Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(
                    id,
                    rng.below(len),
                    0,
                    "m",
                ))),
            )
            .unwrap();
        }
        records += 1;
    }
    let session_dir = s.dir().to_path_buf();
    drop(s);
    let started = Instant::now();
    let (s, report) = Session::recover(&session_dir, options()).unwrap();
    let elapsed = started.elapsed();
    eprintln!("recovered {records} records over 60 min in {elapsed:?}");
    assert_eq!(report.lost_changes, 0);
    assert!(s.history().undo_depth() > 0);
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}
