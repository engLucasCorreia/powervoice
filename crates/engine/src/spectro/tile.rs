//! Tile geometry, content keys and tile computation (SPEC-007 §4.3–§4.5).

use vox_dsp::spectro::{FrameAnalyzer, frame_input};
use vox_project::{ChunkId, DocSnapshot, ProjectError, SnapshotReader, Source};

/// Frames per tile (SPEC-007 §3 `tile_frames`, ADR-003 `VXST` `frames ≤ 256`).
pub const TILE_FRAMES: u32 = 256;
/// Version of the tile algorithm, part of every cache key (SPEC-007 §4.5).
pub const ALGO_VERSION: u32 = 1;

/// Frames of the grid anchored at 0 whose centre `i·hop` lies in `[0, len)`: `ceil(len / hop)`.
pub fn total_frames(len_samples: u64, hop: u32) -> u64 {
    len_samples.div_ceil(u64::from(hop.max(1)))
}

/// Tiles covering all frames: `ceil(total_frames / 256)`.
pub fn tile_count(len_samples: u64, hop: u32) -> u64 {
    total_frames(len_samples, hop).div_ceil(u64::from(TILE_FRAMES))
}

/// Frames in tile `tile_index` (0 for a tile past the end).
pub fn tile_frames(len_samples: u64, hop: u32, tile_index: u32) -> u32 {
    let first = u64::from(tile_index) * u64::from(TILE_FRAMES);
    let total = total_frames(len_samples, hop);
    if first >= total {
        0
    } else {
        (total - first).min(u64::from(TILE_FRAMES)) as u32
    }
}

/// One tile of the grid: FFT size, hop, index and its frame count (≥ 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileGeom {
    pub fft_size: u32,
    pub hop: u32,
    pub tile_index: u32,
    pub frames: u32,
}

impl TileGeom {
    /// Tile `tile_index` of a `len_samples` document, or `None` past its end.
    pub fn new(len_samples: u64, fft_size: u32, hop: u32, tile_index: u32) -> Option<Self> {
        let frames = tile_frames(len_samples, hop, tile_index);
        (frames > 0).then_some(TileGeom {
            fft_size,
            hop,
            tile_index,
            frames,
        })
    }

    /// `256·k·hop` (ADR-003 `first_frame_center_sample`).
    pub fn first_frame_center_sample(&self) -> u64 {
        u64::from(self.tile_index) * u64::from(TILE_FRAMES) * u64::from(self.hop)
    }

    /// `fft_size / 2 + 1`.
    pub fn bins(&self) -> u32 {
        self.fft_size / 2 + 1
    }

    /// The document samples `[start, end)` this tile reads (SPEC-007 §4.5 "input span"): from the
    /// first frame's input start to the last frame's input end. May extend outside `[0, len)`;
    /// those samples are zeros.
    pub fn input_span(&self, preview: bool) -> (i64, i64) {
        let fi = frame_input(self.fft_size, self.hop, preview);
        let first = self.first_frame_center_sample() as i64;
        let last = first + i64::from(self.frames.saturating_sub(1)) * i64::from(self.hop);
        (first + fi.offset, last + fi.offset + fi.len as i64)
    }
}

/// One run of a tile's input: part of a chunk, or zeros (silence pieces and the padding outside
/// the document, merged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Run {
    Chunk { id: ChunkId, offset: u32, len: u32 },
    Zero(u64),
}

/// Content key of a tile payload (SPEC-007 §4.5): the runs covering its input span plus every
/// parameter the payload depends on. Position-free: the same input content at another document
/// position (after a cut, say) has the same key. Chunk ids are only meaningful within one
/// [`vox_project::ChunkStore`]; the cache is bound to one store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TileKey {
    algo: u32,
    fft_size: u32,
    hop: u32,
    window: u32,
    preview: bool,
    frames: u32,
    runs: Box<[Run]>,
}

fn push_zero(runs: &mut Vec<Run>, n: u64) {
    if n == 0 {
        return;
    }
    if let Some(Run::Zero(z)) = runs.last_mut() {
        *z += n;
    } else {
        runs.push(Run::Zero(n));
    }
}

/// The content key of `geom` over `snapshot`.
pub(crate) fn tile_key(
    snapshot: &DocSnapshot,
    geom: &TileGeom,
    preview: bool,
    window: u32,
) -> TileKey {
    let (start, end) = geom.input_span(preview);
    let len = snapshot.len_samples as i64;
    let mut runs = Vec::new();
    // Zeros before the document start.
    push_zero(&mut runs, (end.min(0) - start).max(0) as u64);
    let lo = start.clamp(0, len);
    let hi = end.clamp(lo, len);
    if hi > lo
        && let Some((mut i, mut off)) = snapshot.locate(lo as u64)
    {
        let mut remaining = (hi - lo) as u64;
        while remaining > 0 && i < snapshot.pieces.len() {
            let piece = snapshot.pieces[i];
            let take = (u64::from(piece.len) - off).min(remaining);
            match piece.source {
                Source::Silence => push_zero(&mut runs, take),
                Source::Chunk(id) => runs.push(Run::Chunk {
                    id,
                    offset: piece.offset + off as u32,
                    len: take as u32,
                }),
            }
            remaining -= take;
            i += 1;
            off = 0;
        }
    }
    // Zeros past the document end.
    push_zero(&mut runs, (end - start.max(len)).max(0) as u64);
    TileKey {
        algo: ALGO_VERSION,
        fft_size: geom.fft_size,
        hop: geom.hop,
        window,
        preview,
        frames: geom.frames,
        runs: runs.into_boxed_slice(),
    }
}

/// Reads `[start, start + out.len())` of the document, zeros outside `[0, len)`.
fn read_padded(
    reader: &mut SnapshotReader,
    start: i64,
    out: &mut [f32],
) -> Result<(), ProjectError> {
    out.fill(0.0);
    let len = reader.len_samples() as i64;
    let lo = start.max(0);
    let hi = (start + out.len() as i64).min(len);
    if hi > lo {
        let dst = &mut out[(lo - start) as usize..(hi - start) as usize];
        reader.read(lo as u64, dst)?;
    }
    Ok(())
}

/// Computes `geom`'s payload (`frames × bins` codes, frame-major, bin 0 = DC). Returns
/// `Ok(None)` as soon as `cancelled()` is true (checked between frames, SPEC-007 §4.6).
pub(crate) fn compute_tile(
    reader: &mut SnapshotReader,
    geom: &TileGeom,
    preview: bool,
    analyzer: &mut FrameAnalyzer,
    buf: &mut Vec<f32>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Vec<u8>>, ProjectError> {
    debug_assert_eq!(analyzer.fft_size(), geom.fft_size as usize);
    let fi = frame_input(geom.fft_size, geom.hop, preview);
    let bins = geom.bins() as usize;
    buf.resize(fi.len, 0.0);
    let mut payload = vec![0u8; geom.frames as usize * bins];
    let first = geom.first_frame_center_sample() as i64;
    for (f, row) in payload.chunks_exact_mut(bins).enumerate() {
        if cancelled() {
            return Ok(None);
        }
        let center = first + f as i64 * i64::from(geom.hop);
        read_padded(reader, center + fi.offset, buf)?;
        analyzer.frame_codes(buf, fi.sub_frames, row);
    }
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_project::Piece;

    #[test]
    fn grid_geometry() {
        assert_eq!(total_frames(0, 512), 0);
        assert_eq!(total_frames(1, 512), 1);
        assert_eq!(total_frames(512, 512), 1);
        assert_eq!(total_frames(513, 512), 2);
        assert_eq!(tile_count(480_000, 128), 15); // 3750 frames
        assert_eq!(tile_frames(480_000, 128, 14), 3750 - 14 * 256);
        assert_eq!(tile_frames(480_000, 128, 15), 0);
        let g = TileGeom::new(480_000, 2048, 128, 2).unwrap();
        assert_eq!(g.first_frame_center_sample(), 2 * 256 * 128);
        assert_eq!(g.bins(), 1025);
        assert_eq!(
            g.input_span(false),
            (65_536 - 1024, 65_536 + 255 * 128 + 1024)
        );
        assert!(TileGeom::new(480_000, 2048, 128, 15).is_none());
        // Overview: refined frames read [c − hop/2 − N/4, c + hop/2 + N/4).
        let o = TileGeom::new(10_000_000, 2048, 8192, 0).unwrap();
        assert_eq!(o.input_span(false), (-4096 - 512, 255 * 8192 + 4096 + 512));
        assert_eq!(o.input_span(true), (-1024, 255 * 8192 + 1024));
    }

    #[test]
    fn keys_are_position_free_and_pad_with_zeros() {
        // Two different piece tables with the same content around a tile hash alike.
        let a = DocSnapshot::new(
            48_000,
            vec![Piece::chunk(0, 0, 60_000), Piece::chunk(1, 0, 60_000)],
            vec![],
        );
        let geom = TileGeom::new(a.len_samples, 256, 16, 0).unwrap();
        let k = tile_key(&a, &geom, false, 0);
        assert_eq!(k.runs[0], Run::Zero(128), "leading padding");
        assert_eq!(
            k.runs[1],
            Run::Chunk {
                id: 0,
                offset: 0,
                len: 255 * 16 + 128
            }
        );
        // A silence piece at the start merges with the padding.
        let s = DocSnapshot::new(
            48_000,
            vec![Piece::silence(100), Piece::chunk(0, 0, 60_000)],
            vec![],
        );
        let ks = tile_key(&s, &geom, false, 0);
        assert_eq!(ks.runs[0], Run::Zero(228));
        // Same parameters, different content → different key; preview/window are part of it.
        assert_ne!(k, ks);
        assert_ne!(k, tile_key(&a, &geom, true, 0));
        assert_ne!(k, tile_key(&a, &geom, false, 1));
        // Trailing padding at the document end.
        let short = DocSnapshot::new(48_000, vec![Piece::chunk(3, 10, 1000)], vec![]);
        let g = TileGeom::new(short.len_samples, 256, 16, 0).unwrap();
        assert_eq!(g.frames, 63);
        let k = tile_key(&short, &g, false, 0);
        assert_eq!(
            &*k.runs,
            &[
                Run::Zero(128),
                Run::Chunk {
                    id: 3,
                    offset: 10,
                    len: 1000
                },
                Run::Zero(62 * 16 + 128 - 1000),
            ]
        );
    }
}
