//! T-602: Bake rack through sandboxed plugins — a plugin that crashes mid-bake aborts it with
//! the document unchanged (ADR-008 §5: offline renders abort), and a healthy sandboxed plugin
//! bakes bit-exactly like the in-process module.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use vox_engine::bake::{DocumentRange, RangeRenderError, plan_bake};
use vox_project::{CancelToken, Session, SessionConfig, SnapshotReader, StoreOptions};
use vox_rack::offline::RenderError;

/// A session directory removed on drop (the store preallocates 64 MB segments).
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "powervoice-app-sandboxbake-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn session_with(dir: &Dir, samples: &[f32]) -> Session {
    let mut session = Session::create(
        &dir.0,
        SessionConfig {
            store: StoreOptions::with_memory_budget(64 * 1024 * 1024),
            ..SessionConfig::new(48_000)
        },
    )
    .unwrap();
    let mut writer = session.chunk_writer();
    writer.append(samples).unwrap();
    let audio = writer.finish().unwrap();
    session.set_floor(&audio, Vec::new()).unwrap();
    session
}

fn read_all(session: &Session) -> Vec<f32> {
    let snapshot = session.current();
    let mut reader = SnapshotReader::new(Arc::clone(session.store()), Arc::clone(&snapshot));
    let mut out = vec![0.0f32; snapshot.len_samples as usize];
    let mut pos = 0;
    while pos < out.len() {
        let n = reader.read(pos as u64, &mut out[pos..]).unwrap();
        assert!(n > 0);
        pos += n;
    }
    out
}

fn whole(session: &Session) -> vox_project::Range {
    let len = session.current().len_samples;
    vox_project::validate_range(0, len, len).unwrap()
}

#[test]
fn a_plugin_crash_aborts_the_bake_with_the_document_unchanged() {
    let registry = registry_with(&[factory("crash", "crash?after=2", exact_options())]);
    let dir = Dir::new("crash");
    let x = noise(4096 * 6, 5);
    let session = session_with(&dir, &x);
    let before = session.current();
    let t0 = Instant::now();
    let err = plan_bake(
        &registry,
        &model(vec![gain_slot("test:crash@1.0.0", 0.0)]),
        DocumentRange {
            store: session.store(),
            snapshot: &before,
            sample_rate_hz: 48_000,
            range: whole(&session),
        },
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            RangeRenderError::Render(RenderError::SlotFailed { index: 0, .. })
        ),
        "{err}"
    );
    assert!(t0.elapsed() < Duration::from_secs(4), "aborted promptly");
    // Nothing was committed: same revision, same samples, no undo entry.
    assert!(Arc::ptr_eq(&session.current(), &before));
    assert_eq!(session.history().undo_depth(), 0);
    assert_bits(&read_all(&session), &x, "document after the aborted bake");
}

#[test]
fn a_sandboxed_plugin_bakes_like_the_in_process_module() {
    let registry = registry_with(&[factory("gain", "gain", exact_options())]);
    let x = noise(30_000, 9);
    let mut baked = Vec::new();
    for module in ["test:gain@1.0.0", "org.powervoice.gain@1.0.0"] {
        let dir = Dir::new("gain");
        let mut session = session_with(&dir, &x);
        let snapshot = session.current();
        let edit = plan_bake(
            &registry,
            &model(vec![gain_slot(module, -6.0)]),
            DocumentRange {
                store: session.store(),
                snapshot: &snapshot,
                sample_rate_hz: 48_000,
                range: whole(&session),
            },
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        session.commit_edit(edit).unwrap();
        baked.push(read_all(&session));
    }
    assert_bits(&baked[0], &baked[1], "sandboxed vs in-process bake");
}
