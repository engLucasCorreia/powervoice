//! Per-chunk min/max peak pyramids (ADR-004 §5).
//!
//! Levels are 64, 256, 1024, 4096, 16 384 and 65 536 samples per bucket: 1 024 + 256 + 64 + 16 +
//! 4 + 1 = 1 365 `[min, max]` buckets per chunk. Each pyramid is stored as a fixed-size record at
//! `id × PEAK_RECORD_BYTES` in `peaks.<gen>.bin`: `[id u32][len u32][crc32 u32][flags u32]`
//! followed by the buckets as f32 LE pairs. The CRC covers `len` and the buckets (not `flags`, so
//! a pre-H-09 record — `flags` always `0` — still validates under the current code, decoding as
//! `has_non_finite: false`). A torn or missing record (a hole reads as zeros, i.e. `len = 0`) is
//! detected and recomputed.

use std::fs::File;
use std::io;

use crate::CHUNK_SAMPLES;
use crate::fs_util::{read_exact_at, write_all_at};
use crate::store::ChunkId;

/// Samples per bucket of each pyramid level, finest first.
pub const PEAK_LEVELS_SPP: [usize; 6] = [64, 256, 1024, 4096, 16_384, 65_536];
/// Buckets per chunk over all levels.
pub const PEAK_BUCKETS_PER_CHUNK: usize = 1365;
const LEVEL_OFFSETS: [usize; 6] = [0, 1024, 1280, 1344, 1360, 1364];
const RECORD_HEADER_BYTES: usize = 16;
pub(crate) const PEAK_RECORD_BYTES: usize = RECORD_HEADER_BYTES + PEAK_BUCKETS_PER_CHUNK * 8;

/// The min/max pyramid of one chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkPeaks {
    len_samples: u32,
    has_non_finite: bool,
    buckets: Box<[[f32; 2]]>,
}

impl ChunkPeaks {
    /// Computes the pyramid of up to 65 536 samples. Buckets past the end of a partial chunk are
    /// `[0, 0]`; a bucket whose samples are all NaN is `[0, 0]` too (min/max ignore NaN, so a
    /// mixed finite/NaN bucket silently loses the NaN from its own `[min, max]` — that is exactly
    /// why [`Self::has_non_finite`] is tracked separately here, over every raw sample, rather than
    /// inferred from the buckets later: SPEC-010 §4 step 2's pyramid-accelerated peak scan
    /// (`vox_project::normalize::scan_peak_accelerated`) relies on this flag to still catch a
    /// non-finite sample without a raw read).
    pub fn compute(samples: &[f32]) -> ChunkPeaks {
        let n = samples.len().min(CHUNK_SAMPLES);
        let samples = &samples[..n];
        // T-704: branch-free (no early exit) so it vectorizes; the pyramid used to be ~40 % of a
        // 60-min import.
        let has_non_finite = samples.iter().fold(false, |acc, s| acc | !s.is_finite());
        let mut buckets = vec![[0.0f32; 2]; PEAK_BUCKETS_PER_CHUNK].into_boxed_slice();
        for (bucket, block) in buckets.iter_mut().zip(samples.chunks(PEAK_LEVELS_SPP[0])) {
            *bucket = finish(min_max(block));
        }
        for level in 1..PEAK_LEVELS_SPP.len() {
            let valid = n.div_ceil(PEAK_LEVELS_SPP[level]);
            let child_valid = n.div_ceil(PEAK_LEVELS_SPP[level - 1]);
            let (lower, upper) = buckets.split_at_mut(LEVEL_OFFSETS[level]);
            let children = &lower[LEVEL_OFFSETS[level - 1]..LEVEL_OFFSETS[level - 1] + child_valid];
            for (parent, group) in upper[..valid].iter_mut().zip(children.chunks(4)) {
                *parent = finish(
                    group
                        .iter()
                        .fold(EMPTY, |acc, b| [acc[0].min(b[0]), acc[1].max(b[1])]),
                );
            }
        }
        ChunkPeaks {
            len_samples: n as u32,
            has_non_finite,
            buckets,
        }
    }

    /// Number of samples the pyramid describes.
    pub fn len_samples(&self) -> u32 {
        self.len_samples
    }

    /// `true` if any of the chunk's raw samples (at the time [`Self::compute`] ran) was NaN or
    /// infinite — tracked directly over the samples, since a `[min, max]` bucket alone can't
    /// always tell (see [`Self::compute`]'s doc comment).
    pub fn has_non_finite(&self) -> bool {
        self.has_non_finite
    }

    /// The exact `[min, max]` over the *whole* chunk (all `len_samples` of it): the coarsest
    /// pyramid level's one bucket, since [`PEAK_LEVELS_SPP`]'s top level (65 536) equals
    /// `CHUNK_SAMPLES` (SPEC-010 §4 step 2's pyramid acceleration: a piece referencing a whole,
    /// untouched chunk can read its peak from here in O(1) instead of a raw scan).
    pub fn overall(&self) -> [f32; 2] {
        self.buckets[*LEVEL_OFFSETS.last().expect("PEAK_LEVELS_SPP is non-empty")]
    }

    /// The valid `[min, max]` buckets of the level with `spp` samples per bucket
    /// (`ceil(len / spp)` of them), or `None` if `spp` is not a pyramid level.
    pub fn level(&self, spp: usize) -> Option<&[[f32; 2]]> {
        let level = PEAK_LEVELS_SPP.iter().position(|&s| s == spp)?;
        let valid = (self.len_samples as usize).div_ceil(spp);
        self.buckets
            .get(LEVEL_OFFSETS[level]..LEVEL_OFFSETS[level] + valid)
    }

    pub(crate) fn write_record(&self, file: &File, id: ChunkId) -> io::Result<()> {
        let mut buf = vec![0u8; PEAK_RECORD_BYTES];
        buf[0..4].copy_from_slice(&id.to_le_bytes());
        buf[4..8].copy_from_slice(&self.len_samples.to_le_bytes());
        let flags: u32 = u32::from(self.has_non_finite);
        buf[12..16].copy_from_slice(&flags.to_le_bytes());
        let (records, _) = buf[RECORD_HEADER_BYTES..].as_chunks_mut::<8>();
        for (bucket, out) in self.buckets.iter().zip(records) {
            out[0..4].copy_from_slice(&bucket[0].to_le_bytes());
            out[4..8].copy_from_slice(&bucket[1].to_le_bytes());
        }
        let crc = record_crc(&buf);
        buf[8..12].copy_from_slice(&crc.to_le_bytes());
        write_all_at(file, &buf, u64::from(id) * PEAK_RECORD_BYTES as u64)
    }

    pub(crate) fn read_record(file: &File, id: ChunkId) -> Option<ChunkPeaks> {
        let mut buf = vec![0u8; PEAK_RECORD_BYTES];
        read_exact_at(file, &mut buf, u64::from(id) * PEAK_RECORD_BYTES as u64).ok()?;
        Self::decode(&buf, id)
    }

    fn decode(buf: &[u8], id: ChunkId) -> Option<ChunkPeaks> {
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?))
        };
        if buf.len() != PEAK_RECORD_BYTES || word(0)? != id {
            return None;
        }
        let len_samples = word(4)?;
        if len_samples == 0 || len_samples as usize > CHUNK_SAMPLES || word(8)? != record_crc(buf) {
            return None;
        }
        // Not covered by the CRC (see the module doc comment): a pre-H-09 record reads as `0`,
        // i.e. `has_non_finite: false`.
        let has_non_finite = word(12)? & 1 != 0;
        let (records, _) = buf[RECORD_HEADER_BYTES..].as_chunks::<8>();
        let buckets = records
            .iter()
            .map(|&[a, b, c, d, e, f, g, h]| {
                [
                    f32::from_le_bytes([a, b, c, d]),
                    f32::from_le_bytes([e, f, g, h]),
                ]
            })
            .collect();
        Some(ChunkPeaks {
            len_samples,
            has_non_finite,
            buckets,
        })
    }
}

const EMPTY: [f32; 2] = [f32::INFINITY, f32::NEG_INFINITY];

/// Independent accumulators of [`min_max`] (T-704).
const LANES: usize = 8;

/// `[min, max]` of `block` ignoring NaN ([`EMPTY`] if every sample is NaN): the per-sample fold
/// `[acc[0].min(s), acc[1].max(s)]` rewritten as [`LANES`] independent compare-and-select
/// accumulators, which compile to packed min/max instead of one serial dependency chain (T-704:
/// ~8× faster). A NaN sample never replaces an accumulator (`NaN < x` is false) and the
/// accumulators start at ±inf, so the result equals the fold's (up to the sign of a zero).
fn min_max(block: &[f32]) -> [f32; 2] {
    let mut lo = [f32::INFINITY; LANES];
    let mut hi = [f32::NEG_INFINITY; LANES];
    let (lanes, rest) = block.as_chunks::<LANES>();
    for chunk in lanes {
        for ((l, h), &s) in lo.iter_mut().zip(hi.iter_mut()).zip(chunk) {
            *l = if s < *l { s } else { *l };
            *h = if s > *h { s } else { *h };
        }
    }
    let mut acc = EMPTY;
    for (&l, &h) in lo.iter().zip(&hi).chain(rest.iter().zip(rest)) {
        acc[0] = if l < acc[0] { l } else { acc[0] };
        acc[1] = if h > acc[1] { h } else { acc[1] };
    }
    acc
}

fn finish(acc: [f32; 2]) -> [f32; 2] {
    if acc[0] > acc[1] { [0.0, 0.0] } else { acc }
}

fn record_crc(buf: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&buf[4..8]);
    hasher.update(&buf[RECORD_HEADER_BYTES..]);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The straightforward per-sample fold (the pre-T-704 `compute`): the reference the
    /// vectorized version must equal bucket for bucket.
    fn reference_compute(samples: &[f32]) -> (Vec<[f32; 2]>, bool) {
        let n = samples.len().min(CHUNK_SAMPLES);
        let samples = &samples[..n];
        let has_non_finite = samples.iter().any(|s| !s.is_finite());
        let mut buckets = vec![[0.0f32; 2]; PEAK_BUCKETS_PER_CHUNK];
        for (bucket, block) in buckets.iter_mut().zip(samples.chunks(PEAK_LEVELS_SPP[0])) {
            *bucket = finish(
                block
                    .iter()
                    .fold(EMPTY, |acc, &s| [acc[0].min(s), acc[1].max(s)]),
            );
        }
        for level in 1..PEAK_LEVELS_SPP.len() {
            let valid = n.div_ceil(PEAK_LEVELS_SPP[level]);
            let child_valid = n.div_ceil(PEAK_LEVELS_SPP[level - 1]);
            let (lower, upper) = buckets.split_at_mut(LEVEL_OFFSETS[level]);
            let children = &lower[LEVEL_OFFSETS[level - 1]..LEVEL_OFFSETS[level - 1] + child_valid];
            for (parent, group) in upper[..valid].iter_mut().zip(children.chunks(4)) {
                *parent = finish(
                    group
                        .iter()
                        .fold(EMPTY, |acc, b| [acc[0].min(b[0]), acc[1].max(b[1])]),
                );
            }
        }
        (buckets, has_non_finite)
    }

    /// T-704: the vectorized `compute` equals the per-sample fold on random data with NaN, ±inf
    /// and ±0 sprinkled in, an all-NaN bucket, and every chunk-length class.
    #[test]
    #[allow(clippy::float_cmp)] // exact equality is the point (±0 compare equal)
    fn compute_matches_the_reference_fold() {
        let mut rng = vox_testkit::prng::Pcg32::new(0x7704, 1);
        let specials = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0];
        for len in [1usize, 7, 63, 64, 65, 1000, 4097, 65_535, CHUNK_SAMPLES] {
            for with_specials in [false, true] {
                let mut samples: Vec<f32> = (0..len)
                    .map(|_| {
                        if with_specials && rng.next_u32().is_multiple_of(97) {
                            specials[(rng.next_u32() % 5) as usize]
                        } else {
                            rng.next_signed() as f32
                        }
                    })
                    .collect();
                if with_specials && len >= 128 {
                    samples[64..128].fill(f32::NAN);
                }
                let got = ChunkPeaks::compute(&samples);
                let (want, non_finite) = reference_compute(&samples);
                assert_eq!(got.has_non_finite(), non_finite, "len {len}");
                assert_eq!(got.buckets.len(), want.len());
                for (i, (g, w)) in got.buckets.iter().zip(&want).enumerate() {
                    assert!(
                        g[0] == w[0] && g[1] == w[1],
                        "len {len}, specials {with_specials}, bucket {i}: {g:?} vs {w:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact min/max of known sample values
    fn pyramid_levels_are_unions_of_the_samples() {
        let samples: Vec<f32> = (0..CHUNK_SAMPLES)
            .map(|i| ((i as f32) * 0.001).sin() * 0.5)
            .collect();
        let peaks = ChunkPeaks::compute(&samples);
        for &spp in &PEAK_LEVELS_SPP {
            let level = peaks.level(spp).unwrap();
            assert_eq!(level.len(), CHUNK_SAMPLES / spp);
            for (b, bucket) in level.iter().enumerate() {
                let block = &samples[b * spp..(b + 1) * spp];
                let min = block.iter().copied().fold(f32::INFINITY, f32::min);
                let max = block.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                assert_eq!(*bucket, [min, max], "spp {spp} bucket {b}");
            }
        }
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact values
    fn partial_chunk_has_ceil_buckets() {
        let samples = vec![0.25f32; 100];
        let peaks = ChunkPeaks::compute(&samples);
        assert_eq!(peaks.level(64).unwrap().len(), 2);
        assert_eq!(peaks.level(65_536).unwrap(), &[[0.25, 0.25]]);
        assert!(peaks.level(128).is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact min/max of known sample values
    fn overall_is_the_exact_min_max_of_the_whole_chunk() {
        let samples: Vec<f32> = (0..1000)
            .map(|i| ((i as f32) * 0.007).sin() * 0.8)
            .collect();
        let peaks = ChunkPeaks::compute(&samples);
        let min = samples.iter().copied().fold(f32::INFINITY, f32::min);
        let max = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(peaks.overall(), [min, max]);
    }

    #[test]
    fn has_non_finite_is_set_only_when_a_sample_is_nan_or_infinite() {
        let clean = vec![0.1f32, -0.2, 0.3];
        assert!(!ChunkPeaks::compute(&clean).has_non_finite());

        let with_nan = vec![0.1f32, f32::NAN, 0.3];
        assert!(ChunkPeaks::compute(&with_nan).has_non_finite());

        let with_inf = vec![0.1f32, f32::INFINITY, 0.3];
        assert!(ChunkPeaks::compute(&with_inf).has_non_finite());
    }

    #[test]
    fn has_non_finite_round_trips_through_a_record() {
        let samples = vec![0.1f32, f32::NAN, 0.3];
        let peaks = ChunkPeaks::compute(&samples);
        assert!(peaks.has_non_finite());
        let path = std::env::temp_dir().join(format!(
            "vox-project-peaks-nonfinite-{}",
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        peaks.write_record(&file, 7).unwrap();
        let read_back = ChunkPeaks::read_record(&file, 7).unwrap();
        assert!(read_back.has_non_finite());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    /// A record written before H-09 always has its reserved header word `0`. Since the CRC
    /// doesn't cover that word (module doc comment), such a record still decodes as valid — as
    /// `has_non_finite: false`, the documented limitation of a pre-H-09 record even when the
    /// chunk it describes did contain a non-finite sample.
    #[test]
    fn a_pre_h09_record_with_a_zeroed_reserved_word_decodes_as_not_non_finite() {
        let samples = vec![0.1f32, f32::NAN, 0.3];
        let peaks = ChunkPeaks::compute(&samples);
        assert!(peaks.has_non_finite());
        let path =
            std::env::temp_dir().join(format!("vox-project-peaks-legacy-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        peaks.write_record(&file, 2).unwrap();
        // Zero the reserved/flags word directly, as a pre-H-09 writer would have left it — the
        // CRC doesn't cover it, so the record still validates.
        write_all_at(&file, &[0, 0, 0, 0], 2 * PEAK_RECORD_BYTES as u64 + 12).unwrap();
        let read_back = ChunkPeaks::read_record(&file, 2).unwrap();
        assert!(!read_back.has_non_finite());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn record_round_trips_and_detects_damage() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0) - 0.5).collect();
        let peaks = ChunkPeaks::compute(&samples);
        let path = std::env::temp_dir().join(format!("vox-project-peaks-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        peaks.write_record(&file, 3).unwrap();
        assert_eq!(ChunkPeaks::read_record(&file, 3).unwrap(), peaks);
        // Records 0..3 are a hole (zeros): detected as missing.
        assert!(ChunkPeaks::read_record(&file, 0).is_none());
        // One flipped byte in the buckets is detected.
        let offset = 3 * PEAK_RECORD_BYTES as u64 + 100;
        write_all_at(&file, &[0xAB], offset).unwrap();
        assert!(ChunkPeaks::read_record(&file, 3).is_none());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}
