//! Chunk store: commit protocol, CRC verification, preallocation failure, peaks, memory budget.

mod common;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use vox_project::{
    CHUNK_ALIGN_BYTES, CHUNK_SAMPLES, ChunkPeaks, ChunkStore, DocSnapshot, Piece, ProjectError,
    SEGMENT_BYTES, SnapshotReader, StoreOptions,
};

#[test]
fn incremental_fnv_matches_testkit() {
    let s = noise(1, 10_000);
    let mut h = Fnv::new();
    h.update(&s[..3333]);
    h.update(&s[3333..]);
    assert_eq!(h.finish(), vox_testkit::golden::fnv1a_hash(&s));
}

#[test]
fn chunks_are_aligned_checksummed_and_read_back_exactly() {
    let tmp = TempDir::new("store-roundtrip");
    let store = store_in(tmp.path(), 512 * MIB);
    let source = noise(2, 3 * CHUNK_SAMPLES + 3_392);
    let written = write_audio(&store, &source);
    assert_eq!(written.len_samples, source.len() as u64);
    assert_eq!(written.chunks, vec![0, 1, 2, 3]);
    assert_eq!(written.pieces.len(), 4);

    for (i, loc) in store.locations().iter().enumerate() {
        assert_eq!(loc.offset % CHUNK_ALIGN_BYTES, 0);
        assert_eq!(
            loc.offset,
            i as u64 * CHUNK_SAMPLES as u64 * 4,
            "full chunks tile"
        );
        let samples = &source[i * CHUNK_SAMPLES..(i * CHUNK_SAMPLES + loc.len as usize)];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(loc.crc32, crc32fast::hash(&bytes), "chunk {i}");
        // Peaks computed at commit are persisted and match a recomputation.
        assert_eq!(
            store.chunk_peaks(loc.id).unwrap(),
            ChunkPeaks::compute(samples)
        );
    }
    assert_eq!(store.segment_count(), 1);
    let file_len = std::fs::metadata(store.chunks_path()).unwrap().len();
    assert_eq!(file_len, SEGMENT_BYTES, "segment preallocated whole");
    #[cfg(target_os = "linux")]
    assert_eq!(store.unreserved_segments(), 0, "fallocate reserved it");
    assert_eq!(store.unsynced_segments(), 1, "committed, not yet synced");
    store.sync().unwrap();
    assert_eq!(store.unsynced_segments(), 0);

    let snapshot = DocSnapshot::new(RATE, written.pieces, Vec::new());
    assert!(bits_equal(&read_all(&store, &snapshot), &source));

    // Streaming in odd block sizes across piece boundaries, plus reads past the end.
    let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::new(snapshot));
    let mut out = Vec::new();
    let mut pos = 0u64;
    for block in [1usize, 4097, 65_535, 70_000, 12_345].iter().cycle() {
        let mut buf = vec![0.0; *block];
        let n = reader.read(pos, &mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
        pos += n as u64;
        if n < *block {
            break;
        }
    }
    assert!(bits_equal(&out, &source));
    assert_eq!(reader.read(pos, &mut [0.0; 8]).unwrap(), 0);
}

#[test]
fn silence_pieces_read_as_zero() {
    let tmp = TempDir::new("store-silence");
    let store = store_in(tmp.path(), 512 * MIB);
    let a = write_audio(&store, &noise(3, 1000));
    let mut pieces = a.pieces.clone();
    pieces.extend(Piece::silence_run(500));
    pieces.extend(a.pieces);
    let snapshot = DocSnapshot::new(RATE, pieces, Vec::new());
    let all = read_all(&store, &snapshot);
    assert_eq!(all.len(), 2500);
    assert!(all[1000..1500].iter().all(|s| s.to_bits() == 0));
    assert!(bits_equal(&all[..1000], &all[1500..]));
}

/// Ticket: "Chunk commit: CRC verified on read".
#[test]
fn corrupted_chunk_fails_its_crc_on_read() {
    let tmp = TempDir::new("store-crc");
    let store = store_in(tmp.path(), 512 * MIB);
    let source = noise(4, 2 * CHUNK_SAMPLES);
    let written = write_audio(&store, &source);
    let loc = store.location(1).unwrap();

    // Flip one byte of chunk 1 on disk behind the store's back.
    {
        use std::os::unix::fs::FileExt;
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(store.chunks_path())
            .unwrap();
        f.write_all_at(&[0x5A], loc.offset + 1000).unwrap();
    }
    let mut buf = vec![0.0; 16];
    assert!(matches!(
        store.read_chunk(1, 0, &mut buf),
        Err(ProjectError::ChecksumMismatch { chunk: 1 })
    ));
    // Stays detected, and the other chunk is intact.
    assert!(matches!(
        store.verify_chunk(1),
        Err(ProjectError::ChecksumMismatch { chunk: 1 })
    ));
    store.verify_chunk(0).unwrap();
    store.read_chunk(0, 0, &mut buf).unwrap();
    assert!(bits_equal(&buf, &source[..16]));
    let snapshot = DocSnapshot::new(RATE, written.pieces, Vec::new());
    let mut all = vec![0.0; snapshot.len_samples as usize];
    assert!(store.read(&snapshot, 0, &mut all).is_err());
    assert!(matches!(
        store.read_chunk(9, 0, &mut buf),
        Err(ProjectError::UnknownChunk(9))
    ));
    assert!(matches!(
        store.read_chunk(0, CHUNK_SAMPLES as u32 - 8, &mut buf),
        Err(ProjectError::OutOfBounds { .. })
    ));
}

/// Ticket: "preallocation makes disk-full an error at segment creation (fault-injected)".
#[test]
fn disk_full_is_an_error_at_segment_creation() {
    let tmp = TempDir::new("store-diskfull");
    let full = Arc::new(AtomicBool::new(false));
    let hook_full = Arc::clone(&full);
    let options = StoreOptions {
        memory_budget_bytes: 512 * MIB,
        segment_hook: Some(Arc::new(move |_seg| {
            if hook_full.load(Ordering::Relaxed) {
                Err(io::Error::from(io::ErrorKind::StorageFull))
            } else {
                Ok(())
            }
        })),
    };
    let store = ChunkStore::create(tmp.path(), 0, options).unwrap();
    let block: Vec<f32> = (0..CHUNK_SAMPLES).map(|i| i as f32 / 65_536.0).collect();
    let per_segment = (SEGMENT_BYTES / (CHUNK_SAMPLES as u64 * 4)) as usize;
    let mut writer = store.writer();
    for _ in 0..per_segment {
        writer.append(&block).unwrap();
    }
    assert_eq!(store.segment_count(), 1);
    assert_eq!(store.chunk_count() as usize, per_segment);

    full.store(true, Ordering::Relaxed);
    let err = writer.append(&block).unwrap_err();
    assert!(err.is_disk_full(), "{err}");
    assert_eq!(err.i18n_key(), "error.disk_full");
    assert_eq!(store.segment_count(), 1);
    assert_eq!(
        store.chunk_count() as usize,
        per_segment,
        "nothing committed"
    );
    assert_eq!(
        std::fs::metadata(store.chunks_path()).unwrap().len(),
        SEGMENT_BYTES
    );
    assert_eq!(
        writer.committed().len_samples,
        (per_segment * CHUNK_SAMPLES) as u64
    );
    drop(writer);
    // Everything committed before the failure is intact.
    let mut out = vec![0.0; CHUNK_SAMPLES];
    store
        .read_chunk(per_segment as u32 - 1, 0, &mut out)
        .unwrap();
    assert!(bits_equal(&out, &block));

    // Space comes back: the next segment is created normally.
    full.store(false, Ordering::Relaxed);
    let mut writer = store.writer();
    writer.append(&block[..100]).unwrap();
    let audio = writer.finish().unwrap();
    assert_eq!(store.segment_count(), 2);
    assert_eq!(
        store.location(audio.chunks[0]).unwrap().offset,
        SEGMENT_BYTES
    );
}

/// SPEC-004 AC-5 at small scale (the 60-min version is `tests/big.rs`): with a one-segment budget,
/// the resident counter never exceeds budget + 2 segments (pinned tail + the reader's segment),
/// LRU segments are evicted and remapped transparently.
#[test]
fn eviction_keeps_resident_memory_within_budget() {
    let tmp = TempDir::new("store-evict");
    let budget = SEGMENT_BYTES;
    let store = store_in(tmp.path(), budget);
    let per_segment = (SEGMENT_BYTES / (CHUNK_SAMPLES as u64 * 4)) as usize;
    // Two full segments + one chunk in the tail segment.
    let source: Vec<f32> = (0..(2 * per_segment + 1) * CHUNK_SAMPLES)
        .map(|i| (i % 100_003) as f32 * 1e-5)
        .collect();
    let written = write_audio(&store, &source);
    assert_eq!(store.segment_count(), 3);
    assert_eq!(
        store.mapped_segments(),
        1,
        "only the tail stays mapped after writing"
    );
    assert!(store.peak_resident_bytes() <= budget + SEGMENT_BYTES);

    let snapshot = Arc::new(DocSnapshot::new(RATE, written.pieces, Vec::new()));
    let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
    let mut rng = vox_testkit::prng::Pcg32::new(5, 1);
    let mut buf = vec![0.0; 4800];
    store.reset_peak_resident();
    for _ in 0..200 {
        let pos = u64::from(rng.next_u32()) % (snapshot.len_samples - 4800);
        assert_eq!(reader.read(pos, &mut buf).unwrap(), 4800);
        let at = pos as usize;
        assert!(bits_equal(&buf, &source[at..at + 4800]), "pos {pos}");
        assert!(store.resident_bytes() <= budget + 2 * SEGMENT_BYTES);
    }
    assert!(store.peak_resident_bytes() <= budget + 2 * SEGMENT_BYTES);

    // Reading segment 0 then 1 evicts 0; the tail is never evicted.
    reader.read(0, &mut buf).unwrap();
    reader.read(SEGMENT_BYTES / 4, &mut buf).unwrap();
    assert_eq!(store.mapped_segments(), 2);
    // A guarded segment survives a zero budget; after release only the pinned tail remains.
    store.set_memory_budget(0);
    assert_eq!(store.mapped_segments(), 2);
    reader.release_segments();
    store.set_memory_budget(0);
    assert_eq!(store.mapped_segments(), 1);
    assert_eq!(store.mapped_bytes(), SEGMENT_BYTES);

    // External reservations count against the budget and push segments out.
    store.set_memory_budget(3 * SEGMENT_BYTES);
    reader.read(0, &mut buf).unwrap();
    reader.release_segments();
    assert_eq!(store.mapped_segments(), 2);
    let reservation = store.reserve_external(2 * SEGMENT_BYTES);
    assert_eq!(store.mapped_segments(), 1);
    assert_eq!(store.resident_bytes(), 3 * SEGMENT_BYTES);
    drop(reservation);
    assert_eq!(store.resident_bytes(), SEGMENT_BYTES);
}

/// H-71 (SPEC-005 §2.3, ADR-003 Amendment 6): [`vox_project::LiveWrittenAudio`] (`ChunkWriter::
/// track_live`) mirrors every commit as it happens — a read taken mid-stream is a strict,
/// bit-exact prefix of what the same range reads once the writer finishes (SPEC-006 AC-13:
/// buckets fill in with real `(min, max)` values, never a different one than the final read
/// gives), and the reported length never shrinks or races ahead of what was actually appended.
#[test]
#[allow(clippy::float_cmp)] // deliberate bit-exact checks
fn live_written_audio_tracks_commits_in_order_and_matches_the_finished_document() {
    let tmp = TempDir::new("store-live-peaks");
    let store = store_in(tmp.path(), 512 * MIB);
    let mut writer = store.writer();
    let live = writer.track_live();
    assert_eq!(live.len_samples(), 0, "nothing committed yet");

    let samples = noise(9, 3 * CHUNK_SAMPLES + 12_345);
    let mut committed_so_far = 0usize;
    let mut last_len = 0u64;
    let mut last_snapshot: Option<DocSnapshot> = None;
    for block in samples.chunks(CHUNK_SAMPLES / 4) {
        writer.append(block).unwrap();
        committed_so_far += block.len();
        let len = live.len_samples();
        assert!(len >= last_len, "live length must never shrink");
        assert!(
            len <= committed_so_far as u64,
            "live length must never race ahead of what was actually appended"
        );
        last_len = len;
        if len > 0 {
            let snapshot = live.snapshot(48_000);
            // Raw bit-exact check: every sample the live handle claims is committed must equal
            // the source, exactly — not just "close" (SPEC-006 AC-13's progressive fill must
            // never show a value the final read later contradicts).
            let got = vox_project::peaks(&store, &snapshot, 1, 0, len as u32).unwrap();
            for (i, &(mn, mx)) in got.iter().enumerate() {
                assert_eq!(mn, mx, "raw pair must be degenerate");
                assert_eq!(mn, samples[i], "sample {i} mismatches the live read");
            }
            last_snapshot = Some(snapshot);
        }
    }
    let final_len = last_len;
    let last_snapshot = last_snapshot.expect("at least one full chunk committed by now");

    let written = writer.finish().unwrap();
    assert_eq!(written.len_samples, samples.len() as u64);
    let final_snapshot = DocSnapshot::new(48_000, written.pieces, Vec::new());

    // Every pyramid level the finished document serves for the prefix the live handle already
    // reported must be bit-exact to what the live handle itself would have answered at that
    // instant — "the final state equals a non-progressive import" (H-71 ticket).
    for &spp in &[64u32, 256, 1024, 4096, 16_384, 65_536] {
        let count = (final_len / u64::from(spp)) as u32;
        if count == 0 {
            continue;
        }
        let from_live = vox_project::peaks(&store, &last_snapshot, spp, 0, count).unwrap();
        let from_final = vox_project::peaks(&store, &final_snapshot, spp, 0, count).unwrap();
        assert_eq!(
            from_live, from_final,
            "spp {spp}: live prefix must match the finished document"
        );
    }
}

#[test]
fn writer_progress_and_cancel() {
    let tmp = TempDir::new("store-cancel");
    let store = store_in(tmp.path(), 512 * MIB);
    let mut writer = store.writer();
    let progress = writer.progress();
    let cancel = writer.cancel_token();
    let data = noise(6, 100_000);
    let handle = std::thread::spawn(move || {
        writer.append(&data).unwrap();
        let seen = writer.samples_written();
        (writer, seen)
    });
    let (mut writer, seen) = handle.join().unwrap();
    assert_eq!(progress.samples_written(), 100_000);
    assert_eq!(seen, 100_000);
    assert_eq!(writer.committed().len_samples, CHUNK_SAMPLES as u64);
    cancel.cancel();
    assert!(matches!(
        writer.append(&[0.0]),
        Err(ProjectError::Cancelled)
    ));
    assert!(matches!(writer.finish(), Err(ProjectError::Cancelled)));
    // The committed chunk stays in the store, unreferenced (reclaimed by compaction, T-301).
    assert_eq!(store.chunk_count(), 1);
}
