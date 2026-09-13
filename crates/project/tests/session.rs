//! Session: take → one undo entry (SPEC-002 AC-5 project part), refusal while recording
//! (SPEC-004 AC-15 project side), journal contents, attachments, dirty state.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::*;
use vox_project::journal::{Record, TakeModeRecord, read_journal};
use vox_project::{
    Edit, Marker, MarkerId, MarkerOp, Piece, ProjectError, Session, SessionConfig, StoreOptions,
    TAKE_LABEL_KEY, TakeMode, TakeWriterOptions,
};
use vox_testkit::wav::read_wav_file;

fn new_session(dir: &Path) -> Session {
    let mut config = SessionConfig::new(RATE);
    config.store = StoreOptions::with_memory_budget(512 * MIB);
    Session::create(dir, config).unwrap()
}

/// Records `samples` as a take in 50 ms blocks and commits it with `markers`.
fn record(session: &mut Session, samples: &[f32], markers: Vec<Marker>) {
    let mut capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    for block in samples.chunks(2400) {
        capture.append(block).unwrap();
    }
    let finished = capture.finish();
    assert!(finished.error.is_none());
    assert_eq!(finished.wav_samples, samples.len() as u64);
    session.commit_take(&finished, &markers).unwrap().unwrap();
}

/// SPEC-002 AC-5 (project part): a 5 s take with 2 user markers and 1 dropout is exactly one
/// undo entry labelled "Record"; undo → length 0 and no markers; redo → same hash and markers.
#[test]
fn take_is_one_undo_entry_with_audio_and_markers() {
    let tmp = TempDir::new("session-take");
    let mut session = new_session(tmp.path());
    let floor_rev = session.current().rev;

    // 3 s of noise, a 10 ms dropout filled with silence, then the rest of the 5 s.
    let mut source = noise(31, 3 * RATE as usize);
    source.extend(std::iter::repeat_n(0.0, 480));
    source.extend(noise(32, 2 * RATE as usize - 480));
    let markers = vec![
        Marker::new(session.new_marker_id(), RATE_U64, 0, "Marker 01"),
        Marker::new(session.new_marker_id(), 2 * RATE_U64 + 1234, 0, "Marker 02"),
        Marker::new(session.new_marker_id(), 3 * RATE_U64, 0, "Dropout 10 ms"),
    ];
    record(&mut session, &source, markers.clone());

    let history = session.history();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.undo_label(), Some(TAKE_LABEL_KEY));
    let after = session.current();
    assert_eq!(after.len_samples, 5 * RATE_U64);
    assert_ne!(after.rev, floor_rev);
    let hash = vox_testkit::golden::fnv1a_hash(&read_all(session.store(), &after));
    assert_eq!(hash, vox_testkit::golden::fnv1a_hash(&source));
    assert_eq!(&*after.markers, &markers[..]);
    assert!(session.is_dirty());

    let undo = session.undo().unwrap().unwrap();
    assert!(undo.audio_changed);
    assert_eq!(&*undo.label_key, TAKE_LABEL_KEY);
    assert_eq!(session.current().len_samples, 0);
    assert_eq!(session.current().markers.len(), 0);
    assert!(!session.is_dirty(), "back at the empty floor");

    let redo = session.redo().unwrap().unwrap();
    assert!(redo.audio_changed);
    let redone = session.current();
    assert_eq!(
        vox_testkit::golden::fnv1a_hash(&read_all(session.store(), &redone)),
        hash
    );
    assert_eq!(&*redone.markers, &markers[..]);

    // The take WAV stays as a bit-identical backup.
    let (wav, _) = read_wav_file(session.takes_dir().join("take-0001.wav")).unwrap();
    assert!(bits_equal(&wav, &source));

    // Journal: open, checkpoint, take_begin, chunks, edit(take 1, 3 markers), undo, redo.
    let journal = read_journal(session.journal_path()).unwrap();
    assert!(!journal.damaged);
    let kinds: Vec<&str> = journal
        .records
        .iter()
        .map(|r| match r {
            Record::Open { .. } => "open",
            Record::Checkpoint(_) => "checkpoint",
            Record::TakeBegin { .. } => "take_begin",
            Record::Chunks { .. } => "chunks",
            Record::Edit(_) => "edit",
            Record::Undo { .. } => "undo",
            Record::Redo { .. } => "redo",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "open",
            "checkpoint",
            "take_begin",
            "chunks",
            "edit",
            "undo",
            "redo"
        ]
    );
    let Record::TakeBegin { take, mode, at, .. } = &journal.records[2] else {
        unreachable!()
    };
    assert_eq!((*take, *mode, *at), (1, TakeModeRecord::New, 0));
    let Record::Chunks { chunks } = &journal.records[3] else {
        unreachable!()
    };
    assert_eq!(chunks.len(), 4, "5 s = 240 000 samples = 4 chunks");
    assert_eq!(*chunks, session.store().locations());
    let Record::Edit(edit) = &journal.records[4] else {
        unreachable!()
    };
    assert_eq!(edit.take, Some(1));
    assert_eq!(edit.label_key, TAKE_LABEL_KEY);
    assert_eq!(edit.marker_ops.len(), 3);
}

/// SPEC-004 AC-15 (project side): while a take is open, audio edits, undo and redo return
/// `error.not_while_recording` and change nothing; marker edits still work.
#[test]
fn audio_edits_undo_and_redo_are_refused_while_recording() {
    let tmp = TempDir::new("session-refuse");
    let mut session = new_session(tmp.path());
    record(&mut session, &noise(33, RATE as usize), Vec::new());
    let id = session.new_marker_id();
    session
        .commit_edit(
            Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(id, 100, 0, "a"))),
        )
        .unwrap();
    session.undo().unwrap().unwrap(); // leaves one undo and one redo entry

    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    assert!(session.is_recording());
    let before = session.current();
    let journal_len = std::fs::metadata(session.journal_path()).unwrap().len();
    let depths = (
        session.history().undo_depth(),
        session.history().redo_depth(),
    );

    let cut = Edit::new("history.cut").replace(0, 100, Vec::new());
    for result in [
        session.commit_edit(cut).map(|_| ()),
        session.undo().map(|_| ()),
        session.redo().map(|_| ()),
    ] {
        let err = result.unwrap_err();
        assert!(matches!(err, ProjectError::NotWhileRecording));
        assert_eq!(err.i18n_key(), "error.not_while_recording");
    }
    assert_eq!(session.current().rev, before.rev);
    assert_eq!(
        (
            session.history().undo_depth(),
            session.history().redo_depth()
        ),
        depths
    );
    assert_eq!(
        std::fs::metadata(session.journal_path()).unwrap().len(),
        journal_len,
        "nothing journaled"
    );
    assert!(matches!(
        session.begin_take(TakeMode::New, TakeWriterOptions::default()),
        Err(ProjectError::TakeAlreadyOpen)
    ));

    // Marker-only edits are not destructive and stay available.
    let id = session.new_marker_id();
    session
        .commit_edit(
            Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(id, 200, 0, "b"))),
        )
        .unwrap();

    // After the take is committed, undo works again.
    let mut capture = capture;
    capture.append(&noise(34, 4800)).unwrap();
    session.commit_take(&capture.finish(), &[]).unwrap();
    assert!(!session.is_recording());
    assert_eq!(session.current().len_samples, RATE_U64 + 4800);
    session.undo().unwrap().unwrap();
    assert_eq!(session.current().len_samples, RATE_U64);
}

#[test]
fn empty_take_adds_no_entry_and_discard_closes_it() {
    let tmp = TempDir::new("session-empty-take");
    let mut session = new_session(tmp.path());
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    assert!(
        session
            .commit_take(&capture.finish(), &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(session.history().undo_depth(), 0);
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    session.discard_take(capture.id()).unwrap();
    assert!(!session.is_recording());
    let journal = read_journal(session.journal_path()).unwrap();
    let discards = journal
        .records
        .iter()
        .filter(|r| matches!(r, Record::TakeDiscard { .. }))
        .count();
    assert_eq!(discards, 2);
}

#[test]
fn attachment_survives_journal_and_undo_redo() {
    let tmp = TempDir::new("session-attachment");
    let mut session = new_session(tmp.path());
    record(&mut session, &noise(35, 10_000), Vec::new());
    let baked = write_audio(session.store(), &noise(36, 10_000));
    let rack = br#"{"slots":[{"id":"org.powervoice.gain","params":{"gain_db":-6.0}}]}"#.to_vec();
    let step = session
        .commit_edit(
            Edit::new("history.bake")
                .replace(0, 10_000, baked.pieces)
                .with_attachment(rack.clone()),
        )
        .unwrap();
    assert_eq!(step.attachment.as_deref(), Some(&rack[..]));
    let undo = session.undo().unwrap().unwrap();
    assert_eq!(&*undo.label_key, "history.bake");
    assert_eq!(undo.attachment.as_deref(), Some(&rack[..]));
    let redo = session.redo().unwrap().unwrap();
    assert_eq!(redo.attachment.as_deref(), Some(&rack[..]));
    let journal = read_journal(session.journal_path()).unwrap();
    let bake = journal
        .records
        .iter()
        .find_map(|r| match r {
            Record::Edit(e) if e.label_key == "history.bake" => Some(e.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(bake.to_edit().attachment, Some(rack));
    assert!(bake.take.is_none());
}

#[test]
fn saved_state_and_dirty_flag() {
    let tmp = TempDir::new("session-saved");
    let mut session = new_session(tmp.path());
    record(&mut session, &noise(37, 1000), Vec::new());
    assert!(session.is_dirty());
    session
        .mark_saved(Path::new("/tmp/x.wav"), "wav24")
        .unwrap();
    assert!(!session.is_dirty());
    session.undo().unwrap();
    assert!(session.is_dirty());
    session.redo().unwrap();
    assert!(!session.is_dirty());
    session
        .append_state(serde_json::json!({"view": {"zoom": 1.5}}))
        .unwrap();
    let journal = read_journal(session.journal_path()).unwrap();
    assert!(journal.records.iter().any(|r| matches!(
        r,
        Record::Saved { seq: 1, format, .. } if format == "wav24"
    )));
    assert!(matches!(journal.records.last(), Some(Record::State { .. })));
}

#[test]
fn invalid_edits_are_rejected_without_side_effects() {
    let tmp = TempDir::new("session-invalid");
    let mut session = new_session(tmp.path());
    let journal_len = std::fs::metadata(session.journal_path()).unwrap().len();
    for edit in [
        Edit::new("x").replace(0, 0, vec![Piece::chunk(42, 0, 10)]),
        Edit::new("x").replace(5, 0, vec![Piece::silence(10)]),
        Edit::new("x"),
        Edit::new("x").marker(MarkerOp::Rename {
            id: MarkerId(1),
            name: Arc::from("n"),
        }),
    ] {
        assert!(session.commit_edit(edit).is_err());
    }
    let audio = write_audio(session.store(), &noise(38, 100));
    let mut too_long = audio.pieces.clone();
    too_long[0].len += 1;
    assert!(matches!(
        session.commit_edit(Edit::new("x").replace(0, 0, too_long)),
        Err(ProjectError::OutOfBounds { .. })
    ));
    assert_eq!(session.history().undo_depth(), 0);
    assert_eq!(
        std::fs::metadata(session.journal_path()).unwrap().len(),
        journal_len
    );
}

#[test]
fn cancelled_writer_leaves_the_document_untouched() {
    let tmp = TempDir::new("session-cancel");
    let mut session = new_session(tmp.path());
    record(&mut session, &noise(39, 100_000), Vec::new());
    let before = session.current();
    let mut writer = session.chunk_writer();
    writer.append(&noise(40, 150_000)).unwrap();
    writer.cancel_token().cancel();
    assert!(matches!(writer.finish(), Err(ProjectError::Cancelled)));
    let after = session.current();
    assert_eq!((after.rev, after.audio_rev), (before.rev, before.audio_rev));
    assert_eq!(session.history().undo_depth(), 1);
}

/// A marker a few ms past the final take length (M pressed just before Stop, or audio lost to a
/// store failure) is clamped into the take instead of failing the whole commit.
#[test]
fn take_markers_past_the_end_are_clamped() {
    let tmp = TempDir::new("session-clamp");
    let mut session = new_session(tmp.path());
    let mut capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&noise(41, 48_000)).unwrap();
    let finished = capture.finish();
    let late = Marker::new(session.new_marker_id(), 48_000 + 480, 0, "late");
    let region = Marker::new(session.new_marker_id(), 47_000, 5_000, "region");
    let step = session
        .commit_take(&finished, &[late.clone(), region.clone()])
        .unwrap()
        .unwrap();
    let marker = |id| step.snapshot.markers.iter().find(|m| m.id == id).unwrap();
    assert_eq!(marker(late.id).pos_samples, 48_000);
    let r = marker(region.id);
    assert_eq!((r.pos_samples, r.len_samples), (47_000, 1_000));
    assert_eq!(
        session.store().unsynced_segments(),
        0,
        "chunks synced before the journal referenced them"
    );
}

/// A take commit that fails leaves the take open and the caller's take and markers intact.
#[test]
fn failed_take_commit_can_be_retried() {
    let tmp = TempDir::new("session-retry");
    let mut session = new_session(tmp.path());
    let taken = session.new_marker_id();
    session
        .commit_edit(
            Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(taken, 0, 0, "a"))),
        )
        .unwrap();
    let mut capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    capture.append(&noise(42, 1000)).unwrap();
    let finished = capture.finish();
    let duplicate = [Marker::new(taken, 10, 0, "duplicate id")];
    assert!(matches!(
        session.commit_take(&finished, &duplicate),
        Err(ProjectError::InvalidEdit(_))
    ));
    assert!(session.is_recording(), "the take stays open");
    let fixed = [Marker::new(session.new_marker_id(), 10, 0, "ok")];
    session.commit_take(&finished, &fixed).unwrap().unwrap();
    assert!(!session.is_recording());
    assert_eq!(session.current().len_samples, 1000);
    assert_eq!(session.current().markers.len(), 2);
}

/// SPEC-004 §2.1: an imported file is the undo floor, not an undoable edit.
#[test]
fn imported_audio_is_the_clean_undo_floor() {
    let tmp = TempDir::new("session-floor");
    let mut session = new_session(tmp.path());
    let rev0 = session.current().rev;
    let source = noise(43, 100_000);
    let imported = write_audio(session.store(), &source);
    let cue = Marker::new(MarkerId(7), 50_000, 0, "cue");
    let floor = session.set_floor(&imported, vec![cue.clone()]).unwrap();
    assert!(floor.rev > rev0 && floor.audio_rev > 1);
    assert_eq!(floor.len_samples, 100_000);
    assert_eq!(&*floor.markers, &[cue]);
    assert!(!session.is_dirty());
    assert_eq!(session.history().undo_depth(), 0);
    assert!(bits_equal(&read_all(session.store(), &floor), &source));
    assert_eq!(session.store().unsynced_segments(), 0);

    let journal = read_journal(session.journal_path()).unwrap();
    let n = journal.records.len();
    assert!(matches!(journal.records[n - 2], Record::Chunks { .. }));
    let Record::Checkpoint(checkpoint) = &journal.records[n - 1] else {
        panic!("floor not checkpointed")
    };
    assert_eq!(checkpoint.chunks.len(), imported.chunks.len());
    assert_eq!(checkpoint.current.rev, floor.rev);
    assert!(checkpoint.next_rev > floor.rev && checkpoint.next_audio_rev > floor.audio_rev);
    assert!(!journal.records.iter().any(|r| matches!(r, Record::Edit(_))));
    assert!(
        session.new_marker_id().0 > 7,
        "ids never collide with imported ones"
    );

    session
        .commit_edit(Edit::new("history.cut").replace(0, 10, Vec::new()))
        .unwrap();
    assert!(
        session.set_floor(&imported, Vec::new()).is_err(),
        "only before the first edit"
    );
    session.undo().unwrap().unwrap();
    assert!(session.undo().unwrap().is_none(), "undo stops at the floor");
    assert_eq!(session.current().len_samples, 100_000);
}
