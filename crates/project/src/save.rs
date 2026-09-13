//! Save a snapshot to WAV (SPEC-005 §2.7, §2.8; ADR-004 §8: atomic, no rack).

use crate::reader::SnapshotReader;
use crate::{CHUNK_SAMPLES, Result};

/// Renders every sample `reader`'s snapshot holds and writes it to `path` as a WAV at `bits`
/// through [`vox_io::write_wav`] (TPDF-dithered for 16/24-bit, atomic — temp file, `fdatasync`,
/// rename, ADR-004 §8). Save does not render the rack (D-019): this writes exactly the document's
/// audio.
///
/// Markers (`cue `/`LIST adtl`) and the multichannel-source/clip-prompt dialogs are deferred
/// (S1-02 ticket scope) — the caller checks the document peak pyramid for overs before calling
/// this, if it wants that prompt.
pub fn save_snapshot_wav(
    reader: &mut SnapshotReader,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
) -> Result<vox_io::WriteReport> {
    let len = reader.len_samples();
    let mut samples = Vec::with_capacity(len as usize);
    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    let mut pos = 0u64;
    while pos < len {
        let n = reader.read(pos, &mut buf)?;
        if n == 0 {
            break;
        }
        samples.extend_from_slice(&buf[..n]);
        pos += n as u64;
    }
    let rate = reader.snapshot().sample_rate_hz;
    Ok(vox_io::write_wav(path, rate, bits, &samples)?)
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
        let report = save_snapshot_wav(&mut reader, &out_path, vox_io::BitDepth::Float32).unwrap();
        assert_eq!(report.clipped_samples, 0);

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(decoded, samples);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
