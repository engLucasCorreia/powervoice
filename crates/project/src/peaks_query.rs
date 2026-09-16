//! Peak queries over a document snapshot (ADR-004 §5, ADR-003 `VXPK`): the conservative union of
//! the per-chunk pyramid buckets each output bucket overlaps, or raw samples below the finest
//! pyramid level.

use crate::snapshot::{DocSnapshot, Source};
use crate::store::{ChunkId, ChunkPeaks, ChunkStore, PEAK_LEVELS_SPP};
use crate::{ProjectError, Result};

/// `samples_per_bucket` below which the pyramid has no data: the caller reads raw samples
/// instead (ADR-003 `VXPK` `RAW` flag). Each returned pair is then `(sample, sample)`.
pub const PEAKS_RAW_SPP: u32 = 1;

/// T-704: the pyramid of the chunk the previous output bucket read. Consecutive buckets overlap the
/// same chunk (up to 1 024 of them at the finest level), so one query reads each chunk's ~11 KB
/// pyramid record once instead of once per bucket (a 2126-px view at a fine level: ~9 ms → well
/// under 1 ms per `peaks_get`).
type LastPyramid = Option<(ChunkId, ChunkPeaks)>;

/// `count` buckets of `(min, max)`, each covering `spp` document samples starting at `start`, as
/// the conservative union of the pyramid buckets of every chunk each output bucket overlaps
/// (ADR-004 §5): it never under-reports amplitude, and is exact to within ±1 pyramid bucket in
/// time. `spp` must be [`PEAKS_RAW_SPP`] (raw samples) or one of [`PEAK_LEVELS_SPP`]; any other
/// value is [`ProjectError::InvalidArgument`]. A bucket at or past the end of the document, and
/// silence, read as `(0.0, 0.0)`.
///
/// Buckets not yet computed (`PARTIAL`, ADR-003) don't arise here: this ticket always has the
/// full pyramid available (peaks-during-import progress is deferred, S1-02 ticket scope).
pub fn peaks(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    spp: u32,
    start: u64,
    count: u32,
) -> Result<Vec<(f32, f32)>> {
    if spp == PEAKS_RAW_SPP {
        return peaks_raw(store, snapshot, start, count);
    }
    if !PEAK_LEVELS_SPP.contains(&(spp as usize)) {
        return Err(ProjectError::InvalidArgument(
            "spp must be 1 (raw) or a pyramid level (64, 256, 1024, 4096, 16384, 65536)",
        ));
    }
    let spp64 = u64::from(spp);
    let mut out = Vec::with_capacity(count as usize);
    let mut last: LastPyramid = None;
    for b in 0..u64::from(count) {
        let lo = start.saturating_add(b.saturating_mul(spp64));
        if lo >= snapshot.len_samples {
            out.push((0.0, 0.0));
            continue;
        }
        let hi = lo.saturating_add(spp64).min(snapshot.len_samples);
        out.push(bucket_union(store, snapshot, spp, lo, hi, &mut last)?);
    }
    Ok(out)
}

/// Raw samples `[start, start + count)`, degenerate `(sample, sample)` pairs (past the document
/// end reads as `(0.0, 0.0)`, matching [`ChunkStore::read`]'s clipped-read convention).
fn peaks_raw(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    start: u64,
    count: u32,
) -> Result<Vec<(f32, f32)>> {
    let mut buf = vec![0.0f32; count as usize];
    store.read(snapshot, start, &mut buf)?;
    Ok(buf.into_iter().map(|s| (s, s)).collect())
}

/// The conservative union of pyramid buckets overlapping document range `[lo, hi)`.
fn bucket_union(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    spp: u32,
    lo: u64,
    hi: u64,
    last: &mut LastPyramid,
) -> Result<(f32, f32)> {
    let Some((mut i, mut piece_off)) = snapshot.locate(lo) else {
        return Ok((0.0, 0.0));
    };
    let mut acc = (f32::INFINITY, f32::NEG_INFINITY);
    let mut seen = false;
    let mut pos = lo;
    while pos < hi && i < snapshot.pieces.len() {
        let piece = snapshot.pieces[i];
        let avail = u64::from(piece.len) - piece_off;
        let take = avail.min(hi - pos);
        match piece.source {
            Source::Silence => {
                seen = true;
                acc.0 = acc.0.min(0.0);
                acc.1 = acc.1.max(0.0);
            }
            Source::Chunk(id) => {
                if let Some((mn, mx)) = chunk_range_union(
                    store,
                    id,
                    u64::from(piece.offset) + piece_off,
                    take,
                    spp,
                    last,
                )? {
                    seen = true;
                    acc.0 = acc.0.min(mn);
                    acc.1 = acc.1.max(mx);
                }
            }
        }
        pos += take;
        i += 1;
        piece_off = 0;
    }
    Ok(if seen { acc } else { (0.0, 0.0) })
}

/// The union of chunk `id`'s level-`spp` pyramid buckets overlapping `[c_off, c_off + len)`
/// (chunk-relative); `None` if `len == 0` or that level has no buckets for this chunk.
fn chunk_range_union(
    store: &ChunkStore,
    id: ChunkId,
    c_off: u64,
    len: u64,
    spp: u32,
    last: &mut LastPyramid,
) -> Result<Option<(f32, f32)>> {
    if len == 0 {
        return Ok(None);
    }
    if last.as_ref().is_none_or(|(cached, _)| *cached != id) {
        *last = Some((id, store.chunk_peaks(id)?));
    }
    let Some((_, peaks)) = last.as_ref() else {
        return Ok(None);
    };
    let Some(level) = peaks.level(spp as usize) else {
        return Ok(None);
    };
    if level.is_empty() {
        return Ok(None);
    }
    let spp64 = u64::from(spp);
    let b_lo = (c_off / spp64) as usize;
    let b_hi = ((c_off + len - 1) / spp64) as usize;
    let mut acc = (f32::INFINITY, f32::NEG_INFINITY);
    let mut any = false;
    for &[mn, mx] in &level[b_lo..=b_hi.min(level.len() - 1)] {
        acc.0 = acc.0.min(mn);
        acc.1 = acc.1.max(mx);
        any = true;
    }
    Ok(any.then_some(acc))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::StoreOptions;
    use crate::{DocSnapshot as Snap, Piece};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-peaksquery-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store_with(dir: &std::path::Path, samples: &[f32]) -> (Arc<ChunkStore>, Snap) {
        let store =
            ChunkStore::create(dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024)).unwrap();
        let mut writer = store.writer();
        for block in samples.chunks(10_000) {
            writer.append(block).unwrap();
        }
        let audio = writer.finish().unwrap();
        let snapshot = Snap::new(48_000, audio.pieces, Vec::new());
        (store, snapshot)
    }

    /// T-704: one query reads each chunk's pyramid record at most once, however many output
    /// buckets overlap that chunk. It used to re-read, CRC-check and decode the whole ~11 KB
    /// record for every bucket (~1.1 ms per 1000 buckets at the fine levels, i.e. ~9 ms per
    /// `peaks_get` for a 2126-px view — on every zoom/scroll frame).
    #[test]
    fn a_query_reads_each_chunk_pyramid_record_once() {
        let dir = tmp_dir("reads-once");
        let chunks = 10usize;
        let samples: Vec<f32> = (0..chunks * crate::CHUNK_SAMPLES)
            .map(|i| (i % 977) as f32 / 977.0 - 0.5)
            .collect();
        let (store, snapshot) = store_with(&dir, &samples);
        for &spp in &PEAK_LEVELS_SPP {
            let count = snapshot.len_samples.div_ceil(spp as u64) as u32;
            let before = store.peak_record_reads();
            let out = peaks(&store, &snapshot, spp as u32, 0, count).unwrap();
            assert_eq!(out.len(), count as usize);
            let reads = store.peak_record_reads() - before;
            assert!(
                reads <= chunks as u64,
                "spp {spp}: {reads} pyramid record reads for {count} buckets over {chunks} chunks"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Brute-force min/max over `[start, start+spp*count)`, reading raw samples directly — the
    /// reference the pyramid-accelerated `peaks()` must never under-report against.
    fn brute_force(
        store: &ChunkStore,
        snapshot: &DocSnapshot,
        spp: u32,
        start: u64,
        count: u32,
    ) -> Vec<(f32, f32)> {
        (0..count)
            .map(|b| {
                let lo = start + u64::from(b) * u64::from(spp);
                let hi = (lo + u64::from(spp)).min(snapshot.len_samples);
                if lo >= hi {
                    return (0.0, 0.0);
                }
                let mut buf = vec![0.0f32; (hi - lo) as usize];
                store.read(snapshot, lo, &mut buf).unwrap();
                buf.iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &s| {
                        (mn.min(s), mx.max(s))
                    })
            })
            .collect()
    }

    #[test]
    fn peaks_never_under_report_vs_brute_force_on_random_piece_tables() {
        let dir = tmp_dir("brute");
        let mut rng = vox_testkit::prng::Pcg32::new(11, 3);
        let samples: Vec<f32> = (0..300_000)
            .map(|_| (rng.next_signed() as f32) * 0.9)
            .collect();
        let (store, base) = store_with(&dir, &samples);

        // Build a fragmented piece table: interleave chunk sub-ranges and silence runs (each
        // piece clamped to stay inside one base chunk piece), so pieces don't line up with
        // pyramid bucket boundaries.
        let mut pieces = Vec::new();
        let mut cursor = 0u64;
        let mut toggle = false;
        while cursor < base.len_samples {
            let (i, off) = base.locate(cursor).unwrap();
            let piece = base.pieces[i];
            let remaining_in_piece = u64::from(piece.len) - off;
            let want = 1 + u64::from(rng.next_u32() % 9000);
            let take = want.min(remaining_in_piece) as u32;
            if toggle {
                pieces.push(Piece::silence(take));
            } else {
                match piece.source {
                    Source::Chunk(id) => {
                        pieces.push(Piece::chunk(id, piece.offset + off as u32, take));
                    }
                    Source::Silence => pieces.push(Piece::silence(take)),
                }
            }
            cursor += u64::from(take);
            toggle = !toggle;
        }
        let snapshot = Snap::new(48_000, pieces, Vec::new());

        // H-60: the sweep used to stop at 4096, leaving the two coarsest pyramid levels (16 384,
        // 65 536 — SPEC-006 §2.3's own level set) never checked against the brute-force reference.
        for &spp in &[1u32, 64, 256, 1024, 4096, 16_384, 65_536] {
            let count = 200u32;
            let start = u64::from(rng.next_u32()) % snapshot.len_samples.max(1);
            let fast = peaks(&store, &snapshot, spp, start, count).unwrap();
            let slow = brute_force(&store, &snapshot, spp, start, count);
            for (b, (&(fmn, fmx), &(smn, smx))) in fast.iter().zip(slow.iter()).enumerate() {
                assert!(
                    fmn <= smn + 1e-6,
                    "spp {spp} bucket {b}: fast min {fmn} > brute-force min {smn}"
                );
                assert!(
                    fmx >= smx - 1e-6,
                    "spp {spp} bucket {b}: fast max {fmx} < brute-force max {smx}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check: raw pairs must be (x, x)
    fn raw_mode_returns_exact_samples_as_degenerate_pairs() {
        let dir = tmp_dir("raw");
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32) * 0.001 - 0.5).collect();
        let (store, snapshot) = store_with(&dir, &samples);
        let got = peaks(&store, &snapshot, PEAKS_RAW_SPP, 10, 5).unwrap();
        for (i, &(mn, mx)) in got.iter().enumerate() {
            assert_eq!(mn, mx);
            assert!((mn - samples[10 + i]).abs() < 1e-9);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bucket_at_a_pyramid_level_matches_the_level_directly_for_one_chunk() {
        let dir = tmp_dir("level");
        let samples: Vec<f32> = (0..65_536)
            .map(|i| ((i as f32) * 0.01).sin() * 0.7)
            .collect();
        let (store, snapshot) = store_with(&dir, &samples);
        let got = peaks(&store, &snapshot, 64, 0, 4).unwrap();
        let expected = crate::store::ChunkPeaks::compute(&samples);
        let level = expected.level(64).unwrap();
        for (g, e) in got.iter().zip(level.iter()) {
            assert_eq!(*g, (e[0], e[1]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_an_spp_that_is_not_raw_or_a_pyramid_level() {
        let dir = tmp_dir("badspp");
        let (store, snapshot) = store_with(&dir, &[0.0; 100]);
        assert!(matches!(
            peaks(&store, &snapshot, 3, 0, 1),
            Err(ProjectError::InvalidArgument(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn buckets_past_the_document_end_are_silent() {
        let dir = tmp_dir("past-end");
        let (store, snapshot) = store_with(&dir, &[0.5; 100]);
        let got = peaks(&store, &snapshot, 64, 1000, 3).unwrap();
        assert_eq!(got, vec![(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
