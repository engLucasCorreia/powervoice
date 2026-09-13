//! Recording service (S1-04, SPEC-002 §2.2): the app side of a new recording. The engine owns
//! the streams and threads (`vox_engine::record`); the document service (S1-03) owns the session.
//!
//! - **Record**: arms the input (its rate is the new document's rate), asks the document service
//!   for the take (`DocumentService::begin_recording`: an empty document records into itself, a
//!   document with audio is replaced after the UI's unsaved-changes prompt) and hands it to the
//!   engine.
//! - **Take finished** (the engine's done callback, on the capture-writer thread): posts the take
//!   notices, commits it (`DocumentService::commit_take`: one undoable "Record" edit, then
//!   `EngineHandle::set_document`) and emits `document_changed`, so the waveform shows the take.

use std::sync::{Arc, Weak};

use tauri::{AppHandle, Emitter, Runtime};
use vox_engine::EngineHandle;
use vox_engine::record::{LiveTakePeaks, RecordDone, RecordError, RecordingResult, StopReason};

use crate::document::{DocumentInfo, DocumentService};
use crate::ipc::{
    DocumentDto, EventName, IpcError, IpcErrorCode, Notice, NoticeLevel, RecordStateDto,
    emit_notice, engine_monitor_mode,
};
use crate::settings::{BitDepth, MonitorMode, Settings};

/// What the service tells the UI.
pub enum RecordingEvent {
    /// The current document changed (a new recording started, a take was committed).
    DocumentChanged(DocumentInfo),
    /// A notice (clip count, take errors, …).
    Notice(Notice),
}

/// Emits [`RecordingEvent`]s (built by [`start`] from the app handle).
pub type RecordingEmitter = Arc<dyn Fn(RecordingEvent) + Send + Sync>;

struct Inner {
    engine: EngineHandle,
    documents: DocumentService,
    preferred_rate_hz: u32,
    save_bits: BitDepth,
    emit: RecordingEmitter,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state.
#[derive(Clone)]
pub struct RecordingService(Arc<Inner>);

/// Builds the service for the running app: new recordings use the default format (rate, save
/// bit depth) and the saved monitoring mode applies (SPEC-002 §2.2, §2.7).
pub fn start<R: Runtime>(
    app: &AppHandle<R>,
    engine: EngineHandle,
    documents: DocumentService,
    settings: &Settings,
) -> RecordingService {
    let app = app.clone();
    let emit: RecordingEmitter = Arc::new(move |event| {
        let result = match event {
            RecordingEvent::DocumentChanged(info) => app.emit(
                EventName::document_changed.as_str(),
                DocumentDto::from(info),
            ),
            RecordingEvent::Notice(notice) => emit_notice(&app, notice),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "emitting a recording event failed");
        }
    });
    RecordingService::new(
        engine,
        documents,
        settings.default_format.sample_rate_hz,
        settings.default_format.bit_depth,
        settings.monitor_mode,
        emit,
    )
}

fn engine_stopped() -> IpcError {
    IpcError::internal("the audio engine is not running")
}

fn record_error(e: RecordError) -> IpcError {
    match e {
        RecordError::AlreadyRecording => IpcError::not_while_recording(),
        RecordError::NoInputDevice => {
            IpcError::new(IpcErrorCode::DeviceNotFound, "error.record.no_input")
        }
        RecordError::InputNotOpen => {
            IpcError::new(IpcErrorCode::DeviceLost, "error.record.input_unavailable")
        }
        other => IpcError::new(IpcErrorCode::Internal, "error.record.failed")
            .with_param("message", other.to_string()),
    }
}

/// `m:ss` (`h:mm:ss` from one hour) of `samples` at `rate_hz`, for notices ("12:31 recorded").
fn duration_text(samples: u64, rate_hz: u32) -> String {
    let s = samples / u64::from(rate_hz.max(1));
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The done callback's work (see the module docs). Runs on the capture-writer thread.
fn on_take_finished(inner: &Inner, result: RecordingResult) {
    let notice = |n: Notice| (inner.emit)(RecordingEvent::Notice(n));
    tracing::info!(
        reason = ?result.reason,
        samples = result.finished.wav_samples,
        clips = result.clip_events,
        overflow = result.overflow_samples,
        "take finished"
    );
    for e in [&result.finished.error, &result.write_error]
        .into_iter()
        .flatten()
    {
        tracing::error!(error = %e, "take error");
        notice(
            Notice::toast(NoticeLevel::Error, "notice.record.take_error")
                .with_param("message", e.to_string()),
        );
    }
    if result.reason == StopReason::Overflow {
        // H-05, SPEC-002 §2.4: the take ended by itself; say why and what was kept.
        notice(
            Notice::toast(NoticeLevel::Error, "notice.record.overflow").with_param(
                "duration",
                duration_text(result.finished.wav_samples, result.sample_rate_hz),
            ),
        );
    }
    if result.clip_events > 0 {
        notice(
            Notice::toast(NoticeLevel::Warning, "notice.record.clipped")
                .with_param("count", result.clip_events.to_string()),
        );
    }
    match inner.documents.commit_take(&result.finished) {
        Ok(Some(info)) => (inner.emit)(RecordingEvent::DocumentChanged(info)),
        Ok(None) => tracing::info!("empty take discarded"),
        Err(e) => {
            tracing::error!(error = %e, "committing the take failed; kept for recovery");
            let message = e.params.get("message").cloned().unwrap_or(e.key);
            notice(
                Notice::toast(NoticeLevel::Error, "notice.record.commit_failed")
                    .with_param("message", message),
            );
        }
    }
}

impl RecordingService {
    pub fn new(
        engine: EngineHandle,
        documents: DocumentService,
        preferred_rate_hz: u32,
        save_bits: BitDepth,
        monitor: MonitorMode,
        emit: RecordingEmitter,
    ) -> Self {
        engine.set_record_rate(preferred_rate_hz);
        let _ = engine.set_monitor_mode(engine_monitor_mode(monitor));
        Self(Arc::new(Inner {
            engine,
            documents,
            preferred_rate_hz,
            save_bits,
            emit,
        }))
    }

    pub fn state(&self) -> Result<RecordStateDto, IpcError> {
        let st = self.0.engine.record_state().ok_or_else(engine_stopped)?;
        Ok(RecordStateDto::from(&st))
    }

    pub fn arm(&self, armed: bool) -> Result<RecordStateDto, IpcError> {
        let st = self.0.engine.set_armed(armed).ok_or_else(engine_stopped)?;
        Ok(RecordStateDto::from(&st))
    }

    pub fn set_monitor(&self, mode: MonitorMode) -> Result<RecordStateDto, IpcError> {
        let st = self
            .0
            .engine
            .set_monitor_mode(engine_monitor_mode(mode))
            .ok_or_else(engine_stopped)?;
        Ok(RecordStateDto::from(&st))
    }

    pub fn stop(&self) -> Result<RecordStateDto, IpcError> {
        let st = self.0.engine.record_stop().ok_or_else(engine_stopped)?;
        Ok(RecordStateDto::from(&st))
    }

    /// H-07: a snapshot of the take being captured's running peaks (`None`: not recording). Thin
    /// forward to `EngineHandle::live_take_peaks` — `record_peaks_get` encodes the `VXPK` frame.
    pub fn live_peaks(&self, start_bucket: u32, count: u32) -> Option<LiveTakePeaks> {
        self.0.engine.live_take_peaks(start_bucket, count)
    }

    /// New recording (SPEC-002 §2.2): into the current document when it is empty, else into a
    /// new untitled one replacing it (only with `replace`: the UI ran the unsaved-changes prompt).
    pub fn start(&self, replace: bool) -> Result<RecordStateDto, IpcError> {
        let inner = &self.0;
        let st = inner.engine.set_armed(true).ok_or_else(engine_stopped)?;
        if st.recording || st.finishing {
            return Err(IpcError::not_while_recording());
        }
        if st.input_device.is_none() {
            return Err(record_error(RecordError::NoInputDevice));
        }
        let rate = st
            .input_rate_hz
            .ok_or_else(|| record_error(RecordError::InputNotOpen))?;
        let (capture, info) = inner
            .documents
            .begin_recording(rate, inner.save_bits, replace)?;
        let take = capture.id();
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        let done: RecordDone = Box::new(move |result| {
            if let Some(inner) = weak.upgrade() {
                on_take_finished(&inner, result);
            }
        });
        match inner.engine.record_start(rate, capture, done) {
            Ok(st) => {
                if rate != inner.preferred_rate_hz {
                    (inner.emit)(RecordingEvent::Notice(
                        Notice::toast(NoticeLevel::Warning, "notice.record.rate_changed")
                            .with_param("rate", rate.to_string())
                            .with_param("preferred", inner.preferred_rate_hz.to_string()),
                    ));
                }
                (inner.emit)(RecordingEvent::DocumentChanged(info));
                Ok(RecordStateDto::from(&st))
            }
            Err(e) => {
                inner.documents.discard_take(take);
                Err(record_error(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::duration_text;

    #[test]
    fn duration_text_is_m_ss_then_h_mm_ss() {
        assert_eq!(duration_text(0, 48_000), "0:00");
        assert_eq!(duration_text(48_000 * 751 + 47_999, 48_000), "12:31");
        assert_eq!(duration_text(44_100 * 3_661, 44_100), "1:01:01");
    }
}
