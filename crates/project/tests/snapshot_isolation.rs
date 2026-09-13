//! SPEC-004 AC-1: a reader streaming revision r of a 10-min document reads exactly r while 50
//! audio edits commit (FNV-1a equal to r read with no concurrent edits).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use vox_project::{Edit, Session, SessionConfig, SnapshotReader, StoreOptions};
use vox_testkit::prng::Pcg32;

const TEN_MIN: usize = 10 * 60 * RATE as usize;
const EDITS: usize = 50;
const BLOCK: usize = 48_000;

#[test]
fn reader_is_isolated_from_concurrent_commits() {
    let tmp = TempDir::new("ac1");
    let mut config = SessionConfig::new(RATE);
    // A small budget (2 segments for a 115 MB document plus growing edit data) makes eviction
    // and remapping happen while the reader streams.
    config.store = StoreOptions::with_memory_budget(128 * MIB);
    let mut session = Session::create(tmp.path(), config).unwrap();

    let source = noise(11, TEN_MIN);
    let reference_hash = vox_testkit::golden::fnv1a_hash(&source);
    let imported = write_audio(session.store(), &source);
    session.set_floor(&imported, Vec::new()).unwrap();
    assert!(
        !session.is_dirty(),
        "the imported file is the clean undo floor"
    );
    assert_eq!(session.history().undo_depth(), 0);
    let r = session.current();
    assert_eq!(r.len_samples, TEN_MIN as u64);
    assert_eq!(
        Fnv::from_reader(session.store(), &r),
        reference_hash,
        "r read with no concurrent edits"
    );

    let edits_done = Arc::new(AtomicUsize::new(0));
    let reader_thread = {
        let store = Arc::clone(session.store());
        let r = Arc::clone(&r);
        let edits_done = Arc::clone(&edits_done);
        std::thread::spawn(move || {
            let mut reader = SnapshotReader::new(store, r);
            let blocks = TEN_MIN.div_ceil(BLOCK);
            let mut hash = Fnv::new();
            let mut buf = vec![0.0; BLOCK];
            let mut pos = 0u64;
            for b in 0..blocks {
                // Interleave: block b is read only after ~b/blocks of the edits committed.
                let wanted = (b * EDITS / blocks).min(EDITS);
                while edits_done.load(Ordering::Acquire) < wanted {
                    std::thread::yield_now();
                }
                let n = reader.read(pos, &mut buf).unwrap();
                hash.update(&buf[..n]);
                pos += n as u64;
            }
            (hash.finish(), pos, edits_done.load(Ordering::Acquire))
        })
    };

    let mut rng = Pcg32::new(12, 3);
    for i in 0..EDITS {
        let len = session.current().len_samples;
        let at = u64::from(rng.next_u32()) % (len - 100_000);
        let edit = if i % 2 == 0 {
            // Destructive replacement with new audio (normalize-like): writes new chunks.
            let fresh = write_audio(session.store(), &noise(100 + i as u64, 70_000));
            Edit::new("history.normalize").replace(at, 70_000, fresh.pieces)
        } else {
            // Cut + paste elsewhere: a pure piece splice.
            let clip = session.current().slice(at, 10_000).unwrap();
            let to = u64::from(rng.next_u32()) % (len - 10_000);
            Edit::new("history.move")
                .replace(at, 10_000, Vec::new())
                .replace(to, 0, clip)
        };
        session.commit_edit(edit).unwrap();
        edits_done.fetch_add(1, Ordering::Release);
    }

    let (hash, samples_read, edits_seen) = reader_thread.join().unwrap();
    assert_eq!(samples_read, TEN_MIN as u64);
    assert_eq!(
        edits_seen, EDITS,
        "all edits committed while the reader streamed"
    );
    assert_eq!(hash, reference_hash, "reader saw exactly revision r");
    let now = session.current();
    assert_eq!(session.history().undo_depth(), EDITS);
    assert_ne!(now.audio_rev, r.audio_rev);
    assert_ne!(Fnv::from_reader(session.store(), &now), reference_hash);
    // r is still fully readable after the edits.
    assert_eq!(Fnv::from_reader(session.store(), &r), reference_hash);
}

trait FromReader {
    fn from_reader(
        store: &Arc<vox_project::ChunkStore>,
        snapshot: &Arc<vox_project::DocSnapshot>,
    ) -> u64;
}

impl FromReader for Fnv {
    fn from_reader(
        store: &Arc<vox_project::ChunkStore>,
        snapshot: &Arc<vox_project::DocSnapshot>,
    ) -> u64 {
        let mut reader = SnapshotReader::new(Arc::clone(store), Arc::clone(snapshot));
        let mut hash = Fnv::new();
        let mut buf = vec![0.0; 1 << 16];
        let mut pos = 0;
        loop {
            let n = reader.read(pos, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            pos += n as u64;
        }
        hash.finish()
    }
}
