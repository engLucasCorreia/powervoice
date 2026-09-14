//! T-301: undo/redo exactness and speed (SPEC-004 AC-3, AC-8).

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::*;
use vox_project::{
    Edit, EditTarget, Marker, MarkerOp, NormalizeResult, Piece, Session, SessionConfig,
    StoreOptions, WrittenAudio, edit, normalize_peak, validate_range,
};

fn new_session(dir: &Path) -> Session {
    Session::create(
        dir,
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: StoreOptions::with_memory_budget(256 * MIB),
        },
    )
    .unwrap()
}

fn state(s: &Session) -> (u64, Vec<Marker>) {
    let mut fnv = Fnv::new();
    fnv.update(&read_all(s.store(), &s.current()));
    (fnv.finish(), s.current().markers.to_vec())
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        if n == 0 { 0 } else { x % n }
    }
}

/// SPEC-004 AC-3 (scaled: a 30 s document instead of 10 min, to keep scratch small): a seeded
/// random sequence of 200 operations (cut, paste, delete, silence, insert silence, normalize,
/// marker add/rename/move/delete); undoing all of them restores the original audio hash and
/// marker list, redoing all of them the final state.
#[test]
fn ac3_200_random_ops_undo_and_redo_exactly() {
    let dir = TempDir::new("ac3");
    let mut s = new_session(dir.path());
    let audio = write_audio(s.store(), &noise(3, 30 * RATE as usize));
    s.set_floor(&audio, Vec::new()).unwrap();
    let original = state(&s);
    let mut rng = Rng(0x00ac_3ac3);
    let mut clip: Vec<Piece> = Vec::new();
    let mut applied = 0;
    while applied < 200 {
        let len = s.current().len_samples;
        let a = rng.below(len.saturating_sub(1));
        let b = (a + 1 + rng.below(RATE_U64 * 3)).min(len);
        let range = validate_range(a, b, len).ok();
        let ok =
            match (rng.below(9), range) {
                (0, Some(r)) => {
                    let (e, c) = edit::cut(&s.current(), r).unwrap();
                    clip = c;
                    s.commit_edit(e).is_ok()
                }
                (1, _) if !clip.is_empty() => s
                    .commit_edit(edit::paste(&clip, EditTarget::Cursor(rng.below(len + 1))))
                    .is_ok(),
                (2, Some(r)) if len > RATE_U64 * 5 => s.commit_edit(edit::delete(r)).is_ok(),
                (3, Some(r)) => s.commit_edit(edit::silence(r)).is_ok(),
                (4, _) => s
                    .commit_edit(Edit::new("history.insert_silence").replace(
                        rng.below(len + 1),
                        0,
                        Piece::silence_run(1 + rng.below(RATE_U64)),
                    ))
                    .is_ok(),
                (5, Some(r)) => matches!(
                    normalize_peak(&mut s, r, -1.0 - rng.below(6) as f64).unwrap(),
                    NormalizeResult::Applied(_)
                ),
                (6, _) => {
                    let id = s.new_marker_id();
                    s.commit_edit(Edit::new("history.marker_add").marker(MarkerOp::Add(
                        Marker::new(id, rng.below(len + 1), 0, format!("m{}", id.0)),
                    )))
                    .is_ok()
                }
                (7 | 8, _) => {
                    let markers = s.current().markers.clone();
                    if markers.is_empty() {
                        false
                    } else {
                        let m = &markers[rng.below(markers.len() as u64) as usize];
                        let op = match rng.below(3) {
                            0 => MarkerOp::Rename {
                                id: m.id,
                                name: format!("renamed {}", rng.below(100)).into(),
                            },
                            1 => MarkerOp::Move {
                                id: m.id,
                                pos_samples: rng.below(len + 1),
                                len_samples: 0,
                            },
                            _ => MarkerOp::Remove(m.id),
                        };
                        s.commit_edit(Edit::new("history.marker_edit").marker(op))
                            .is_ok()
                    }
                }
                _ => false,
            };
        if ok {
            applied += 1;
        }
    }
    let last = state(&s);
    assert_eq!(s.history().undo_depth(), 200);
    while s.undo().unwrap().is_some() {}
    assert_eq!(state(&s), original, "all undone = the original");
    while s.redo().unwrap().is_some() {}
    assert_eq!(state(&s), last, "all redone = the final state");
}

/// SPEC-004 AC-8: an edit sets dirty, Save clears it, Undo sets it, Redo back to the saved
/// state clears it; the undo depth survives the save.
#[test]
fn ac8_modified_mark_follows_the_saved_state() {
    let dir = TempDir::new("ac8");
    let mut s = new_session(dir.path());
    let audio = write_audio(s.store(), &noise(1, 48_000));
    s.set_floor(&audio, Vec::new()).unwrap();
    assert!(!s.is_dirty());
    s.commit_edit(edit::silence(validate_range(0, 100, 48_000).unwrap()))
        .unwrap();
    assert!(s.is_dirty());
    s.mark_saved(Path::new("/tmp/x.wav"), "wav24").unwrap();
    assert!(!s.is_dirty());
    assert_eq!(s.history().undo_depth(), 1, "history survives the save");
    s.undo().unwrap();
    assert!(s.is_dirty());
    s.redo().unwrap();
    assert!(!s.is_dirty());
}

/// SPEC-004 AC-3 timing: on a 60-min document with 20 000 pieces, each undo or redo command
/// (including its journal `fdatasync`) completes in ≤ 50 ms. SSD-dependent: ignored by default
/// (`cargo test -p vox-project --test history_exact -- --ignored`, with `POWERVOICE_TEST_TMP`
/// on the SSD under test).
#[test]
#[ignore = "bench: SSD timing"]
fn ac3_undo_redo_on_20000_pieces_takes_at_most_50_ms() {
    let dir = TempDir::new("ac3-timing");
    let mut s = new_session(dir.path());
    // 64 real chunks, referenced by 20 000 pieces that add up to 60 min.
    let base = write_audio(s.store(), &noise(5, 64 * vox_project::CHUNK_SAMPLES));
    let chunk_ids: Vec<u32> = base
        .pieces
        .iter()
        .filter_map(|p| match p.source {
            vox_project::Source::Chunk(id) => Some(id),
            vox_project::Source::Silence => None,
        })
        .collect();
    let piece_len = (60 * 60 * RATE_U64 / 20_000) as u32;
    let pieces: Vec<Piece> = (0..20_000u32)
        .map(|i| {
            let id = chunk_ids[i as usize % chunk_ids.len()];
            let offset = (i * 37) % (vox_project::CHUNK_SAMPLES as u32 - piece_len);
            Piece::chunk(id, offset, piece_len)
        })
        .collect();
    let len = pieces.iter().map(Piece::len_samples).sum();
    let floor = WrittenAudio {
        pieces,
        chunks: Vec::new(),
        len_samples: len,
    };
    s.set_floor(&floor, Vec::new()).unwrap();
    assert!(s.current().pieces.len() >= 19_000);
    let mut rng = Rng(7);
    for _ in 0..50 {
        let len = s.current().len_samples;
        let a = rng.below(len - 10_000);
        s.commit_edit(edit::delete(validate_range(a, a + 1_000, len).unwrap()))
            .unwrap();
    }
    let mut worst = Duration::ZERO;
    for _ in 0..50 {
        let t = Instant::now();
        s.undo().unwrap().unwrap();
        worst = worst.max(t.elapsed());
    }
    for _ in 0..50 {
        let t = Instant::now();
        s.redo().unwrap().unwrap();
        worst = worst.max(t.elapsed());
    }
    eprintln!("worst undo/redo on 20 000 pieces: {worst:?}");
    assert!(worst <= Duration::from_millis(50), "{worst:?}");
}
