//! Save a snapshot to WAV (SPEC-005 §2.7, §2.8, §2.9; ADR-004 §8: atomic, no rack).

use crate::reader::SnapshotReader;
use crate::snapshot::Marker;
use crate::{CHUNK_SAMPLES, Result};

/// Renders every sample `reader`'s snapshot holds and writes it to `path` as a WAV at `bits`
/// through [`vox_io::write_wav_with_markers`] (TPDF-dithered for 16/24-bit, atomic — temp file,
/// `fdatasync`, rename, ADR-004 §8), plus `cue `/`LIST adtl` chunks for `markers` (SPEC-005 §2.9;
/// `markers` empty writes neither chunk). Save does not render the rack (D-019): this writes
/// exactly the document's audio.
///
/// The multichannel-source/clip-prompt dialogs are deferred (S1-02 ticket scope) — the caller
/// checks the document peak pyramid for overs before calling this, if it wants that prompt.
pub fn save_snapshot_wav(
    reader: &mut SnapshotReader,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
    markers: &[Marker],
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
    // `markers` (`DocSnapshot::markers`) is already in canonical, position-sorted order, which
    // `write_wav_with_markers` relies on to assign cue ids 1..n in position order (SPEC-005 §2.9).
    let wav_markers: Vec<vox_io::WavMarker> = markers
        .iter()
        .map(|m| vox_io::WavMarker {
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name.to_string(),
        })
        .collect();
    Ok(vox_io::write_wav_with_markers(
        path,
        rate,
        bits,
        &samples,
        &wav_markers,
    )?)
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
}
