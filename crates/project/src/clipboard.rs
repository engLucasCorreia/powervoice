//! Cross-document clipboard (H-56, SPEC-008 §2.6/§4.4): materializing a clipboard's audio to a
//! plain file when its owning session closes, and importing it (with resampling if the target
//! document's rate differs) into whatever session is open when the next Paste needs it.
//!
//! `project` owns the clipboard (ADR-001 §4); `src-tauri` (`document.rs`) only holds the small
//! `Bound`/`Materialized` state machine and calls these functions at the right times (a closing
//! session, a paste) — no business logic there (CLAUDE.md).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::reader::SnapshotReader;
use crate::snapshot::{DocSnapshot, Piece};
use crate::store::{CancelToken, ChunkStore, ChunkWriter, WrittenAudio};
use crate::{ProjectError, Result};

/// Streamed in blocks this many samples at a time — matches [`ChunkWriter`]'s own chunk size, so
/// cancelling an import (SPEC-008 §2.6.1) is checked at least once per chunk either way.
const IO_BLOCK_SAMPLES: usize = 65_536;

const CLIP_FILE: &str = "clip.f32";
const CLIP_META_FILE: &str = "meta.json";

#[derive(Serialize, Deserialize)]
struct ClipMeta {
    sample_rate_hz: u32,
    len_samples: u64,
}

/// A clipboard whose owning session has closed: its audio lives in a plain file (SPEC-008 §2.6
/// "materialize at close"), not in any session's chunk store.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedClip {
    pub path: PathBuf,
    pub sample_rate_hz: u32,
    pub len_samples: u64,
}

/// Streams `pieces` (read through `store`, at `sample_rate_hz`) to `dir/clip.f32` plus a small
/// `meta.json` (SPEC-008 §2.6). `dir` is created if needed. Fixed file names: there is only ever
/// one clipboard (§2.6 "one in-app clipboard").
pub fn materialize(
    store: &Arc<ChunkStore>,
    pieces: &[Piece],
    sample_rate_hz: u32,
    dir: &Path,
) -> Result<MaterializedClip> {
    std::fs::create_dir_all(dir).map_err(ProjectError::io("creating the clipboard directory"))?;
    let synthetic = DocSnapshot::new(sample_rate_hz, pieces.to_vec(), Vec::new());
    let len_samples = synthetic.len_samples;
    let mut reader = SnapshotReader::new(Arc::clone(store), Arc::new(synthetic));
    let path = dir.join(CLIP_FILE);
    let file =
        std::fs::File::create(&path).map_err(ProjectError::io("writing the clipboard file"))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut buf = vec![0.0f32; IO_BLOCK_SAMPLES];
    let mut pos = 0u64;
    while pos < len_samples {
        let want = (len_samples - pos).min(IO_BLOCK_SAMPLES as u64) as usize;
        let got = reader.read(pos, &mut buf[..want])?;
        if got == 0 {
            break;
        }
        let mut bytes = Vec::with_capacity(got * 4);
        for &s in &buf[..got] {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        writer
            .write_all(&bytes)
            .map_err(ProjectError::io("writing the clipboard file"))?;
        pos += got as u64;
    }
    writer
        .flush()
        .map_err(ProjectError::io("writing the clipboard file"))?;
    let meta = ClipMeta {
        sample_rate_hz,
        len_samples,
    };
    let meta_bytes = serde_json::to_vec(&meta)?;
    std::fs::write(dir.join(CLIP_META_FILE), meta_bytes)
        .map_err(ProjectError::io("writing the clipboard metadata"))?;
    Ok(MaterializedClip {
        path,
        sample_rate_hz,
        len_samples,
    })
}

/// Reads back a clipboard directory's `clip.f32`/`meta.json` (e.g. across app runs, though H-56
/// sweeps stale ones at start-up — [`sweep_stale_dir`]).
pub fn read_materialized(dir: &Path) -> Result<Option<MaterializedClip>> {
    let meta_path = dir.join(CLIP_META_FILE);
    let clip_path = dir.join(CLIP_FILE);
    if !meta_path.is_file() || !clip_path.is_file() {
        return Ok(None);
    }
    let bytes =
        std::fs::read(&meta_path).map_err(ProjectError::io("reading the clipboard metadata"))?;
    let meta: ClipMeta = serde_json::from_slice(&bytes)?;
    Ok(Some(MaterializedClip {
        path: clip_path,
        sample_rate_hz: meta.sample_rate_hz,
        len_samples: meta.len_samples,
    }))
}

fn read_clip_samples(clip: &MaterializedClip) -> Result<Vec<f32>> {
    let bytes =
        std::fs::read(&clip.path).map_err(ProjectError::io("reading the clipboard file"))?;
    let expected = clip.len_samples as usize * 4;
    if bytes.len() < expected {
        return Err(ProjectError::InvalidEdit(
            "the clipboard file is shorter than its metadata claims".into(),
        ));
    }
    let (chunks, _) = bytes[..expected].as_chunks::<4>();
    let out: Vec<f32> = chunks.iter().copied().map(f32::from_le_bytes).collect();
    Ok(out)
}

/// Imports a materialized clip into `target_store`, resampling to `target_rate_hz` when it
/// differs from the clip's own rate (SPEC-008 §4.4: the same primed/trimmed fixed-ratio resampler
/// as export's `resample_offline`, output length exactly `round(n * target / clip)`). Cancellable
/// (SPEC-008 §2.6.1): `cancel` is checked at least once per [`IO_BLOCK_SAMPLES`] chunk, both by
/// this loop and by the [`ChunkWriter`] it drives — a cancelled or failed import commits nothing
/// (the writer is simply dropped; its chunks become unreachable store data, reclaimed by
/// compaction). `on_progress` receives `[0, 1]`, monotonically non-decreasing.
pub fn import_clip(
    clip: &MaterializedClip,
    target_store: &Arc<ChunkStore>,
    target_rate_hz: u32,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(f32),
) -> Result<WrittenAudio> {
    let samples = read_clip_samples(clip)?;
    let resampled: Vec<f32> = if target_rate_hz == clip.sample_rate_hz {
        samples
    } else {
        let input: Vec<f64> = samples.iter().map(|&s| f64::from(s)).collect();
        let out = vox_dsp::resample::resample_offline(&input, clip.sample_rate_hz, target_rate_hz)
            .map_err(|e| {
                ProjectError::InvalidEdit(format!("resampling the clipboard failed: {e}"))
            })?;
        out.into_iter().map(|s| s as f32).collect()
    };
    let total = resampled.len().max(1) as f32;
    let mut writer = ChunkWriter::with_cancel(Arc::clone(target_store), cancel.clone());
    let mut pos = 0usize;
    while pos < resampled.len() {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let n = (resampled.len() - pos).min(IO_BLOCK_SAMPLES);
        writer.append(&resampled[pos..pos + n])?;
        pos += n;
        on_progress(pos as f32 / total);
    }
    writer.finish()
}

/// Deletes a materialized clip's file and metadata (SPEC-008 §2.6: rebinding after a same-rate
/// import, since later pastes are then pure splices again). Best-effort.
pub fn delete_materialized(clip: &MaterializedClip) {
    let _ = std::fs::remove_file(&clip.path);
    if let Some(dir) = clip.path.parent() {
        let _ = std::fs::remove_file(dir.join(CLIP_META_FILE));
    }
}

/// Clears the clipboard directory (SPEC-008 §2.6: "deleted at app quit and, if stale, at
/// start-up" — PowerVoice does the latter, since a fresh app run has no use for a previous run's
/// materialized clip either way). Best-effort; a missing directory is not an error.
pub fn sweep_stale_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StoreOptions;

    /// A fresh scratch directory under the OS temp dir, `vox-project-clipboard-<pid>-<tag>`
    /// (matches `import.rs`/`journal.rs`'s convention: no `tempfile` dependency in this crate).
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-clipboard-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_store(dir: &Path) -> Arc<ChunkStore> {
        ChunkStore::create(dir, 1, StoreOptions::default()).unwrap()
    }

    /// Commits `samples` as one or more chunks (a chunk holds at most `CHUNK_SAMPLES`).
    fn commit_samples(store: &Arc<ChunkStore>, samples: &[f32]) -> Vec<Piece> {
        samples
            .chunks(crate::CHUNK_SAMPLES)
            .map(|block| {
                let loc = store.commit_chunk(block).unwrap();
                Piece::chunk(loc.id, 0, loc.len)
            })
            .collect()
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check: progress must reach exactly 1.0
    fn materialize_then_import_same_rate_round_trips() {
        let source_dir = scratch_dir("source");
        let store = make_store(&source_dir);
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32) / 1000.0).collect();
        let pieces = commit_samples(&store, &samples);
        let clip_dir = scratch_dir("clip");
        let clip = materialize(&store, &pieces, 48_000, &clip_dir).unwrap();
        assert_eq!(clip.len_samples, 1000);
        assert_eq!(clip.sample_rate_hz, 48_000);

        let target_dir = scratch_dir("target");
        let target = make_store(&target_dir);
        let cancel = CancelToken::new();
        let mut last = 0.0f32;
        let written = import_clip(&clip, &target, 48_000, &cancel, &mut |f| {
            assert!(f >= last, "progress must not go backwards");
            last = f;
        })
        .unwrap();
        assert_eq!(written.len_samples, 1000);
        assert_eq!(last, 1.0);

        // Read the imported pieces back and confirm they're bit-identical to the original.
        let synthetic = DocSnapshot::new(48_000, written.pieces, Vec::new());
        let mut reader = SnapshotReader::new(Arc::clone(&target), Arc::new(synthetic));
        let mut out = vec![0.0f32; 1000];
        reader.read(0, &mut out).unwrap();
        assert_eq!(out, samples);

        let _ = std::fs::remove_dir_all(&source_dir);
        let _ = std::fs::remove_dir_all(&clip_dir);
        let _ = std::fs::remove_dir_all(&target_dir);
    }

    #[test]
    fn import_with_a_rate_mismatch_resamples_to_the_exact_expected_length() {
        let source_dir = scratch_dir("mismatch-source");
        let store = make_store(&source_dir);
        let samples = vec![0.0f32; 44_100]; // 1.000 s at 44.1 kHz
        let pieces = commit_samples(&store, &samples);
        let clip_dir = scratch_dir("mismatch-clip");
        let clip = materialize(&store, &pieces, 44_100, &clip_dir).unwrap();

        let target_dir = scratch_dir("mismatch-target");
        let target = make_store(&target_dir);
        let cancel = CancelToken::new();
        let written = import_clip(&clip, &target, 48_000, &cancel, &mut |_| {}).unwrap();
        assert_eq!(written.len_samples, 48_000, "round(44100 * 48000/44100)");

        let _ = std::fs::remove_dir_all(&source_dir);
        let _ = std::fs::remove_dir_all(&clip_dir);
        let _ = std::fs::remove_dir_all(&target_dir);
    }

    #[test]
    fn cancelling_an_import_commits_nothing() {
        let source_dir = scratch_dir("cancel-source");
        let store = make_store(&source_dir);
        let samples = vec![0.0f32; 200_000];
        let pieces = commit_samples(&store, &samples);
        let clip_dir = scratch_dir("cancel-clip");
        let clip = materialize(&store, &pieces, 48_000, &clip_dir).unwrap();

        let target_dir = scratch_dir("cancel-target");
        let target = make_store(&target_dir);
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = import_clip(&clip, &target, 48_000, &cancel, &mut |_| {}).unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));

        let _ = std::fs::remove_dir_all(&source_dir);
        let _ = std::fs::remove_dir_all(&clip_dir);
        let _ = std::fs::remove_dir_all(&target_dir);
    }

    #[test]
    fn delete_materialized_removes_both_files() {
        let source_dir = scratch_dir("delete-source");
        let store = make_store(&source_dir);
        let pieces = commit_samples(&store, &[0.0f32; 10]);
        let clip_dir = scratch_dir("delete-clip");
        let clip = materialize(&store, &pieces, 48_000, &clip_dir).unwrap();
        assert!(clip.path.exists());
        delete_materialized(&clip);
        assert!(!clip.path.exists());
        assert!(!clip_dir.join(CLIP_META_FILE).exists());

        let _ = std::fs::remove_dir_all(&source_dir);
        let _ = std::fs::remove_dir_all(&clip_dir);
    }

    #[test]
    fn sweep_stale_dir_removes_a_leftover_clipboard() {
        let source_dir = scratch_dir("sweep-source");
        let store = make_store(&source_dir);
        let pieces = commit_samples(&store, &[0.0f32; 10]);
        let clip_dir = scratch_dir("sweep-clip");
        materialize(&store, &pieces, 48_000, &clip_dir).unwrap();
        assert!(clip_dir.join(CLIP_FILE).exists());
        sweep_stale_dir(&clip_dir);
        assert!(!clip_dir.exists());

        let _ = std::fs::remove_dir_all(&source_dir);
    }

    #[test]
    fn read_materialized_round_trips_through_a_fresh_handle() {
        let source_dir = scratch_dir("reread-source");
        let store = make_store(&source_dir);
        let pieces = commit_samples(&store, &[1.0f32; 25]);
        let clip_dir = scratch_dir("reread-clip");
        let clip = materialize(&store, &pieces, 44_100, &clip_dir).unwrap();
        let reread = read_materialized(&clip_dir).unwrap().unwrap();
        assert_eq!(reread, clip);

        let _ = std::fs::remove_dir_all(&source_dir);
        let _ = std::fs::remove_dir_all(&clip_dir);
    }
}
