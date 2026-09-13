//! Save a snapshot to WAV (SPEC-005 §2.7, §2.8, §2.9; ADR-004 §8: atomic, no rack).

use crate::reader::SnapshotReader;
use crate::snapshot::Marker;
use crate::{CHUNK_SAMPLES, Result};

/// A source of a document's samples for [`save_snapshot_wav`]: generalized (over
/// [`SnapshotReader`] in production) so the save streams through any backing reader without ever
/// holding the whole document in one buffer, and so tests can substitute a lightweight fake that
/// checks the block sizes the save actually asks for (H-02).
trait SampleSource {
    /// Sample rate to put in the WAV header.
    fn sample_rate_hz(&self) -> u32;
    /// Document length in samples.
    fn len_samples(&self) -> u64;
    /// Copies `[pos, pos + out.len())` into `out`; see [`SnapshotReader::read`].
    fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize>;
}

impl SampleSource for SnapshotReader {
    fn sample_rate_hz(&self) -> u32 {
        self.snapshot().sample_rate_hz
    }

    fn len_samples(&self) -> u64 {
        SnapshotReader::len_samples(self)
    }

    fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize> {
        SnapshotReader::read(self, pos, out)
    }
}

/// Streams every sample `reader`'s snapshot holds to `path` as a WAV at `bits` through
/// [`vox_io::WavStreamWriter`] (TPDF-dithered for 16/24-bit, atomic — temp file, `fdatasync`,
/// rename, ADR-004 §8), plus `cue `/`LIST adtl` chunks for `markers` (SPEC-005 §2.9; `markers`
/// empty writes neither chunk). Save does not render the rack (D-019): this writes exactly the
/// document's audio.
///
/// Reads and writes [`CHUNK_SAMPLES`]-sample blocks at a time (H-02): memory stays bounded by
/// that one block, not by the document's length — a 60-minute mono recording no longer needs
/// ~691 MB of `Vec<f32>` to save.
///
/// The multichannel-source/clip-prompt dialogs are deferred (S1-02 ticket scope) — the caller
/// checks the document peak pyramid for overs before calling this, if it wants that prompt.
pub fn save_snapshot_wav(
    reader: &mut SnapshotReader,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
    markers: &[Marker],
) -> Result<vox_io::WriteReport> {
    save_snapshot_wav_streaming(reader, path, bits, markers)
}

fn save_snapshot_wav_streaming<R: SampleSource>(
    source: &mut R,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
    markers: &[Marker],
) -> Result<vox_io::WriteReport> {
    let len = source.len_samples();
    let rate = source.sample_rate_hz();
    let mut writer = vox_io::WavStreamWriter::create(path, rate, bits)?;

    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    let mut pos = 0u64;
    let stream_result: Result<()> = (|| {
        while pos < len {
            let n = source.read(pos, &mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_block(&buf[..n])?;
            pos += n as u64;
        }
        Ok(())
    })();
    if let Err(e) = stream_result {
        writer.abort();
        return Err(e);
    }

    // `markers` (`DocSnapshot::markers`) is already in canonical, position-sorted order, which
    // `WavStreamWriter::finish` relies on to assign cue ids 1..n in position order (SPEC-005
    // §2.9).
    let wav_markers: Vec<vox_io::WavMarker> = markers
        .iter()
        .map(|m| vox_io::WavMarker {
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name.to_string(),
        })
        .collect();
    Ok(writer.finish(&wav_markers)?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::ChunkStore;
    use crate::{DocSnapshot, StoreOptions};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vox-project-save-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_snapshot_wav_round_trips_float32() {
        let dir = tmp_dir("float32");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::sine(1000.0, -6.0, 0.05, 48_000).unwrap();
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));

        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        let out_path = dir.join("out.wav");
        let report =
            save_snapshot_wav(&mut reader, &out_path, vox_io::BitDepth::Float32, &[]).unwrap();
        assert_eq!(report.clipped_samples, 0);

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(decoded, samples);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_snapshot_wav_writes_markers_as_cue_points() {
        use crate::snapshot::{Marker, MarkerId};

        let dir = tmp_dir("markers");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::silence(0.2, 48_000).unwrap();
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let markers = vec![
            Marker::new(MarkerId(1), 0, 0, "Intro"),
            Marker::new(MarkerId(2), 4_800, 2_400, "Take 2"),
        ];
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, markers));

        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        let out_path = dir.join("out.wav");
        save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Int16,
            &snapshot.markers,
        )
        .unwrap();

        let read_back = vox_io::read_wav_markers(&out_path).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!((read_back[0].pos_samples, read_back[0].len_samples), (0, 0));
        assert_eq!(read_back[0].name, "Intro");
        assert_eq!(
            (read_back[1].pos_samples, read_back[1].len_samples),
            (4_800, 2_400)
        );
        assert_eq!(read_back[1].name, "Take 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-02: streaming ------------------------------------------------------------------

    /// A fake [`SampleSource`] over an in-memory buffer that records every `(pos, requested_len)`
    /// it was asked for, so a test can assert the save path never asks for more than one
    /// `CHUNK_SAMPLES` block at a time — i.e. it never allocates (or requests into) a buffer
    /// proportional to the document length.
    struct CountingSource {
        samples: Vec<f32>,
        rate: u32,
        calls: Vec<(u64, usize)>,
    }

    impl SampleSource for CountingSource {
        fn sample_rate_hz(&self) -> u32 {
            self.rate
        }

        fn len_samples(&self) -> u64 {
            self.samples.len() as u64
        }

        fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize> {
            self.calls.push((pos, out.len()));
            let start = pos as usize;
            let n = out.len().min(self.samples.len() - start);
            out[..n].copy_from_slice(&self.samples[start..start + n]);
            Ok(n)
        }
    }

    fn noise(seed: u64, n: usize) -> Vec<f32> {
        let mut state = seed | 1;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / (1u32 << 24) as f32) * 1.6 - 0.8
            })
            .collect()
    }

    #[test]
    fn save_streams_in_bounded_blocks_never_reading_the_whole_document_at_once() {
        // Several blocks plus a short tail: exercises the "last read may be shorter" path too.
        let n = CHUNK_SAMPLES * 3 + 12_345;
        let samples = noise(1, n);
        let mut source = CountingSource {
            samples,
            rate: 48_000,
            calls: Vec::new(),
        };
        let dir = tmp_dir("streamed-bounded");
        let out_path = dir.join("out.wav");
        save_snapshot_wav_streaming(&mut source, &out_path, vox_io::BitDepth::Int16, &[]).unwrap();

        assert!(
            !source.calls.is_empty(),
            "the save must actually read the source"
        );
        for &(_, requested) in &source.calls {
            assert!(
                requested <= CHUNK_SAMPLES,
                "save_snapshot_wav must never request more than one CHUNK_SAMPLES block at a \
                 time (requested {requested})"
            );
        }
        let expected_calls = n.div_ceil(CHUNK_SAMPLES);
        assert_eq!(
            source.calls.len(),
            expected_calls,
            "expected one read per CHUNK_SAMPLES block (plus the short tail)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_of_a_document_larger_than_several_blocks_round_trips_via_fnv1a() {
        let dir = tmp_dir("streamed-large");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let n = CHUNK_SAMPLES * 4 + 777;
        let samples = noise(2, n);
        let mut writer = store.writer();
        // Feed the store in odd-sized blocks too, so chunk boundaries and CHUNK_SAMPLES read
        // blocks don't just trivially line up.
        for block in samples.chunks(10_007) {
            writer.append(block).unwrap();
        }
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.wav");
        save_snapshot_wav(&mut reader, &out_path, vox_io::BitDepth::Float32, &[]).unwrap();

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(decoded.len(), samples.len());
        assert_eq!(
            vox_testkit::golden::fnv1a_hash(&decoded),
            vox_testkit::golden::fnv1a_hash(&samples),
            "a streamed save of a multi-block document must round-trip exactly"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_of_a_document_larger_than_several_blocks_keeps_its_markers() {
        use crate::snapshot::{Marker, MarkerId};

        let dir = tmp_dir("streamed-large-markers");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let n = CHUNK_SAMPLES * 3 + 4_000;
        let samples = noise(3, n);
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let markers = vec![
            Marker::new(MarkerId(1), 100, 0, "Start"),
            // Sits exactly on a CHUNK_SAMPLES block boundary.
            Marker::new(MarkerId(2), CHUNK_SAMPLES as u64, 0, "Boundary"),
            // Falls in the final, short block.
            Marker::new(MarkerId(3), (CHUNK_SAMPLES * 3 + 500) as u64, 1_000, "Tail"),
        ];
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, markers));
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.wav");
        save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Int16,
            &snapshot.markers,
        )
        .unwrap();

        let read_back = vox_io::read_wav_markers(&out_path).unwrap();
        assert_eq!(read_back.len(), 3);
        assert_eq!(read_back[0].name, "Start");
        assert_eq!(read_back[1].pos_samples, CHUNK_SAMPLES as u64);
        assert_eq!(read_back[1].name, "Boundary");
        assert_eq!(read_back[2].pos_samples, (CHUNK_SAMPLES * 3 + 500) as u64);
        assert_eq!(read_back[2].len_samples, 1_000);
        assert_eq!(read_back[2].name, "Tail");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
