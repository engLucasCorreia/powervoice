//! Per-chunk min/max peak pyramids (ADR-004 §5).
//!
//! Levels are 64, 256, 1024, 4096, 16 384 and 65 536 samples per bucket: 1 024 + 256 + 64 + 16 +
//! 4 + 1 = 1 365 `[min, max]` buckets per chunk. Each pyramid is stored as a fixed-size record at
//! `id × PEAK_RECORD_BYTES` in `peaks.<gen>.bin`: `[id u32][len u32][crc32 u32][0 u32]` followed
//! by the buckets as f32 LE pairs. The CRC covers `len` and the buckets, so a torn or missing
//! record (a hole reads as zeros, i.e. `len = 0`) is detected and recomputed.

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
    buckets: Box<[[f32; 2]]>,
}

impl ChunkPeaks {
    /// Computes the pyramid of up to 65 536 samples. Buckets past the end of a partial chunk are
    /// `[0, 0]`; a bucket whose samples are all NaN is `[0, 0]` too.
    pub fn compute(samples: &[f32]) -> ChunkPeaks {
        let n = samples.len().min(CHUNK_SAMPLES);
        let samples = &samples[..n];
        let mut buckets = vec![[0.0f32; 2]; PEAK_BUCKETS_PER_CHUNK].into_boxed_slice();
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
        ChunkPeaks {
            len_samples: n as u32,
            buckets,
        }
    }

    /// Number of samples the pyramid describes.
    pub fn len_samples(&self) -> u32 {
        self.len_samples
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
            buckets,
        })
    }
}

const EMPTY: [f32; 2] = [f32::INFINITY, f32::NEG_INFINITY];

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
