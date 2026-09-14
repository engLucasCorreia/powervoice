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
//!
//! T-304 (SPEC-022): **Record on a document with audio** (`record_start_at`) resolves per §2.2 in
//! the engine (`EngineHandle::record_prepare`: Insert / Overwrite at the cursor or selection
//! start, or a punch-in over the selection), opens the matching take
//! (`DocumentService::begin_record_op`) and runs it (`EngineHandle::record_start_op`) with the
//! device setup's recording offset (§2.13). When it ends, the record window is committed as one
//! edit (`DocumentService::commit_take_op`) or the operation is cancelled, and `record_finished`
//! tells the UI (selection and playhead per §2.10).

use std::sync::{Arc, Weak};

use tauri::{AppHandle, Emitter, Manager, Runtime};
use vox_engine::record::{
    CancelReason, LiveTakePeaks, OpResult, RecordDone, RecordError, RecordingResult, StopReason,
};
use vox_engine::record_op::{CursorRecordMode, RecordOpKind, RecordPrefs};
use vox_engine::{BufferRequest, EngineHandle, TransportCommand};

use crate::document::{DocumentInfo, DocumentService};
use crate::ipc::{
    DocumentDto, EventName, HistoryStateDto, IpcError, IpcErrorCode, Notice, NoticeLevel,
    RecordCancelDto, RecordFinishedDto, RecordStartedDto, RecordStateDto, emit_notice,
    engine_monitor_mode,
};
use crate::settings::{
    DefaultFormatDto, MonitorMode, RecordModePref, RecordOffsetEntry, RecordPrefsDto, Settings,
    SettingsStore, find_record_offset,
};

/// What the service tells the UI.
pub enum RecordingEvent {
    /// The current document changed (a new recording started, a take was committed).
    DocumentChanged(DocumentInfo),
    /// A notice (clip count, take errors, …).
    Notice(Notice),
    /// T-304: a record operation ended (committed or cancelled).
    Finished(RecordFinishedDto),
}

/// Emits [`RecordingEvent`]s (built by [`start`] from the app handle).
pub type RecordingEmitter = Arc<dyn Fn(RecordingEvent) + Send + Sync>;

/// Reads the current default recording format fresh from the settings store (H-06: New Recording
/// or File → New Recording… can change it any time — the service must not cache a stale value
/// from startup).
type DefaultFormatFn = Box<dyn Fn() -> DefaultFormatDto + Send + Sync>;

/// T-304: reads the Punch & pre-roll preferences and the stored recording offsets fresh from the
/// settings store (they change from the record panel and the calibration dialog).
pub type RecordSettingsFn = Box<dyn Fn() -> (RecordPrefsDto, Vec<RecordOffsetEntry>) + Send + Sync>;

struct Inner {
    engine: EngineHandle,
    documents: DocumentService,
    default_format: DefaultFormatFn,
    record_settings: RecordSettingsFn,
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
    let format_app = app.clone();
    let default_format: DefaultFormatFn =
        Box::new(move || format_app.state::<SettingsStore>().get().default_format);
    let record_app = app.clone();
    let record_settings: RecordSettingsFn = Box::new(move || {
        let s = record_app.state::<SettingsStore>().get();
        (s.record, s.record_offsets)
    });
    let app = app.clone();
    let emit: RecordingEmitter = Arc::new(move |event| {
        let result = match event {
            RecordingEvent::DocumentChanged(info) => {
                // H-21: a committed take, or a cancelled operation's markers turned into "Add
                // Marker" entries, also changes the Edit menu's Undo/Redo.
                if let Some(documents) = app.try_state::<DocumentService>() {
                    let history: HistoryStateDto = documents.history_state().into();
                    if let Err(e) = app.emit(EventName::history_state.as_str(), history) {
                        tracing::warn!(error = %e, "emitting history_state failed");
                    }
                }
                app.emit(
                    EventName::document_changed.as_str(),
                    DocumentDto::from(info),
                )
            }
            RecordingEvent::Notice(notice) => emit_notice(&app, notice),
            RecordingEvent::Finished(dto) => app.emit(EventName::record_finished.as_str(), dto),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "emitting a recording event failed");
        }
    });
    RecordingService::with_record_settings(
        engine,
        documents,
        default_format,
        record_settings,
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
        RecordError::PunchNeedsOutput => {
            IpcError::new(IpcErrorCode::DeviceNotFound, "error.punch_needs_output")
        }
        RecordError::InvalidPosition => {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.invalid_range")
        }
        RecordError::Calibrating => IpcError::new(IpcErrorCode::Busy, "error.calibration.busy"),
        other => IpcError::new(IpcErrorCode::Internal, "error.record.failed")
            .with_param("message", other.to_string()),
    }
}

/// "44.1 kHz"-style formatting of a sample rate (H-10 item 7, SPEC-002 §2.2's resampling
/// notice) — a whole number of kHz drops the decimal, matching the New Recording dialog's
/// labels ("48 kHz", "44.1 kHz").
fn format_rate_hz(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{:.1} kHz", f64::from(hz) / 1000.0)
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

/// T-304 (SPEC-022 §2.13): the device setup a recording offset is keyed by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSetup {
    pub host: String,
    pub input_device: String,
    pub output_device: String,
    /// The output stream's rate.
    pub device_rate_hz: u32,
    /// The output buffer request (`None`: Auto).
    pub buffer_frames: Option<u32>,
}

/// The current device setup (`None` unless an input and an output device are configured and the
/// output is open).
pub fn device_setup(engine: &EngineHandle) -> Option<DeviceSetup> {
    let view = engine.devices()?;
    Some(DeviceSetup {
        host: view.host?.as_str().to_owned(),
        input_device: view.input_device?,
        output_device: view.output_device?,
        device_rate_hz: view.output_rate_hz?,
        buffer_frames: match view.output_buffer {
            Some(BufferRequest::Frames(n)) => Some(n),
            _ => None,
        },
    })
}

/// The stored offset entry for `setup`, if any.
pub fn record_offset_for<'a>(
    setup: &DeviceSetup,
    entries: &'a [RecordOffsetEntry],
) -> Option<&'a RecordOffsetEntry> {
    find_record_offset(
        entries,
        &setup.host,
        &setup.input_device,
        &setup.output_device,
        setup.device_rate_hz,
    )
}

/// δ for the current device setup (0: not calibrated).
pub fn current_offset_ms(engine: &EngineHandle, settings: &Settings) -> f64 {
    device_setup(engine)
        .and_then(|s| record_offset_for(&s, &settings.record_offsets).map(|e| e.offset_ms))
        .unwrap_or(0.0)
}

/// The SPEC-022 §3 preferences the engine resolves a Record press with.
pub fn record_prefs(p: &RecordPrefsDto, offset_ms: f64) -> RecordPrefs {
    RecordPrefs {
        mode: match p.mode {
            RecordModePref::Insert => CursorRecordMode::Insert,
            RecordModePref::Overwrite => CursorRecordMode::Overwrite,
        },
        punch_on_selection: p.punch_on_selection,
        preroll_s: p.preroll_s,
        postroll_s: p.postroll_s,
        preroll_at_cursor: p.preroll_at_cursor,
        hear_original: p.hear_original,
        xfade_ms: p.punch_xfade_ms,
        offset_ms,
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
        op = ?result.op.map(|o| (o.plan.kind, o.window, o.cancelled)),
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
    if result.reason == StopReason::DiskFull {
        // H-11, SPEC-002 §2.5, AC-13: free space fell below the hard floor; say what was kept.
        notice(
            Notice::toast(NoticeLevel::Error, "notice.record.disk_full").with_param(
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
    if !result.dropouts.is_empty() {
        // H-10 item 4, SPEC-002 §2.4: `commit_take` below turns each into a "Dropout N ms"
        // marker, committed with the take. Deviation from the spec's exact wording: no "Go to
        // first" action on the notice itself (no action affordance exists on notices yet) — the
        // markers are visible/clickable in the Markers panel once the take commits.
        notice(
            Notice::toast(NoticeLevel::Warning, "notice.record.dropouts")
                .with_param("count", result.dropouts.len().to_string()),
        );
    }
    if let Some(op) = result.op {
        on_op_finished(inner, &result, op);
        return;
    }
    match inner
        .documents
        .commit_take(&result.finished, &result.dropouts)
    {
        Ok(Some(info)) => (inner.emit)(RecordingEvent::DocumentChanged(info)),
        Ok(None) => tracing::info!("empty take discarded"),
        Err(e) => {
            tracing::error!(error = %e, "committing the take failed; kept for recovery");
            let message = e.params.get("message").cloned().unwrap_or(e.key);
            notice(
                Notice::toast(NoticeLevel::Error, "notice.record.commit_failed")
                    .with_param("message", message),
            );
            // H-10 item 1: `commit_take` already replaced the stuck document with a fresh empty
            // one (`DocumentService::commit_take` docs) so the app stays usable — tell the UI so
            // the waveform/title reflect it instead of showing the old, now-abandoned document.
            (inner.emit)(RecordingEvent::DocumentChanged(inner.documents.info()));
        }
    }
}

/// T-304 (SPEC-022 §2.10–§2.12): commits a finished operation's window as one edit, or cancels
/// it (document unchanged, playhead back at the record point), then `record_finished`.
fn on_op_finished(inner: &Inner, result: &RecordingResult, op: OpResult) {
    let notice = |n: Notice| (inner.emit)(RecordingEvent::Notice(n));
    let take_id = result.finished.take.0;
    let kind = op.plan.kind;
    let dto = match inner
        .documents
        .commit_take_op(&result.finished, &op, &result.dropouts)
    {
        Ok(Some((info, edit))) => {
            (inner.emit)(RecordingEvent::DocumentChanged(info));
            if result.reason == StopReason::InputLost {
                // §2.10: a partial result at the last good sample.
                notice(
                    Notice::toast(NoticeLevel::Warning, "notice.punch_partial_input_lost")
                        .with_param(
                            "duration",
                            duration_text(
                                edit.playhead_samples.saturating_sub(op.plan.at_samples),
                                result.sample_rate_hz,
                            ),
                        ),
                );
            }
            RecordFinishedDto {
                take_id,
                op: kind.into(),
                committed: true,
                cancel_reason: None,
                result: Some(edit.into()),
            }
        }
        Ok(None) => {
            let reason = op.cancelled.unwrap_or(CancelReason::Aborted);
            let _ = inner
                .engine
                .transport(TransportCommand::Seek(op.plan.at_samples));
            // H-21 (SPEC-022 §2.9): markers added during the cancelled operation were committed
            // as "Add Marker" entries — refresh the marker list and the Edit menu.
            (inner.emit)(RecordingEvent::DocumentChanged(inner.documents.info()));
            if reason != CancelReason::User {
                // §2.10: "Punch-in cancelled — nothing changed" when not user-initiated.
                notice(Notice::toast(
                    NoticeLevel::Warning,
                    "notice.punch_cancelled",
                ));
            }
            RecordFinishedDto {
                take_id,
                op: kind.into(),
                committed: false,
                cancel_reason: Some(reason.into()),
                result: None,
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "committing the record operation failed");
            let message = e.params.get("message").cloned().unwrap_or(e.key);
            notice(
                Notice::toast(NoticeLevel::Error, "notice.record.commit_failed")
                    .with_param("message", message),
            );
            (inner.emit)(RecordingEvent::DocumentChanged(inner.documents.info()));
            RecordFinishedDto {
                take_id,
                op: kind.into(),
                committed: false,
                cancel_reason: Some(RecordCancelDto::Aborted),
                result: None,
            }
        }
    };
    (inner.emit)(RecordingEvent::Finished(dto));
}

impl RecordingService {
    pub fn new(
        engine: EngineHandle,
        documents: DocumentService,
        default_format: DefaultFormatFn,
        monitor: MonitorMode,
        emit: RecordingEmitter,
    ) -> Self {
        Self::with_record_settings(
            engine,
            documents,
            default_format,
            Box::new(|| (RecordPrefsDto::default(), Vec::new())),
            monitor,
            emit,
        )
    }

    /// [`Self::new`] with the T-304 record-settings source.
    pub fn with_record_settings(
        engine: EngineHandle,
        documents: DocumentService,
        default_format: DefaultFormatFn,
        record_settings: RecordSettingsFn,
        monitor: MonitorMode,
        emit: RecordingEmitter,
    ) -> Self {
        engine.set_record_rate(default_format().sample_rate_hz);
        let _ = engine.set_monitor_mode(engine_monitor_mode(monitor));
        Self(Arc::new(Inner {
            engine,
            documents,
            default_format,
            record_settings,
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

    /// Emits a notice through the recording events (T-107: the one-time headphone hint).
    pub fn notify(&self, notice: Notice) {
        (self.0.emit)(RecordingEvent::Notice(notice));
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

    /// T-304 (SPEC-022 §4.6): journals the opened window of `take` — on a worker, never on the
    /// engine's control thread (the event sink this comes from must not block on the document).
    pub fn note_window(&self, take: u32, k_start: u64) {
        let documents = self.0.documents.clone();
        let spawned = std::thread::Builder::new()
            .name("vox-take-window".into())
            .spawn(move || documents.note_take_window(take, k_start));
        if let Err(error) = spawned {
            tracing::warn!(%error, "journaling the record window failed");
        }
    }

    /// New recording (SPEC-002 §2.2): into the current document when it is empty, else into a
    /// new untitled one replacing it (only with `replace`: the UI ran the unsaved-changes
    /// prompt). `format`: the New Recording dialog's choice (H-06); `None` uses the current
    /// default format (Record with no document, no dialog).
    pub fn start(
        &self,
        replace: bool,
        format: Option<DefaultFormatDto>,
    ) -> Result<RecordStateDto, IpcError> {
        let inner = &self.0;
        let format = format.unwrap_or_else(|| (inner.default_format)());
        // The input opens at the chosen rate when the device supports it; if it can't, the
        // capture-writer resamples instead of refusing to record (H-06, SPEC-002 §2.2).
        inner.engine.set_record_rate(format.sample_rate_hz);
        let st = inner.engine.set_armed(true).ok_or_else(engine_stopped)?;
        if st.recording || st.finishing {
            return Err(IpcError::not_while_recording());
        }
        if st.input_device.is_none() {
            return Err(record_error(RecordError::NoInputDevice));
        }
        if !st.input_open {
            return Err(record_error(RecordError::InputNotOpen));
        }
        let (capture, info) =
            inner
                .documents
                .begin_recording(format.sample_rate_hz, format.bit_depth, replace)?;
        let take = capture.id();
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        let done: RecordDone = Box::new(move |result| {
            if let Some(inner) = weak.upgrade() {
                on_take_finished(&inner, result);
            }
        });
        match inner
            .engine
            .record_start(format.sample_rate_hz, capture, done)
        {
            Ok(st) => {
                if let Some(input_hz) = st.input_rate_hz
                    && input_hz != format.sample_rate_hz
                {
                    (inner.emit)(RecordingEvent::Notice(
                        Notice::toast(NoticeLevel::Info, "notice.record.resampled")
                            .with_param("input", format_rate_hz(input_hz))
                            .with_param("document", format_rate_hz(format.sample_rate_hz)),
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

    /// T-304 (SPEC-022 §2.2, §4.9 `record_start`): Record with the current `selection`. An empty
    /// document records a new take into itself (row 1); otherwise the engine resolves the press
    /// (punch-in over a non-empty selection, else Insert/Overwrite at the selection start or the
    /// cursor, stopping a playing transport first) and the operation starts.
    pub fn start_at(&self, selection: Option<(u64, u64)>) -> Result<RecordStartedDto, IpcError> {
        let inner = &self.0;
        if inner.documents.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let new_recording = |st: RecordStateDto| RecordStartedDto {
            take_id: 0,
            op: RecordOpKind::New.into(),
            at_samples: 0,
            end_samples: None,
            preroll_samples: 0,
            postroll_samples: 0,
            aligned: false,
            state: st,
        };
        if inner.documents.current_len_samples() == 0 {
            return self.start(false, None).map(new_recording);
        }
        let st = inner.engine.set_armed(true).ok_or_else(engine_stopped)?;
        if st.recording || st.finishing {
            return Err(IpcError::not_while_recording());
        }
        if st.input_device.is_none() {
            return Err(record_error(RecordError::NoInputDevice));
        }
        if !st.input_open {
            return Err(record_error(RecordError::InputNotOpen));
        }
        let (prefs, offsets) = (inner.record_settings)();
        let offset_ms = device_setup(&inner.engine)
            .and_then(|s| record_offset_for(&s, &offsets).map(|e| e.offset_ms))
            .unwrap_or(0.0);
        let plan = inner
            .engine
            .record_prepare(selection, record_prefs(&prefs, offset_ms))
            .map_err(record_error)?;
        if plan.kind == RecordOpKind::New {
            return self.start(false, None).map(new_recording);
        }
        let capture = inner.documents.begin_record_op(&plan)?;
        let take = capture.id();
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        let done: RecordDone = Box::new(move |result| {
            if let Some(inner) = weak.upgrade() {
                on_take_finished(&inner, result);
            }
        });
        match inner.engine.record_start_op(plan, capture, done) {
            Ok(st) => Ok(RecordStartedDto {
                take_id: take.0,
                op: plan.kind.into(),
                at_samples: plan.at_samples,
                end_samples: plan.end_samples,
                preroll_samples: plan.preroll_samples,
                postroll_samples: plan.postroll_samples,
                aligned: plan.aligned,
                state: RecordStateDto::from(&st),
            }),
            Err(e) => {
                inner.documents.discard_take(take);
                Err(record_error(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{duration_text, format_rate_hz, record_error, record_prefs};
    use crate::settings::{RecordModePref, RecordPrefsDto};
    use vox_engine::record::RecordError;
    use vox_engine::record_op::CursorRecordMode;

    #[test]
    fn duration_text_is_m_ss_then_h_mm_ss() {
        assert_eq!(duration_text(0, 48_000), "0:00");
        assert_eq!(duration_text(48_000 * 751 + 47_999, 48_000), "12:31");
        assert_eq!(duration_text(44_100 * 3_661, 44_100), "1:01:01");
    }

    /// H-10 item 7: the resampling notice shows "44.1 kHz"-style rates, not raw Hz.
    #[test]
    fn format_rate_hz_is_khz_with_a_decimal_only_when_needed() {
        assert_eq!(format_rate_hz(44_100), "44.1 kHz");
        assert_eq!(format_rate_hz(48_000), "48 kHz");
        assert_eq!(format_rate_hz(88_200), "88.2 kHz");
        assert_eq!(format_rate_hz(96_000), "96 kHz");
    }

    /// T-304: the settings map onto the engine's resolution prefs; a punch without an output is
    /// `error.punch_needs_output` (SPEC-022 §2.2).
    #[test]
    fn record_prefs_and_errors_map() {
        let p = record_prefs(
            &RecordPrefsDto {
                mode: RecordModePref::Overwrite,
                preroll_s: 2.5,
                ..RecordPrefsDto::default()
            },
            3.0,
        );
        assert_eq!(p.mode, CursorRecordMode::Overwrite);
        assert_eq!((p.preroll_s, p.offset_ms, p.xfade_ms), (2.5, 3.0, 10.0));
        assert!(p.punch_on_selection);
        assert_eq!(
            record_error(RecordError::PunchNeedsOutput).key,
            "error.punch_needs_output"
        );
    }
}
