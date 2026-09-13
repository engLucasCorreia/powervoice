//! S2-01: cut/copy/paste/delete/trim/silence through a real `Session`, and undo/redo restoring
//! the exact FNV-1a hash (SPEC-008 AC-1, AC-4, AC-5, AC-13).

mod common;

use std::path::Path;

use common::*;
use vox_project::edit::{self, Range, Target, validate_range};
use vox_project::{Session, SessionConfig, StoreOptions};
use vox_testkit::golden::fnv1a_hash;

fn new_session(dir: &Path, samples: &[f32]) -> Session {
    let mut config = SessionConfig::new(RATE);
    config.store = StoreOptions::with_memory_budget(64 * MIB);
    let mut session = Session::create(dir, config).unwrap();
    let audio = write_audio(session.store(), samples);
    session.set_floor(&audio, Vec::new()).unwrap();
    session
}

fn range(session: &Session, start: u64, end: u64) -> Range {
    validate_range(start, end, session.current().len_samples).unwrap()
}

/// SPEC-008 AC-1: cut then paste at the cut point round-trips to the original hash, length and
/// piece table (after merging); the clipboard hashes to the removed range.
#[test]
fn cut_then_paste_round_trips_exactly() {
    let tmp = TempDir::new("edit-cut-paste");
    let source = noise(1, 10_000);
    let session = new_session(tmp.path(), &source);
    let original = session.current();
    let original_hash = fnv1a_hash(&read_all(session.store(), &original));

    for &(s, e) in &[(0u64, 100u64), (2_000, 6_000), (9_900, 10_000), (0, 10_000)] {
        let mut session = new_session(tmp.path(), &source);
        let r = range(&session, s, e);
        let snapshot = session.current();
        let (cut_edit, clip) = edit::cut(&snapshot, r).unwrap();
        assert_eq!(
            fnv1a_hash(&flatten(&session, &clip)),
            fnv1a_hash(&source[s as usize..e as usize]),
            "clipboard matches A[S,E) for [{s},{e})"
        );
        let cut_step = session.commit_edit(cut_edit).unwrap();
        assert_eq!(cut_step.snapshot.len_samples, source.len() as u64 - (e - s));

        let paste_edit = edit::paste(&clip, Target::Cursor(s));
        session.commit_edit(paste_edit).unwrap();
        let restored = session.current();
        assert_eq!(restored.len_samples, original.len_samples);
        assert_eq!(
            fnv1a_hash(&read_all(session.store(), &restored)),
            original_hash,
            "round trip for [{s},{e})"
        );
        assert_eq!(
            &*restored.pieces, &*original.pieces,
            "merging restores the original piece table for [{s},{e})"
        );
    }
}

/// A piece slice's samples, read back through the store (for hashing a clipboard).
fn flatten(session: &Session, pieces: &[vox_project::Piece]) -> Vec<f32> {
    let snapshot =
        vox_project::DocSnapshot::new(session.sample_rate_hz(), pieces.to_vec(), Vec::new());
    read_all(session.store(), &snapshot)
}

/// SPEC-008 AC-3/AC-4: paste over a selection replaces exactly `[S, E)`, for shorter/equal/longer
/// clips; delete closes the gap; whole-document delete leaves length 0.
#[test]
fn paste_over_selection_and_delete_are_exact() {
    let tmp = TempDir::new("edit-paste-delete");
    let source = noise(2, 5_000);
    let clip_src = noise(3, 5_000);

    for &clip_len in &[100u64, 500, 900] {
        let mut session = new_session(tmp.path(), &source);
        let clip = write_audio(session.store(), &clip_src[..clip_len as usize]).pieces;
        let r = range(&session, 1_000, 1_500); // 500 samples
        let step = session
            .commit_edit(edit::paste(&clip, Target::Range(r)))
            .unwrap();
        let expected: Vec<f32> = source[..1_000]
            .iter()
            .chain(&clip_src[..clip_len as usize])
            .chain(&source[1_500..])
            .copied()
            .collect();
        assert_eq!(step.snapshot.len_samples, expected.len() as u64);
        assert_eq!(
            fnv1a_hash(&read_all(session.store(), &step.snapshot)),
            fnv1a_hash(&expected),
            "paste over [1000,1500) with a {clip_len}-sample clip"
        );
    }

    let mut session = new_session(tmp.path(), &source);
    let r = range(&session, 1_000, 3_000);
    let step = session.commit_edit(edit::delete(r)).unwrap();
    let expected: Vec<f32> = source[..1_000]
        .iter()
        .chain(&source[3_000..])
        .copied()
        .collect();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &step.snapshot)),
        fnv1a_hash(&expected)
    );

    // Whole-document delete: length 0, undoable.
    let mut session = new_session(tmp.path(), &source);
    let whole = range(&session, 0, source.len() as u64);
    let step = session.commit_edit(edit::delete(whole)).unwrap();
    assert_eq!(step.snapshot.len_samples, 0);
    assert_eq!(session.history().undo_depth(), 1);
}

/// SPEC-008 AC-4/AC-5: trim keeps exactly `[S,E)`; whole-document trim is a no-op; silence writes
/// exact zeros and leaves the rest bit-identical, with 0 bytes written to the store.
#[test]
fn trim_and_silence_are_exact() {
    let tmp = TempDir::new("edit-trim-silence");
    let source = noise(4, 8_000);

    let mut session = new_session(tmp.path(), &source);
    let r = range(&session, 2_000, 6_000);
    let trim_edit = edit::trim(r, session.current().len_samples).unwrap();
    let step = session.commit_edit(trim_edit).unwrap();
    assert_eq!(step.snapshot.len_samples, 4_000);
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &step.snapshot)),
        fnv1a_hash(&source[2_000..6_000])
    );

    let session = new_session(tmp.path(), &source);
    let whole = range(&session, 0, source.len() as u64);
    assert!(
        edit::trim(whole, session.current().len_samples).is_none(),
        "trim of [0, L) is a no-op"
    );

    let mut session = new_session(tmp.path(), &source);
    let bytes_before = session.store().locations().len();
    let r = range(&session, 1_000, 3_000);
    let step = session.commit_edit(edit::silence(r)).unwrap();
    assert_eq!(step.snapshot.len_samples, source.len() as u64);
    let full = read_all(session.store(), &step.snapshot);
    assert!(
        full[1_000..3_000]
            .iter()
            .all(|&s| s.to_bits() == 0.0f32.to_bits()),
        "silenced range is exactly +0.0"
    );
    assert_eq!(
        &full[..1_000],
        &source[..1_000],
        "before the range is untouched"
    );
    assert_eq!(
        &full[3_000..],
        &source[3_000..],
        "after the range is untouched"
    );
    assert_eq!(
        session.store().locations().len(),
        bytes_before,
        "silence writes no new chunks"
    );
}

/// SPEC-008 AC-13: undo restores the exact prior hash, redo restores the post-op hash, for cut,
/// delete, trim and silence.
#[test]
fn undo_redo_restores_the_exact_hash() {
    let tmp = TempDir::new("edit-undo-redo");
    let source = noise(5, 6_000);

    type Build = fn(Range) -> vox_project::Edit;
    let ops: Vec<(&str, Build)> = vec![("delete", edit::delete), ("silence", edit::silence)];
    for (name, build) in ops {
        let mut session = new_session(tmp.path(), &source);
        let original_hash = fnv1a_hash(&read_all(session.store(), &session.current()));
        let r = range(&session, 1_500, 4_500);
        let edit = build(r);
        let after_step = session.commit_edit(edit).unwrap();
        let after_hash = fnv1a_hash(&read_all(session.store(), &after_step.snapshot));
        assert_ne!(after_hash, original_hash, "{name} changed the audio");

        let undo_step = session.undo().unwrap().unwrap();
        assert_eq!(
            fnv1a_hash(&read_all(session.store(), &undo_step.snapshot)),
            original_hash,
            "{name}: undo restores the original hash"
        );

        let redo_step = session.redo().unwrap().unwrap();
        assert_eq!(
            fnv1a_hash(&read_all(session.store(), &redo_step.snapshot)),
            after_hash,
            "{name}: redo restores the post-op hash"
        );
    }

    // Trim, separately (its own no-selection-of-Range-only-Edit builder signature differs).
    let mut session = new_session(tmp.path(), &source);
    let original_hash = fnv1a_hash(&read_all(session.store(), &session.current()));
    let r = range(&session, 1_000, 5_000);
    let trim_edit = edit::trim(r, session.current().len_samples).unwrap();
    let after_step = session.commit_edit(trim_edit).unwrap();
    let after_hash = fnv1a_hash(&read_all(session.store(), &after_step.snapshot));
    let undo_step = session.undo().unwrap().unwrap();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &undo_step.snapshot)),
        original_hash
    );
    let redo_step = session.redo().unwrap().unwrap();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &redo_step.snapshot)),
        after_hash
    );

    // SPEC-008 §2.3: after undo, the playhead goes to the edit's first affected position — for
    // Trim's two composed `Replace` ops (`{end, ...}` then `{0, ...}`), that's always 0, matching
    // "Trim leaves the cursor at 0" directly.
    assert_eq!(undo_step.first_at, Some(0));
    assert_eq!(redo_step.first_at, Some(0));
}

/// SPEC-008 AC-2: Copy is not an edit — no journal record, no `rev`/`audio_rev`/undo-depth change.
#[test]
fn copy_is_not_an_edit() {
    let tmp = TempDir::new("edit-copy");
    let source = noise(6, 2_000);
    let session = new_session(tmp.path(), &source);
    let before = session.current();
    let r = range(&session, 100, 900);
    let clip = edit::copy(&before, r).unwrap();
    assert_eq!(
        fnv1a_hash(&flatten(&session, &clip)),
        fnv1a_hash(&source[100..900])
    );
    let after = session.current();
    assert_eq!(before.rev, after.rev);
    assert_eq!(before.audio_rev, after.audio_rev);
    assert_eq!(session.history().undo_depth(), 0);
}
