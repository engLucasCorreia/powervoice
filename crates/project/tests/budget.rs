//! T-301: disk budget, compaction, dropping the oldest undo steps, memory budget (SPEC-004 §2.4,
//! §2.5, AC-4, AC-13; ADR-004 §4).

mod common;

use std::path::Path;

use common::*;
use vox_project::{
    CHUNK_SAMPLES, DiskLimits, FixedFreeSpace, NormalizeResult, ProjectError, SEGMENT_BYTES,
    Session, SessionConfig, StoreOptions, TakeMode, TakeWriterOptions, edit, normalize_peak,
    validate_range,
};

const GIB: u64 = 1024 * MIB;
const SEG: u64 = SEGMENT_BYTES;

fn new_session(sessions: &Path, budget: u64) -> Session {
    Session::create(
        sessions,
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: StoreOptions::with_memory_budget(budget),
        },
    )
    .unwrap()
}

fn hash(s: &Session) -> u64 {
    let mut fnv = Fnv::new();
    fnv.update(&read_all(s.store(), &s.current()));
    fnv.finish()
}

fn normalize_all(s: &mut Session, target_db: f64) {
    let len = s.current().len_samples;
    let r = normalize_peak(s, validate_range(0, len, len).unwrap(), target_db).unwrap();
    assert!(matches!(r, NormalizeResult::Applied(_)));
}

/// A session whose floor is `chunks` full chunks, then `normalizes` whole-file normalizes (each
/// writes `chunks` new chunks).
fn session_with_history(dir: &Path, chunks: usize, normalizes: usize) -> Session {
    let mut s = new_session(dir, 512 * MIB);
    let audio = write_audio(s.store(), &noise(9, chunks * CHUNK_SAMPLES));
    s.set_floor(&audio, Vec::new()).unwrap();
    for i in 0..normalizes {
        normalize_all(&mut s, if i % 2 == 0 { -1.0 } else { -2.0 });
    }
    s
}

fn plenty_free() -> FixedFreeSpace {
    FixedFreeSpace::new(1024 * GIB)
}

/// Limits whose soft limit leaves room for exactly `segments` chunk-file segments.
fn limits_for(s: &Session, segments: u64) -> DiskLimits {
    let u = s.disk_usage(&[]);
    DiskLimits {
        soft_limit_bytes: Some(u.session_bytes - u.chunk_file_bytes + segments * SEG + MIB),
        free_floor_bytes: 0,
    }
}

/// SPEC-004 AC-13 bullet 1: over the limit with ≥ 25 % unreachable, compaction runs first — no
/// undo step removed, no notice — and the result survives a crash (the new generation's
/// journal replays).
#[test]
fn ac13_compaction_first_when_a_quarter_is_unreachable() {
    let dir = TempDir::new("ac13-compact");
    let mut s = session_with_history(dir.path(), 100, 5); // 600 chunks: 3 segments
    for _ in 0..3 {
        s.undo().unwrap().unwrap();
    }
    // A new edit truncates redo: the 3 undone normalizes' chunks are unreachable (50 %).
    s.commit_edit(edit::silence(validate_range(0, 1_000, 10_000).unwrap()))
        .unwrap();
    let (hash_before, undo_before) = (hash(&s), s.history().undo_depth());
    let usage = s.disk_usage(&[]);
    assert!(usage.unreachable_bytes * 4 >= usage.unreachable_bytes + usage.live_after_drop[0]);
    assert_eq!(usage.chunk_file_bytes, 3 * SEG);

    let limits = limits_for(&s, 2);
    let report = s.housekeep(&[], &plenty_free(), &limits).unwrap();
    assert!(report.compacted);
    assert_eq!(report.dropped_undo, 0, "no undo step removed");
    assert_eq!(report.freed_bytes, SEG);
    assert!(!report.almost_full);
    assert_eq!(s.generation(), 1);
    assert_eq!(s.store().chunk_file_bytes(), 2 * SEG);
    assert_eq!(hash(&s), hash_before);
    assert_eq!(s.history().undo_depth(), undo_before);
    // Every undo step still plays back exactly.
    let mut hashes = vec![hash(&s)];
    while s.undo().unwrap().is_some() {
        hashes.push(hash(&s));
    }
    while s.redo().unwrap().is_some() {}
    assert_eq!(hash(&s), hash_before);

    let session_dir = s.dir().to_path_buf();
    drop(s);
    assert!(
        !session_dir.join("chunks.0.f32").exists(),
        "old generation deleted"
    );
    let (mut r, report) = Session::recover(&session_dir, StoreOptions::default()).unwrap();
    assert_eq!(report.lost_changes, 0);
    assert_eq!(r.generation(), 1);
    assert_eq!(hash(&r), hash_before);
    let mut replayed = vec![hash(&r)];
    while r.undo().unwrap().is_some() {
        replayed.push(hash(&r));
    }
    assert_eq!(replayed, hashes);
}

/// SPEC-004 AC-13 bullet 2: with less unreachable, the oldest undo entries are removed until
/// usage is under the limit; the report carries the count and freed size; the current audio
/// and the redo stack are unchanged.
#[test]
fn ac13_otherwise_the_oldest_undo_steps_are_dropped() {
    let dir = TempDir::new("ac13-drop");
    let mut s = session_with_history(dir.path(), 100, 5);
    s.undo().unwrap().unwrap(); // keep one redo entry
    let (hash_before, undo_before) = (hash(&s), s.history().undo_depth());
    let redo_label = s.history().redo_label().map(str::to_owned);
    assert_eq!(s.disk_usage(&[]).unreachable_bytes, 0);

    let limits = limits_for(&s, 2);
    let report = s.housekeep(&[], &plenty_free(), &limits).unwrap();
    assert_eq!(report.dropped_undo, 1, "just enough to fit");
    assert_eq!(report.freed_bytes, SEG);
    assert!(report.compacted);
    assert_eq!(hash(&s), hash_before, "current audio unchanged");
    assert_eq!(s.history().undo_depth(), undo_before - 1);
    assert_eq!(s.history().redo_depth(), 1, "redo untouched");
    assert_eq!(s.history().redo_label().map(str::to_owned), redo_label);
    s.redo().unwrap().unwrap();
    let after_redo = hash(&s);
    s.undo().unwrap().unwrap();

    let session_dir = s.dir().to_path_buf();
    drop(s);
    let (mut r, _) = Session::recover(&session_dir, StoreOptions::default()).unwrap();
    assert_eq!(hash(&r), hash_before);
    assert_eq!(r.history().undo_depth(), undo_before - 1);
    r.redo().unwrap().unwrap();
    assert_eq!(hash(&r), after_redo);
}

/// `drop_undo` is journaled: a crash before any compaction still recovers the shorter history.
#[test]
fn dropped_undo_steps_survive_a_crash_without_compaction() {
    let dir = TempDir::new("drop-journal");
    let mut s = session_with_history(dir.path(), 4, 5);
    assert_eq!(s.drop_oldest_undo(2).unwrap(), 2);
    let (h, depth) = (hash(&s), s.history().undo_depth());
    let session_dir = s.dir().to_path_buf();
    drop(s);
    let (r, _) = Session::recover(&session_dir, StoreOptions::default()).unwrap();
    assert_eq!((hash(&r), r.history().undo_depth()), (h, depth));
    // The classification counts what's left.
    let listed = vox_project::gc::collect_garbage(dir.path()).recoverable;
    assert!(listed.is_empty(), "locked by `r`");
}

/// SPEC-004 AC-13 bullet 3: neither compaction nor undo removal runs while recording.
#[test]
fn ac13_nothing_runs_while_recording() {
    let dir = TempDir::new("ac13-rec");
    let mut s = session_with_history(dir.path(), 100, 5);
    let limits = limits_for(&s, 1);
    let _capture = s
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let depth = s.history().undo_depth();
    let report = s.housekeep(&[], &plenty_free(), &limits).unwrap();
    assert!(report.skipped_recording);
    assert!(!report.compacted && report.dropped_undo == 0);
    assert!(matches!(
        s.compact(&[]),
        Err(ProjectError::NotWhileRecording)
    ));
    assert!(matches!(
        s.drop_oldest_undo(1),
        Err(ProjectError::NotWhileRecording)
    ));
    assert_eq!(s.history().undo_depth(), depth);
    assert_eq!(s.generation(), 0);
}

/// The clipboard's pieces count as live: compaction keeps them, so a paste after it works.
#[test]
fn compaction_keeps_the_clipboard() {
    let dir = TempDir::new("clip");
    let mut s = session_with_history(dir.path(), 4, 1);
    let len = s.current().len_samples;
    let (cut, clip) = edit::cut(&s.current(), validate_range(0, len / 2, len).unwrap()).unwrap();
    s.commit_edit(cut).unwrap();
    let clip_hash = {
        let snap = vox_project::DocSnapshot::new(RATE, clip.clone(), Vec::new());
        let mut fnv = Fnv::new();
        fnv.update(&read_all(s.store(), &snap));
        fnv.finish()
    };
    // Drop all history, then compact: only the current document and the clipboard stay live.
    s.drop_oldest_undo(usize::MAX).unwrap();
    s.compact(&clip).unwrap();
    s.commit_edit(edit::paste(&clip, vox_project::EditTarget::Cursor(0)))
        .unwrap();
    let snap = vox_project::DocSnapshot::new(RATE, clip, Vec::new());
    let mut fnv = Fnv::new();
    fnv.update(&read_all(s.store(), &snap));
    assert_eq!(fnv.finish(), clip_hash);
}

/// A compaction interrupted before the `meta.json` switch leaves the old generation
/// authoritative; recovery removes the half-written new one.
#[test]
fn interrupted_compaction_leftovers_are_removed_by_recovery() {
    let dir = TempDir::new("half-compact");
    let s = session_with_history(dir.path(), 4, 2);
    let h = hash(&s);
    let session_dir = s.dir().to_path_buf();
    drop(s);
    for name in ["chunks.1.f32", "peaks.1.bin", "journal.1.jsonl"] {
        std::fs::write(session_dir.join(name), b"half-written").unwrap();
    }
    let (r, _) = Session::recover(&session_dir, StoreOptions::default()).unwrap();
    assert_eq!(hash(&r), h);
    assert_eq!(r.generation(), 0);
    assert!(!session_dir.join("chunks.1.f32").exists());
    assert!(!session_dir.join("journal.1.jsonl").exists());
}

/// SPEC-004 AC-4 (scaled down: 15 s instead of 60 min, 64 MiB budget): 100 whole-file
/// normalizes all stay undoable, and the store's resident-memory counter never exceeds the
/// budget + 128 MiB. With the disk limit raised, housekeeping does nothing.
#[test]
fn ac4_depth_is_not_bounded_by_memory() {
    let dir = TempDir::new("ac4");
    let budget = 64 * MIB;
    let mut s = new_session(dir.path(), budget);
    let audio = write_audio(s.store(), &noise(12, 15 * RATE as usize));
    s.set_floor(&audio, Vec::new()).unwrap();
    let original = hash(&s);
    for i in 0..100 {
        normalize_all(&mut s, if i % 2 == 0 { -1.0 } else { -1.5 });
    }
    assert_eq!(s.history().undo_depth(), 100);
    let limits = DiskLimits {
        soft_limit_bytes: Some(100 * GIB),
        free_floor_bytes: 0,
    };
    let report = s.housekeep(&[], &plenty_free(), &limits).unwrap();
    assert!(!report.compacted && report.dropped_undo == 0);
    let mut undone = 0;
    while s.undo().unwrap().is_some() {
        undone += 1;
    }
    assert_eq!(undone, 100);
    assert_eq!(hash(&s), original);
    let peak = s.store().peak_resident_bytes();
    assert!(peak <= budget + 128 * MIB, "peak resident {peak}");
}

/// SPEC-004 §2.4: a budget change applies at once (evicts down to it).
#[test]
fn memory_budget_changes_apply_without_a_restart() {
    let dir = TempDir::new("budget-live");
    let s = session_with_history(dir.path(), 300, 1); // 600 chunks over 3 segments
    let _ = hash(&s); // maps every segment
    assert!(s.store().mapped_bytes() >= 2 * SEG);
    s.store().set_memory_budget(SEG);
    assert!(
        s.store().mapped_bytes() <= 2 * SEG,
        "only the tail may stay over"
    );
}
