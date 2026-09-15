//! Offline renders of a document range through a rack model, and the **bake** plan (T-602;
//! SPEC-004 §2.2 "Bake rack", ADR-001 §4: jobs that combine a snapshot, the rack and the store
//! are orchestrated in `engine`, never in `project`).
//!
//! - [`render_document_range`] is the one render path of export and bake (so a bake is
//!   bit-identical to an export of the same range and rack): it reads the snapshot through a
//!   [`SnapshotReader`], renders with `vox_rack::offline::render_range` (SPEC-012 §2.8 plus the
//!   T-602 amendment: pre-roll from the audio before the range, post-roll from the audio after
//!   it, no tail appended — the output has the range's length), and hands the output to a sink.
//! - [`plan_bake`] streams that output into new store chunks and returns a ready-to-commit
//!   [`Edit`]: "replace the range with the rendered stream" (length-preserving, markers kept in
//!   place), labelled `history.bake`, carrying a [`BakeAttachment`] (ADR-004 Amendment 1: the
//!   pre-bake rack for Undo, the reset rack for Redo). Nothing is committed here; a cancelled or
//!   failed plan leaves only unreachable chunks (reclaimed by compaction), never a changed
//!   document.
//!
//! Runs on a job thread (never the audio thread): reads, allocations and store I/O are fine.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vox_project::{
    CancelToken, ChunkStore, ChunkWriter, DocSnapshot, Edit, MarkerMapping, ProjectError, Range,
    SnapshotReader,
};
use vox_rack::offline::{RenderError, RenderWindow, WindowError, render_range};
use vox_rack::{RackModel, Registry};

/// The undo label of a bake (SPEC-004 §2.2).
pub const LABEL_BAKE: &str = "history.bake";

/// Why a range render or a bake plan stopped.
#[derive(Debug, thiserror::Error)]
pub enum RangeRenderError {
    /// The caller's [`CancelToken`] was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The rack could not be built (missing modules), or a slot failed or crashed (SPEC-012
    /// §2.9, ADR-008 §5: an offline render aborts instead of bypassing).
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Reading the document or writing the rendered chunks failed.
    #[error(transparent)]
    Project(ProjectError),
}

impl From<ProjectError> for RangeRenderError {
    fn from(e: ProjectError) -> Self {
        match e {
            ProjectError::Cancelled => RangeRenderError::Cancelled,
            other => RangeRenderError::Project(other),
        }
    }
}

/// The document range a render reads: one snapshot of one store (SPEC-004 §2.1: a job works on
/// one revision from start to finish).
#[derive(Clone, Copy)]
pub struct DocumentRange<'a> {
    /// The session's store.
    pub store: &'a Arc<ChunkStore>,
    /// The revision to read.
    pub snapshot: &'a Arc<DocSnapshot>,
    /// The document's rate.
    pub sample_rate_hz: u32,
    /// `[start, end)` samples, non-empty and inside `snapshot`.
    pub range: Range,
}

fn read_exact(reader: &mut SnapshotReader, pos: u64, buf: &mut [f32]) -> Result<(), ProjectError> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = reader.read(pos + filled as u64, &mut buf[filled..])?;
        if n == 0 {
            return Err(ProjectError::InvalidEdit(
                "render: the document ended before the range did".into(),
            ));
        }
        filled += n;
    }
    Ok(())
}

/// Renders `source.range` through `model` (see the module docs) and passes the output — exactly
/// `range.len_samples()` samples, in order — to `sink`. `progress` gets the processed fraction
/// after every 4096-frame block; `cancel` is checked there too (and by a cancel-aware sink), so
/// cancellation takes effect within one block. Returns the render window used.
pub fn render_document_range(
    registry: &Registry,
    model: &RackModel,
    source: DocumentRange<'_>,
    cancel: &CancelToken,
    mut progress: impl FnMut(f32),
    mut sink: impl FnMut(&[f32]) -> Result<(), ProjectError>,
) -> Result<RenderWindow, RangeRenderError> {
    if cancel.is_cancelled() {
        return Err(RangeRenderError::Cancelled);
    }
    let mut reader = SnapshotReader::new(Arc::clone(source.store), Arc::clone(source.snapshot));
    let result = render_range(
        registry,
        model,
        f64::from(source.sample_rate_hz),
        source.range.start,
        source.range.len_samples(),
        source.snapshot.len_samples,
        |pos, buf| read_exact(&mut reader, pos, buf).map_err(RangeRenderError::from),
        |out| sink(out).map_err(RangeRenderError::from),
        |done, total| {
            if cancel.is_cancelled() {
                return Err(RangeRenderError::Cancelled);
            }
            progress(done as f32 / total.max(1) as f32);
            Ok(())
        },
    );
    match result {
        Ok(window) => Ok(window),
        Err(WindowError::Render(e)) => Err(RangeRenderError::Render(e)),
        Err(WindowError::Caller(e)) => Err(e),
    }
}

/// [`render_document_range`] into memory (export's path: the resampler and the encoders take
/// whole buffers).
pub fn render_document_range_to_vec(
    registry: &Registry,
    model: &RackModel,
    source: DocumentRange<'_>,
    cancel: &CancelToken,
    progress: impl FnMut(f32),
) -> Result<Vec<f32>, RangeRenderError> {
    let mut out = Vec::with_capacity(source.range.len_samples() as usize);
    render_document_range(registry, model, source, cancel, progress, |s| {
        out.extend_from_slice(s);
        Ok(())
    })?;
    Ok(out)
}

/// The opaque undo attachment of a bake entry (ADR-004 Amendment 1, SPEC-004 AC-16): the rack
/// to restore on Undo (`before`: slots, parameter values, bypass flags and state blobs, exactly
/// as baked) and on Redo (`after`: the reset rack). `project` stores the bytes verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakeAttachment {
    /// Always [`BakeAttachment::KIND`], so another attachment kind is never mistaken for it.
    pub kind: String,
    /// Format version (1).
    pub version: u32,
    /// The rack before the bake.
    pub before: RackModel,
    /// The rack after the bake (SPEC-004 OD-4 default: reset, i.e. empty).
    pub after: RackModel,
}

impl BakeAttachment {
    /// The `kind` tag.
    pub const KIND: &'static str = "bake";
    /// The current `version`.
    pub const VERSION: u32 = 1;

    /// The attachment of a bake of `before` that then resets the rack.
    pub fn reset_after(before: RackModel) -> Self {
        BakeAttachment {
            kind: Self::KIND.into(),
            version: Self::VERSION,
            before,
            after: RackModel::default(),
        }
    }

    /// Serialized (JSON bytes; never fails for a `RackModel`, whose values are all JSON).
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Parses an entry's attachment; `None` for anything that is not a bake attachment of a
    /// known version (Undo then leaves the rack alone rather than guessing).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let a: BakeAttachment = serde_json::from_slice(bytes).ok()?;
        (a.kind == Self::KIND && a.version == Self::VERSION).then_some(a)
    }
}

/// Renders `source.range` through `model` into new chunks of `source.store` and returns the
/// bake's [`Edit`] (not committed): the range replaced by the rendered samples, markers kept in
/// place, the [`BakeAttachment`] of a bake that resets the rack. `cancel` stops it within one
/// block; any error leaves the document untouched (only unreachable chunks were written).
pub fn plan_bake(
    registry: &Registry,
    model: &RackModel,
    source: DocumentRange<'_>,
    cancel: &CancelToken,
    progress: impl FnMut(f32),
) -> Result<Edit, RangeRenderError> {
    let mut writer = ChunkWriter::with_cancel(Arc::clone(source.store), cancel.clone());
    render_document_range(registry, model, source, cancel, progress, |s| {
        writer.append(s)
    })?;
    let written = writer.finish()?;
    let range = source.range;
    Ok(Edit::new(LABEL_BAKE)
        .replace_with(
            range.start,
            range.len_samples(),
            written.pieces,
            MarkerMapping::Identity,
        )
        .with_attachment(BakeAttachment::reset_after(model.clone()).encode()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vox_module_api::{ModuleRef, ModuleState, Version};
    use vox_project::{Session, SessionConfig, StoreOptions, WrittenAudio};
    use vox_rack::SlotModel;

    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "powervoice-app-enginebake-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Removes the temp dir (the store preallocates 64 MB segments on a tmpfs `/tmp`).
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn registry() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
    }

    fn slot(id: &str, bypass: bool, params: &[(&str, f64)]) -> SlotModel {
        SlotModel::new(
            &ModuleRef {
                id: id.into(),
                version: Version::new(1, 0, 0),
            },
            bypass,
            &ModuleState {
                format_version: 1,
                params: params
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), *v))
                    .collect::<BTreeMap<_, _>>(),
                blob: None,
            },
        )
    }

    /// A rack with latency (the limiter's look-ahead) and a long tail (the EQ).
    fn latency_and_tail_model() -> RackModel {
        RackModel {
            slots: vec![
                slot(
                    vox_modules::ParametricEq::ID,
                    false,
                    &[("b1_on", 1.0), ("b1_gain_db", 9.0)],
                ),
                slot(vox_modules::Gain::ID, true, &[("gain_db", 12.0)]),
                slot(
                    vox_modules::TruePeakLimiter::ID,
                    false,
                    &[("input_gain_db", 6.0)],
                ),
            ],
        }
    }

    fn session_with(dir: &std::path::Path, samples: &[f32]) -> Session {
        let mut session = Session::create(
            dir,
            SessionConfig {
                store: StoreOptions::with_memory_budget(64 * 1024 * 1024),
                ..SessionConfig::new(48_000)
            },
        )
        .unwrap();
        let mut writer = session.chunk_writer();
        writer.append(samples).unwrap();
        let audio: WrittenAudio = writer.finish().unwrap();
        session.set_floor(&audio, Vec::new()).unwrap();
        session
    }

    fn read_all(session: &Session) -> Vec<f32> {
        let snapshot = session.current();
        let mut reader = SnapshotReader::new(Arc::clone(session.store()), snapshot.clone());
        let mut out = vec![0.0f32; snapshot.len_samples as usize];
        read_exact(&mut reader, 0, &mut out).unwrap();
        out
    }

    fn signal() -> Vec<f32> {
        let mut x = vox_testkit::signal::pink_noise(7, -18.0, 3.0, 48_000).unwrap();
        x.extend(vox_testkit::signal::sine(1_000.0, -3.0, 0.5, 48_000).unwrap());
        x
    }

    #[test]
    fn a_whole_file_bake_equals_the_export_render_bit_for_bit() {
        let dir = Dir(tmp_dir("whole"));
        let x = signal();
        let mut session = session_with(&dir.0, &x);
        let reg = registry();
        let model = latency_and_tail_model();
        let before = session.current();
        let range = vox_project::validate_range(0, x.len() as u64, x.len() as u64).unwrap();
        let source = DocumentRange {
            store: session.store(),
            snapshot: &before,
            sample_rate_hz: 48_000,
            range,
        };
        let exported =
            render_document_range_to_vec(&reg, &model, source, &CancelToken::new(), |_| {})
                .unwrap();
        // The whole-file render is SPEC-012's: same as `vox_rack::offline::render`.
        let spec = vox_rack::offline::render(&reg, &model, 48_000.0, &x).unwrap();
        assert_eq!(exported, spec);

        let edit = plan_bake(&reg, &model, source, &CancelToken::new(), |_| {}).unwrap();
        assert_eq!(edit.label_key, LABEL_BAKE);
        session.commit_edit(edit).unwrap();
        let baked = read_all(&session);
        assert_eq!(baked.len(), x.len(), "a bake never changes the length");
        assert!(
            baked
                .iter()
                .zip(&exported)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "bake == export render, bit for bit"
        );
    }

    #[test]
    fn a_selection_bake_equals_the_export_render_and_leaves_the_outside_bit_exact() {
        let dir = Dir(tmp_dir("selection"));
        let x = signal();
        let mut session = session_with(&dir.0, &x);
        let reg = registry();
        let model = latency_and_tail_model();
        let before = session.current();
        let (s, e) = (50_000u64, 120_000u64);
        let range = vox_project::validate_range(s, e, x.len() as u64).unwrap();
        let source = DocumentRange {
            store: session.store(),
            snapshot: &before,
            sample_rate_hz: 48_000,
            range,
        };
        let exported =
            render_document_range_to_vec(&reg, &model, source, &CancelToken::new(), |_| {})
                .unwrap();
        let edit = plan_bake(&reg, &model, source, &CancelToken::new(), |_| {}).unwrap();
        session.commit_edit(edit).unwrap();
        let baked = read_all(&session);
        assert_eq!(baked.len(), x.len());
        let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&baked[..s as usize]), bits(&x[..s as usize]), "before");
        assert_eq!(bits(&baked[e as usize..]), bits(&x[e as usize..]), "after");
        assert_eq!(
            bits(&baked[s as usize..e as usize]),
            bits(&exported),
            "inside"
        );
        // The range was rendered in context: it equals the whole-file render's slice.
        let whole = vox_rack::offline::render(&reg, &model, 48_000.0, &x).unwrap();
        assert_eq!(bits(&exported), bits(&whole[s as usize..e as usize]));
    }

    #[test]
    fn a_cancelled_bake_returns_cancelled_and_commits_nothing() {
        let dir = Dir(tmp_dir("cancel"));
        let x = signal();
        let session = session_with(&dir.0, &x);
        let reg = registry();
        let before = session.current();
        let source = DocumentRange {
            store: session.store(),
            snapshot: &before,
            sample_rate_hz: 48_000,
            range: vox_project::validate_range(0, x.len() as u64, x.len() as u64).unwrap(),
        };
        let cancel = CancelToken::new();
        let mut calls = 0;
        let err = plan_bake(&reg, &latency_and_tail_model(), source, &cancel, |_| {
            calls += 1;
            if calls == 3 {
                cancel.cancel();
            }
        })
        .unwrap_err();
        assert!(matches!(err, RangeRenderError::Cancelled), "{err}");
        assert!(Arc::ptr_eq(&session.current(), &before));
        assert_eq!(session.history().undo_depth(), 0, "no undo entry");
    }

    #[test]
    fn a_missing_module_fails_the_bake() {
        let dir = Dir(tmp_dir("missing"));
        let x = signal();
        let session = session_with(&dir.0, &x);
        let before = session.current();
        let source = DocumentRange {
            store: session.store(),
            snapshot: &before,
            sample_rate_hz: 48_000,
            range: vox_project::validate_range(0, 1_000, x.len() as u64).unwrap(),
        };
        let model = RackModel {
            slots: vec![slot("org.example.not-installed", false, &[])],
        };
        let err = plan_bake(&registry(), &model, source, &CancelToken::new(), |_| {}).unwrap_err();
        assert!(
            matches!(
                err,
                RangeRenderError::Render(RenderError::MissingModules(_))
            ),
            "{err}"
        );
    }

    #[test]
    fn the_attachment_round_trips_and_rejects_foreign_bytes() {
        let a = BakeAttachment::reset_after(latency_and_tail_model());
        assert_eq!(BakeAttachment::decode(&a.encode()), Some(a.clone()));
        assert!(a.after.slots.is_empty(), "OD-4 default: the rack is reset");
        assert_eq!(BakeAttachment::decode(b"\x01\x02"), None);
        assert_eq!(
            BakeAttachment::decode(br#"{"kind":"other","version":1}"#),
            None
        );
    }
}
