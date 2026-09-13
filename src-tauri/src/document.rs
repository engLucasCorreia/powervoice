//! Document service (S1-03): the app-level owner of the one open session (ADR-004 §8, SPEC-004
//! §2.1). Wraps S1-02's `import_wav`/`save_snapshot_wav`/`peaks` over a [`vox_project::Session`]
//! and keeps the engine's playback document in sync (`EngineHandle::set_document`, MEMORY.md
//! S1-01 Engine API). `ipc::document_commands` only forwards to this — no business logic lives
//! in the command handlers themselves (CLAUDE.md: "`src-tauri` is a thin command/event layer").

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vox_engine::{EngineHandle, PlaybackDoc};
use vox_project::{ProjectError, Session, SessionConfig, SnapshotReader};

use crate::ipc::{IpcError, IpcErrorCode};
use crate::settings::BitDepth;

/// Every completed retry-with-backoff attempt at closing a superseded session waits this long
/// (ADR-002: the reader thread drops its `Arc<ChunkStore>` asynchronously after
/// `set_document`/`set_document(None)` returns, so a `StoreInUse` refusal right after is
/// expected, not a bug — see [`close_with_retry`]).
const CLOSE_RETRY_DELAY: Duration = Duration::from_millis(5);
/// Give up after this many retries (200 ms total) and leave the directory for a future GC pass
/// (session recovery/GC at start-up is out of this ticket's scope, MEMORY.md follow-up).
const CLOSE_RETRY_ATTEMPTS: u32 = 40;

/// The OS data dir's `sessions` subfolder (mirrors `settings::project_dirs`'s convention; no
/// state dir on macOS/Windows, so those fall back to the data dir like `logging.rs` does).
pub fn default_sessions_dir() -> PathBuf {
    match directories::ProjectDirs::from("app", "powervoice", "powervoice") {
        Some(dirs) => dirs.data_dir().join("sessions"),
        None => std::env::temp_dir().join("powervoice").join("sessions"),
    }
}

/// One open document: its session plus the path/format it's bound to (`None` path = never saved
/// — doesn't arise in this ticket, every document is opened from an existing WAV, SPEC-005 §2.6
/// "new recording" row is S1-04/M3 scope).
struct OpenDocument {
    session: Session,
    path: PathBuf,
    save_bits: BitDepth,
}

/// [`DocumentService::peaks`]'s result: the buckets (or raw samples, `spp == PEAKS_RAW_SPP`) plus
/// the document facts the caller needs to build a `VXPK` header (ADR-003 §2).
#[derive(Debug)]
pub struct PeaksResult {
    pub audio_rev: u64,
    pub sample_rate_hz: u32,
    pub buckets: Vec<(f32, f32)>,
}

/// Everything a `document_changed` event / `document_open`/`document_save`/`document_save_as`
/// result needs. `name: None` means no document is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentInfo {
    pub name: Option<String>,
    pub path: Option<String>,
    pub sample_rate_hz: u32,
    pub len_samples: u64,
    pub dirty: bool,
    pub audio_rev: u64,
}

struct Inner {
    sessions_dir: PathBuf,
    engine: EngineHandle,
    open: Mutex<Option<OpenDocument>>,
}

/// Cheaply cloneable (an `Arc` inside), like [`EngineHandle`] — so command handlers can clone it
/// out of Tauri's `State` and run the blocking session/file work with `spawn_blocking` (the same
/// pattern `ipc::audio_commands` uses for `EngineHandle`).
#[derive(Clone)]
pub struct DocumentService(Arc<Inner>);

fn no_document() -> IpcError {
    IpcError::new(IpcErrorCode::NotFound, "error.document.none")
}

/// Maps a [`ProjectError`] to an [`IpcError`], reusing its i18n key (`ProjectError::i18n_key`).
fn document_error(err: ProjectError) -> IpcError {
    let code = match &err {
        ProjectError::NotWhileRecording => IpcErrorCode::NotWhileRecording,
        ProjectError::InvalidArgument(_) | ProjectError::InvalidEdit(_) => {
            IpcErrorCode::InvalidArgument
        }
        ProjectError::Cancelled => IpcErrorCode::Cancelled,
        _ => IpcErrorCode::Internal,
    };
    IpcError::new(code, err.i18n_key()).with_param("message", err.to_string())
}

/// Maps a [`vox_io::IoError`] (from `read_wav`) to an [`IpcError`]. Only 16/24-bit integer and
/// 32-bit float WAV are accepted (S1-02 scope); every other variant is `Unsupported` (ticket:
/// "show it as a notice").
fn io_open_error(err: vox_io::IoError) -> IpcError {
    match err {
        vox_io::IoError::Unsupported(message) => IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.open.unsupported_format",
        )
        .with_param("message", message),
        vox_io::IoError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            IpcError::new(IpcErrorCode::NotFound, "error.open.not_found")
        }
        other => IpcError::new(IpcErrorCode::Io, "error.open.io")
            .with_param("message", other.to_string()),
    }
}

fn io_save_error(err: vox_io::IoError) -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.save.io").with_param("message", err.to_string())
}

/// `WavSource`'s container format → the save format a document opened from it keeps (SPEC-005
/// §2.6, restricted to this ticket's three supported variants — 8-bit/A-law/µ-law/32-int/64-float
/// mapping is deferred, S1-02 ticket scope already documents that read of those variants is
/// rejected, so no mapping table is needed here).
fn save_bits_for(format: vox_io::WavFormat) -> BitDepth {
    match (format.sample_format, format.bits_per_sample) {
        (vox_io::SampleFormat::Float, 32) => BitDepth::Bit32Float,
        (vox_io::SampleFormat::Int, 16) => BitDepth::Bit16,
        _ => BitDepth::Bit24,
    }
}

impl From<BitDepth> for vox_io::BitDepth {
    fn from(bits: BitDepth) -> Self {
        match bits {
            BitDepth::Bit16 => vox_io::BitDepth::Int16,
            BitDepth::Bit24 => vox_io::BitDepth::Int24,
            BitDepth::Bit32Float => vox_io::BitDepth::Float32,
        }
    }
}

fn format_tag(bits: BitDepth) -> &'static str {
    match bits {
        BitDepth::Bit16 => "wav16",
        BitDepth::Bit24 => "wav24",
        BitDepth::Bit32Float => "wav32f",
    }
}

fn info_of(doc: Option<&OpenDocument>) -> DocumentInfo {
    let Some(doc) = doc else {
        return DocumentInfo::default();
    };
    let snapshot = doc.session.current();
    DocumentInfo {
        name: doc
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
        path: Some(doc.path.to_string_lossy().into_owned()),
        sample_rate_hz: doc.session.sample_rate_hz(),
        len_samples: snapshot.len_samples,
        dirty: doc.session.is_dirty(),
        audio_rev: snapshot.audio_rev,
    }
}

/// Closes a superseded session, retrying briefly on [`ProjectError::StoreInUse`]: the reader
/// thread drops its `Arc<ChunkStore>` a little after `EngineHandle::set_document` returns (it
/// only *sends* the new document to the reader thread, ADR-002), so an immediate `close()` right
/// after swapping the engine's document can race it harmlessly. Never blocks the caller for more
/// than `CLOSE_RETRY_ATTEMPTS * CLOSE_RETRY_DELAY` (200 ms).
fn close_with_retry(mut session: Session) {
    for _ in 0..CLOSE_RETRY_ATTEMPTS {
        match session.close() {
            Ok(()) => return,
            Err(vox_project::CloseError {
                error: ProjectError::StoreInUse,
                session: Some(s),
            }) => {
                session = *s;
                std::thread::sleep(CLOSE_RETRY_DELAY);
            }
            Err(vox_project::CloseError { error, session: _ }) => {
                // Journaled `close` failed, or the directory delete failed after it (`session`
                // is `None` then) — a future GC pass (not wired in this ticket) will finish it.
                tracing::warn!(error = %error, "closing the previous session left it for GC");
                return;
            }
        }
    }
    tracing::warn!(
        "previous session stayed in use after retrying; leaving it for a future GC pass"
    );
}

impl DocumentService {
    pub fn new(sessions_dir: PathBuf, engine: EngineHandle) -> Self {
        Self(Arc::new(Inner {
            sessions_dir,
            engine,
            open: Mutex::new(None),
        }))
    }

    /// The currently open document (or [`DocumentInfo::default`] when none is open).
    pub fn info(&self) -> DocumentInfo {
        info_of(self.0.open.lock().unwrap().as_ref())
    }

    /// Imports `path` as a new session (SPEC-005 §2.3, S1-02 `import_wav`) and makes it the
    /// engine's playback document. Replaces whatever was open before (the frontend is
    /// responsible for the unsaved-changes prompt — SPEC-004 §2.8 "simple version", ticket scope
    /// — before calling this).
    pub fn open(&self, path: &Path) -> Result<DocumentInfo, IpcError> {
        let (rate, _channels, mut source) = vox_io::read_wav(path).map_err(io_open_error)?;
        let save_bits = save_bits_for(source.format());
        let mut session = Session::create(&self.0.sessions_dir, SessionConfig::new(rate))
            .map_err(document_error)?;
        if let Err(err) = vox_project::import_wav(&mut session, &mut source) {
            let _ = std::fs::remove_dir_all(session.dir());
            return Err(document_error(err));
        }
        let snapshot = session.current();
        let store = Arc::clone(session.store());
        self.0
            .engine
            .set_document(Some(PlaybackDoc { store, snapshot }));

        let mut guard = self.0.open.lock().unwrap();
        let previous = guard.replace(OpenDocument {
            session,
            path: path.to_path_buf(),
            save_bits,
        });
        let info = info_of(guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok(info)
    }

    /// Writes the current revision back to its bound path at its bound format (SPEC-005 §2.7).
    /// The frontend routes "Save" with no bound path (never arises in this ticket: every
    /// document is opened from a file) through `document_save_as` instead.
    pub fn save(&self) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let path = doc.path.clone();
        let bits = doc.save_bits;
        save_to(doc, &path, bits)?;
        Ok(info_of(guard.as_ref()))
    }

    /// Writes the current revision to `path` at `bits`, then binds the document to it.
    pub fn save_as(&self, path: &Path, bits: BitDepth) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        save_to(doc, path, bits)?;
        doc.path = path.to_path_buf();
        doc.save_bits = bits;
        Ok(info_of(guard.as_ref()))
    }

    /// `(audio_rev, sample_rate_hz, buckets)` for `[start, start + count)` at `spp` (S1-02
    /// `peaks_query::peaks`; `spp == PEAKS_RAW_SPP` reads raw samples). The caller (`peaks_get`)
    /// encodes the result as `VXPK` (ADR-003 §2).
    pub fn peaks(&self, spp: u32, start: u64, count: u32) -> Result<PeaksResult, IpcError> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        let snapshot = doc.session.current();
        let buckets = vox_project::peaks(doc.session.store(), &snapshot, spp, start, count)
            .map_err(document_error)?;
        Ok(PeaksResult {
            audio_rev: snapshot.audio_rev,
            sample_rate_hz: snapshot.sample_rate_hz,
            buckets,
        })
    }
}

fn save_to(doc: &mut OpenDocument, path: &Path, bits: BitDepth) -> Result<(), IpcError> {
    let snapshot = doc.session.current();
    let mut reader = SnapshotReader::new(Arc::clone(doc.session.store()), snapshot);
    vox_project::save_snapshot_wav(&mut reader, path, bits.into()).map_err(|err| match err {
        ProjectError::Wav(io_err) => io_save_error(io_err),
        other => document_error(other),
    })?;
    doc.session
        .mark_saved(path, format_tag(bits))
        .map_err(document_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vox_engine::backend::fake::FakeBackend;
    use vox_engine::{Engine, EngineConfig};
    use vox_rack::Registry;

    use super::*;
    use crate::ipc::IpcErrorCode;

    /// A running (threaded) engine on the fake backend, no devices plugged in — the tests only
    /// need a real `EngineHandle` to exercise `DocumentService::open`'s
    /// `EngineHandle::set_document` call, not an actual audio callback (MEMORY.md S1-01 Engine
    /// API note: never call `EngineHandle` from an `EventSink` — irrelevant here, no sink set).
    fn test_engine() -> Engine {
        let fake = FakeBackend::new(1);
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "powervoice-app-doc-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture_wav(
        path: &Path,
        samples: &[f32],
        bits: vox_testkit::wav::BitDepth,
        rate: u32,
    ) {
        vox_testkit::wav::write_wav_file(path, samples, 1, rate, bits).unwrap();
    }

    fn service(tag: &str) -> (DocumentService, Engine, PathBuf) {
        let dir = tmp_dir(tag);
        let engine = test_engine();
        let service = DocumentService::new(dir.join("sessions"), engine.handle());
        (service, engine, dir)
    }

    #[test]
    fn open_imports_a_wav_and_reports_it_as_the_current_document() {
        let (service, _engine, dir) = service("open");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );

        let info = service.open(&wav_path).unwrap();
        assert_eq!(info.name.as_deref(), Some("in.wav"));
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.len_samples, samples.len() as u64);
        assert!(
            !info.dirty,
            "import is not an undoable edit (SPEC-005 §2.3)"
        );

        // The engine's transport reflects the newly opened document's length immediately:
        // `set_document` is a synchronous round trip through the control thread.
        let transport = service.0.engine.transport_state();
        assert_eq!(transport.doc_len_samples, samples.len() as u64);
        assert_eq!(transport.doc_rate_hz, 48_000);
    }

    #[test]
    fn open_rejects_an_unsupported_wav_variant() {
        let (service, _engine, dir) = service("unsupported");
        let wav_path = dir.join("in.wav");
        // 8-bit WAVs are outside S1-02's supported set (16/24-bit int, 32-bit float).
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).unwrap();
        w.write_sample(0i32).unwrap();
        w.finalize().unwrap();

        let err = service.open(&wav_path).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert_eq!(err.key, "error.open.unsupported_format");
    }

    #[test]
    fn open_reports_a_missing_file() {
        let (service, _engine, dir) = service("missing");
        let err = service.open(&dir.join("nope.wav")).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NotFound);
        assert_eq!(err.key, "error.open.not_found");
    }

    #[test]
    fn save_and_save_as_round_trip_bit_exactly() {
        let (service, _engine, dir) = service("save");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(1000.0, -3.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );

        let info = service.open(&wav_path).unwrap();
        assert_eq!(info.path.as_deref(), Some(wav_path.to_str().unwrap()));

        // Save (no path change): open -> save with no edits round trips bit-exactly (SPEC-005
        // AC-2), same format (Int24) as the source.
        service.save().unwrap();
        let (decoded, _info) = vox_testkit::wav::read_wav_file(&wav_path).unwrap();
        let (_rate, _channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        let mut original = vec![0.0f32; samples.len()];
        source.read_mono(&mut original).unwrap();
        assert_eq!(decoded, original, "save with no edits is bit-exact");

        // Save As a new path and format (32-bit float): binds the document to the new path/bits.
        let save_as_path = dir.join("out.wav");
        let info = service
            .save_as(&save_as_path, BitDepth::Bit32Float)
            .unwrap();
        assert_eq!(info.path.as_deref(), Some(save_as_path.to_str().unwrap()));
        assert!(!info.dirty);
        let (decoded32, meta) = vox_testkit::wav::read_wav_file(&save_as_path).unwrap();
        assert_eq!(meta.sample_rate, 48_000);
        for (a, b) in decoded32.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn peaks_and_save_before_any_open_report_no_document() {
        let (service, _engine, _dir) = service("nodoc");
        let peaks_err = service.peaks(64, 0, 4).unwrap_err();
        assert_eq!(peaks_err.code, IpcErrorCode::NotFound);
        assert_eq!(peaks_err.key, "error.document.none");
        let save_err = service.save().unwrap_err();
        assert_eq!(save_err.key, "error.document.none");
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check: raw pairs must be (x, x)
    fn peaks_returns_raw_samples_and_pyramid_buckets() {
        let (service, _engine, dir) = service("peaks");
        let wav_path = dir.join("in.wav");
        let samples: Vec<f32> = (0..10_000)
            .map(|i| ((i as f32) * 0.001).sin() * 0.8)
            .collect();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path).unwrap();

        let raw = service.peaks(vox_project::PEAKS_RAW_SPP, 10, 5).unwrap();
        for (i, &(mn, mx)) in raw.buckets.iter().enumerate() {
            assert_eq!(mn, mx);
            assert!((mn - samples[10 + i]).abs() < 1e-6);
        }
        assert_eq!(raw.sample_rate_hz, 48_000);

        let bucketed = service.peaks(64, 0, 4).unwrap();
        assert_eq!(bucketed.buckets.len(), 4);
        for (b, chunk) in bucketed.buckets.iter().zip(samples.chunks(64)) {
            let (mn, mx) = chunk
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &s| {
                    (mn.min(s), mx.max(s))
                });
            assert!((b.0 - mn).abs() < 1e-6);
            assert!((b.1 - mx).abs() < 1e-6);
        }
    }

    /// Manual smoke check for the ticket's DoD ("report time to first waveform"): opens the
    /// `just fixtures` 60-min 48 kHz mono WAV (generating it on the fly if `just fixtures` hasn't
    /// been run, same fallback as `crates/project/tests/big.rs`) through the exact path
    /// `document_open` uses, and times it plus a first `peaks_get`-equivalent pyramid query.
    /// Ignored by default (minutes of I/O the first time); run with
    /// `cargo test -p powervoice-app --release -- --ignored open_and_first_peaks_of_the_60_min_fixture_report_timing --nocapture`.
    #[test]
    #[ignore]
    fn open_and_first_peaks_of_the_60_min_fixture_report_timing() {
        let dir = tmp_dir("sixty-min");
        let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/generated/long-60min-48k-mono.wav");
        let wav_path = if generated.exists() {
            generated
        } else {
            let path = dir.join("long-60min-48k-mono.wav");
            vox_testkit::signal::write_long_fixture(&path, 3600.0, 48_000, 42).unwrap();
            path
        };

        let (service, _engine, _dir2) = service("sixty-min");
        let start = std::time::Instant::now();
        let info = service.open(&wav_path).unwrap();
        let open_elapsed = start.elapsed();

        let peaks_start = std::time::Instant::now();
        let peaks = service.peaks(65_536, 0, 165).unwrap(); // coarsest level, whole 60-min file
        let peaks_elapsed = peaks_start.elapsed();

        println!(
            "document_open of the 60-min fixture: {:.3} s ({} samples); first coarse peaks_get: {:.3} ms ({} buckets)",
            open_elapsed.as_secs_f64(),
            info.len_samples,
            peaks_elapsed.as_secs_f64() * 1000.0,
            peaks.buckets.len(),
        );
        assert!(info.len_samples > 0);
    }

    #[test]
    fn opening_a_second_document_replaces_the_first() {
        let (service, _engine, dir) = service("replace");
        let a_path = dir.join("a.wav");
        let b_path = dir.join("b.wav");
        write_fixture_wav(
            &a_path,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        write_fixture_wav(
            &b_path,
            &vox_testkit::signal::sine(220.0, -6.0, 0.1, 48_000).unwrap(),
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );

        service.open(&a_path).unwrap();
        let info = service.open(&b_path).unwrap();
        assert_eq!(info.name.as_deref(), Some("b.wav"));
        let transport = service.0.engine.transport_state();
        assert_eq!(
            transport.doc_len_samples,
            (0.1 * 48_000.0) as u64,
            "the engine now plays b.wav, not a.wav"
        );
    }
}
