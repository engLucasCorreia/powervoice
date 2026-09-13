//! Peak normalize (SPEC-010): a brute-force peak scan of the scope (§4 step 2), the no-op/refusal
//! decision table (§2.7), and the gained write-back through a [`ChunkWriter`] (§4 step 4) that
//! preserves `Silence` pieces bit-for-bit. One [`crate::history::Edit`] with
//! [`MarkerMapping::Identity`] (length-preserving, no marker moves — §2.1).
//!
//! Gain computation and the write live in `project` (ADR-001 §4: "`project` owns edit ops incl.
//! normalize gain computation").
//!
//! [`normalize_lufs`] (S4-01) is the same shape, driven by a streamed BS.1770 loudness scan
//! ([`vox_dsp::loudness::LoudnessMeter`]) instead of a peak scan, reusing [`write_gained`] for the
//! write-back so silence pieces, undo and marker behaviour match exactly.
//!
//! **H-09** adds the job path `src-tauri` drives for both: [`plan_normalize_peak`] and
//! [`plan_normalize_lufs`] run pass 1 and pass 2 against an `Arc<ChunkStore>`/`Arc<DocSnapshot>`
//! (no `&mut Session` needed until the very end), cancellable via [`CancelToken`] and reporting
//! progress, and hand back a plan — a ready-to-commit [`Edit`], or a no-op outcome — for the
//! caller to commit through [`Session::commit_edit`] once it has reacquired the session. Pass 1 of
//! [`plan_normalize_peak`] uses [`scan_peak_accelerated`] (SPEC-010 §4 step 2's "optional
//! acceleration", AC-9/AC-15): a piece referencing a whole, untouched chunk reads that chunk's
//! pyramid instead of a raw scan. [`normalize_peak`]/[`normalize_lufs`] (the synchronous S2-02/
//! S4-01 entry points) are unchanged and still used directly by this module's own tests.

use std::sync::Arc;

use crate::edit::{PostEdit, Range};
use crate::history::{Edit, HistoryStep, MarkerMapping};
use crate::reader::read_range;
use crate::session::Session;
use crate::snapshot::{DocSnapshot, Piece, Source};
use crate::store::{CancelToken, ChunkStore, ChunkWriter};
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// Undo label key (SPEC-010 §2.8; en: "Normalize").
pub const LABEL_NORMALIZE: &str = "history.normalize";

/// Near-silence floor: below this the scope is treated as silent (SPEC-010 §2.7, §3
/// `min_peak_dbfs`).
pub const MIN_PEAK_DBFS: f64 = -120.0;

/// "Already normalized" tolerance: a gain closer to 0 dB than this is a no-op (SPEC-010 §2.7, §3
/// `already_tol_db`).
pub const ALREADY_TOL_DB: f64 = 0.001;

/// Samples read/written per buffer in both passes (one chunk's worth — bounds memory regardless
/// of scope length; the 60-min/pyramid performance work is deferred, ticket scope).
const IO_BUF_SAMPLES: usize = CHUNK_SAMPLES;

/// Why a normalize command did nothing (SPEC-010 §2.7); `EditResult.changed` is `false` either
/// way. The caller (the IPC layer) turns this into `notice.normalize_silent` /
/// `notice.normalize_already`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizeOutcome {
    /// `P == 0` or `P < `[`MIN_PEAK_DBFS`]` dBFS.
    Silent,
    /// `|20·log10 g| < `[`ALREADY_TOL_DB`].
    AlreadyNormalized,
}

/// The result of [`normalize_peak`].
#[derive(Clone, Debug)]
pub enum NormalizeResult {
    /// Nothing changed: no commit, no undo entry.
    NoOp(NormalizeOutcome),
    /// The scope was gained and committed as one undoable edit.
    Applied(HistoryStep),
}

/// `T = 10^(target_db / 20)` (SPEC-010 §2.2), in f64.
pub fn target_linear(target_db: f64) -> f64 {
    10f64.powf(target_db / 20.0)
}

/// SPEC-010 §2.4/§3 `target_pct`: converts a percent-of-full-scale target (100 % = 0 dBFS) to the
/// dB value [`normalize_peak`]/[`plan_normalize_peak`] take (H-09: the Normalize… dialog's %
/// mode). `p = 0` gives `-inf` (rejected by the dialog's `[0.1, 100.0]` range, not by this
/// function).
pub fn target_pct_to_db(target_pct: f64) -> f64 {
    20.0 * (target_pct / 100.0).log10()
}

/// The inverse of [`target_pct_to_db`] (H-09: switching the dialog's unit toggle converts the
/// shown value, SPEC-010 §2.4).
pub fn target_db_to_pct(target_db: f64) -> f64 {
    100.0 * target_linear(target_db)
}

/// Pass 1 (SPEC-010 §4 step 2): the sample peak `P = max |x[i]|` of `range` in `snapshot`, in
/// linear amplitude. `Silence` pieces contribute 0 and cost no I/O ([`read_range`] fills them with
/// `0.0` without touching the store). Any non-finite sample aborts with
/// [`ProjectError::NonFiniteSample`] (`error.normalize_non_finite`) — defensive, since the store
/// should never hold one.
pub fn scan_peak(store: &ChunkStore, snapshot: &DocSnapshot, range: Range) -> Result<f64> {
    let mut buf = vec![0.0f32; IO_BUF_SAMPLES.min(range.len_samples().max(1) as usize)];
    let mut pos = range.start;
    let mut peak = 0.0f64;
    while pos < range.end {
        let want = ((range.end - pos) as usize).min(buf.len());
        let n = read_range(store, snapshot, pos, &mut buf[..want], &mut None)?;
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            if !s.is_finite() {
                return Err(ProjectError::NonFiniteSample);
            }
            peak = peak.max(f64::from(s.abs()));
        }
        pos += n as u64;
    }
    Ok(peak)
}

/// Pass 1's raw fallback over one sub-range `[start, end)` (a partial chunk reference, or the
/// non-finite check itself): the same read/fold/check loop as [`scan_peak`], but cancellable and
/// progress-reporting (`on_progress` is called with the number of samples this call advanced by,
/// after each buffer — H-09's job path).
fn scan_peak_raw_range(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    start: u64,
    end: u64,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(u64),
) -> Result<f64> {
    let mut buf = vec![0.0f32; IO_BUF_SAMPLES.min((end - start).max(1) as usize)];
    let mut pos = start;
    let mut peak = 0.0f64;
    while pos < end {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let want = ((end - pos) as usize).min(buf.len());
        let n = read_range(store, snapshot, pos, &mut buf[..want], &mut None)?;
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            if !s.is_finite() {
                return Err(ProjectError::NonFiniteSample);
            }
            peak = peak.max(f64::from(s.abs()));
        }
        pos += n as u64;
        on_progress(n as u64);
    }
    Ok(peak)
}

/// Pyramid-accelerated pass 1 (SPEC-010 §4 step 2, AC-9/AC-15; H-09): like [`scan_peak`], but a
/// piece that references a *whole, untouched* chunk (`offset == 0`, `len` equal to the chunk's
/// own length) reads that chunk's [`crate::store::ChunkPeaks::overall`] instead of its raw
/// samples — `max(|min|, |max|)` of the coarsest pyramid bucket is the chunk's *exact* sample
/// peak, since ADR-004 §5 stores true min/max, not an estimate, and that bucket spans the whole
/// chunk (`PEAK_LEVELS_SPP`'s top level equals [`CHUNK_SAMPLES`]). A chunk whose pyramid flags a
/// non-finite sample ([`crate::store::ChunkPeaks::has_non_finite`]) aborts at once with
/// [`ProjectError::NonFiniteSample`] — sound because "whole chunk" means that sample, wherever it
/// is, lies inside this piece. Every other piece (a partial chunk reference — what an edited
/// document usually has) falls back to [`scan_peak_raw_range`], so the non-finite check is never
/// weakened by acceleration. `Silence` pieces cost no I/O, as in [`scan_peak`]. Cancellable
/// ([`CancelToken`], checked at least once per piece) and progress-reporting (`on_progress` is
/// called with each piece's/buffer's sample count, cumulative to `range.len_samples()`).
pub fn scan_peak_accelerated(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    range: Range,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(u64),
) -> Result<f64> {
    let scope = snapshot.slice(range.start, range.len_samples())?;
    let mut peak = 0.0f64;
    let mut pos = range.start;
    for piece in &scope {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let len = piece.len_samples();
        match piece.source {
            Source::Silence => {
                on_progress(len);
            }
            Source::Chunk(id) => {
                let mut accelerated = false;
                if piece.offset == 0 {
                    let chunk_peaks = store.chunk_peaks(id)?;
                    if u64::from(piece.len) == u64::from(chunk_peaks.len_samples()) {
                        if chunk_peaks.has_non_finite() {
                            return Err(ProjectError::NonFiniteSample);
                        }
                        let [mn, mx] = chunk_peaks.overall();
                        peak = peak.max(f64::from(mn.abs())).max(f64::from(mx.abs()));
                        on_progress(len);
                        accelerated = true;
                    }
                }
                if !accelerated {
                    peak = peak.max(scan_peak_raw_range(
                        store,
                        snapshot,
                        pos,
                        pos + len,
                        cancel,
                        &mut on_progress,
                    )?);
                }
            }
        }
        pos += len;
    }
    Ok(peak)
}

/// What to do about a scope whose sample peak is `peak` (linear amplitude), targeting `target_db`
/// dBFS (SPEC-010 §2.7's decision table, `P = 0`/non-finite rows excluded — the caller already
/// handled/ruled those out).
enum Decision {
    NoOp(NormalizeOutcome),
    Gain(f64),
}

fn decide(peak: f64, target_db: f64) -> Decision {
    // `peak.log10()` is `-inf` at `peak == 0.0`, which is `< MIN_PEAK_DBFS` — covers both the
    // digital-silence and near-silence rows of §2.7's table in one comparison.
    if 20.0 * peak.log10() < MIN_PEAK_DBFS {
        return Decision::NoOp(NormalizeOutcome::Silent);
    }
    let gain = target_linear(target_db) / peak;
    if (20.0 * gain.log10()).abs() < ALREADY_TOL_DB {
        return Decision::NoOp(NormalizeOutcome::AlreadyNormalized);
    }
    Decision::Gain(gain)
}

/// A maximal run of consecutive scope pieces that are all `Silence` or all not (SPEC-010 §4 step
/// 4: silence runs are preserved as-is; audio runs are read, gained and rewritten).
struct Run {
    silence: bool,
    len_samples: u64,
}

fn runs_of(pieces: &[Piece]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for piece in pieces {
        let silence = matches!(piece.source, Source::Silence);
        match runs.last_mut() {
            Some(last) if last.silence == silence => last.len_samples += piece.len_samples(),
            _ => runs.push(Run {
                silence,
                len_samples: piece.len_samples(),
            }),
        }
    }
    runs
}

/// Pass 2 (SPEC-010 §4 step 4): gains every non-silence sample of `range` by `gain` (one f64
/// multiplication, one rounding to f32) and streams it through `writer`; `Silence` pieces are
/// preserved verbatim (no I/O). Returns the new pieces for `range`, in order, ready for
/// [`Edit::replace_with`]. `on_progress` is called with each buffer's/run's sample count,
/// cumulative to `range.len_samples()` (H-09's job path; existing callers pass a no-op).
/// Cancellation is `writer`'s own (a [`ChunkWriter::with_cancel`] token, checked on every
/// `append`) — [`ProjectError::Cancelled`] then propagates from here exactly like any other I/O
/// error.
fn write_gained(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    sample_rate_hz: u32,
    range: Range,
    gain: f64,
    mut writer: ChunkWriter,
    mut on_progress: impl FnMut(u64),
) -> Result<Vec<Piece>> {
    let scope = snapshot.slice(range.start, range.len_samples())?;
    let runs = runs_of(&scope);

    let mut buf = vec![0.0f32; IO_BUF_SAMPLES];
    let mut pos = range.start;
    for run in &runs {
        if run.silence {
            pos += run.len_samples;
            on_progress(run.len_samples);
            continue;
        }
        let mut remaining = run.len_samples;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            let n = read_range(store, snapshot, pos, &mut buf[..want], &mut None)?;
            if n == 0 {
                // The scope's own length already bounds every read; a short read here would mean
                // the piece table and the document length disagree.
                return Err(ProjectError::InvalidEdit(
                    "normalize: scope ended before its own pieces did".into(),
                ));
            }
            for s in &mut buf[..n] {
                *s = (f64::from(*s) * gain) as f32;
            }
            writer.append(&buf[..n])?;
            pos += n as u64;
            remaining -= n as u64;
            on_progress(n as u64);
        }
    }
    let written = writer.finish()?;

    // The writer's pieces are the flat, chunk-boundary-split concatenation of every audio run's
    // gained samples, in append order — re-slice them back into the runs' shape using the same,
    // already-tested piece-splitting code `DocSnapshot::slice` uses.
    let written_snapshot = DocSnapshot::new(sample_rate_hz, written.pieces, Vec::new());
    let mut new_pieces = Vec::with_capacity(runs.len());
    let mut cursor = 0u64;
    for run in &runs {
        if run.silence {
            new_pieces.extend(Piece::silence_run(run.len_samples));
        } else {
            new_pieces.extend(written_snapshot.slice(cursor, run.len_samples)?);
            cursor += run.len_samples;
        }
    }
    Ok(new_pieces)
}

/// After an applied normalize: the selection and playhead are unchanged (SPEC-010 §2.1 —
/// length-preserving, no marker moves either). Exposed for `DocumentService` (S2-02).
pub fn applied_post_edit(range: Range) -> PostEdit {
    PostEdit {
        selection: Some((range.start, range.end)),
        playhead: range.start,
    }
}

/// Runs SPEC-010 end to end against `session`'s current snapshot: peak-scans `range`, decides
/// per §2.7, and — unless it's a no-op — gains the scope and commits it as one undoable edit
/// labelled [`LABEL_NORMALIZE`]. Refused while recording ([`ProjectError::NotWhileRecording`],
/// checked by [`Session::commit_edit`]) — callers that must refuse before doing any I/O (SPEC-010
/// §2.8/AC-11) should check [`Session::is_recording`] first, as `DocumentService::edit_normalize_peak`
/// (S2-02) does.
pub fn normalize_peak(
    session: &mut Session,
    range: Range,
    target_db: f64,
) -> Result<NormalizeResult> {
    let snapshot = session.current();
    let store = Arc::clone(session.store());
    let peak = scan_peak(&store, &snapshot, range)?;
    match decide(peak, target_db) {
        Decision::NoOp(outcome) => Ok(NormalizeResult::NoOp(outcome)),
        Decision::Gain(gain) => {
            let writer = session.chunk_writer();
            let new_pieces = write_gained(
                &store,
                &snapshot,
                session.sample_rate_hz(),
                range,
                gain,
                writer,
                |_| {},
            )?;
            let edit = Edit::new(LABEL_NORMALIZE).replace_with(
                range.start,
                range.len_samples(),
                new_pieces,
                MarkerMapping::Identity,
            );
            let step = session.commit_edit(edit)?;
            Ok(NormalizeResult::Applied(step))
        }
    }
}

/// Undo label key for LUFS normalize favorites (S4-01; en: "Normalize (LUFS)").
pub const LABEL_NORMALIZE_LUFS: &str = "history.normalize_lufs";

/// BS.1770's absolute gate (docs/references.md: "absolute gate −70 LUFS"): an integrated loudness
/// at or below this means no block in the scope was above the gate (ebur128 reports a very
/// negative/`-inf` sentinel for a scope with no audio above it) — treated as silent, mirroring
/// [`MIN_PEAK_DBFS`]'s role for peak normalize.
pub const MIN_INTEGRATED_LUFS: f64 = -70.0;

/// The true-peak ceiling LUFS normalize warns about without limiting (S4-01 ticket: "if the
/// resulting true peak would exceed −1 dBTP, apply the gain anyway and show a notice suggesting
/// the true-peak limiter — don't silently limit").
pub const TRUE_PEAK_WARN_CEILING_DBTP: f64 = -1.0;

/// Why a LUFS normalize command did nothing (mirrors [`NormalizeOutcome`]); `EditResult.changed`
/// is `false` either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LufsNormalizeOutcome {
    /// The scope has no block above the BS.1770 absolute gate (silence or near-silence).
    Silent,
    /// `|target_lufs - measured_integrated_lufs| < `[`ALREADY_TOL_DB`].
    AlreadyNormalized,
}

/// The result of [`normalize_lufs`].
#[derive(Clone, Debug)]
pub enum LufsNormalizeResult {
    /// Nothing changed: no commit, no undo entry.
    NoOp(LufsNormalizeOutcome),
    /// The scope was gained and committed as one undoable edit.
    Applied {
        step: HistoryStep,
        /// The gain applied, in dB (`target_lufs - measured_integrated_lufs`).
        gain_db: f64,
        /// The measured true peak shifted by exactly `gain_db` (a linear gain) — predicts the
        /// scope's post-gain true peak without a second scan.
        predicted_true_peak_dbtp: f64,
        /// `predicted_true_peak_dbtp > `[`TRUE_PEAK_WARN_CEILING_DBTP`] — the gain is applied
        /// anyway; the caller shows a notice suggesting the true-peak limiter (S4-01 ticket, no
        /// silent limiting).
        exceeds_true_peak_ceiling: bool,
    },
}

/// Pass 1 for LUFS normalize (mirrors [`scan_peak`]): streams `range` through a
/// [`vox_dsp::loudness::LoudnessMeter`] and returns its report. `Silence` pieces contribute
/// digital-silence samples and cost no I/O, exactly like [`scan_peak`]. Any non-finite sample
/// aborts with [`ProjectError::NonFiniteSample`] — defensive, since the store should never hold
/// one (checked here, before ever handing a sample to `ebur128`, rather than relying on the
/// meter's own non-finite handling, which is meant for renderer output, not the store's
/// invariants). `cancel` is checked once per buffer (H-09's job path; a fresh, never-cancelled
/// [`CancelToken`] for the synchronous callers below); `on_progress` is called with each buffer's
/// sample count, cumulative to `range.len_samples()`. There is no pyramid acceleration here (SPEC-
/// 010 §4's acceleration is for the peak scan only — BS.1770 gating needs every sample).
pub fn scan_loudness(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    range: Range,
    sample_rate_hz: u32,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(u64),
) -> Result<vox_dsp::loudness::LoudnessReport> {
    let mut meter = vox_dsp::loudness::LoudnessMeter::new(sample_rate_hz)?;
    let mut buf = vec![0.0f32; IO_BUF_SAMPLES.min(range.len_samples().max(1) as usize)];
    let mut pos = range.start;
    while pos < range.end {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let want = ((range.end - pos) as usize).min(buf.len());
        let n = read_range(store, snapshot, pos, &mut buf[..want], &mut None)?;
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            if !s.is_finite() {
                return Err(ProjectError::NonFiniteSample);
            }
        }
        meter.push(&buf[..n])?;
        pos += n as u64;
        on_progress(n as u64);
    }
    Ok(meter.finish()?)
}

/// Runs the S4-01 LUFS normalize end to end against `session`'s current snapshot: measures
/// `range`'s integrated loudness (BS.1770/EBU R128 of the raw document samples — the same scope
/// peak normalize would scan, not the rack-processed signal the loudness analysis job can show),
/// decides per the table above, and — unless it's a no-op — gains the scope by `target_lufs -
/// measured` and commits it as one undoable edit labelled [`LABEL_NORMALIZE_LUFS`], reusing
/// [`write_gained`] so it behaves exactly like [`normalize_peak`] (SPEC-010's write path). Refused
/// while recording ([`ProjectError::NotWhileRecording`], checked by [`Session::commit_edit`]) —
/// callers that must refuse before doing any I/O should check [`Session::is_recording`] first, as
/// `DocumentService::edit_normalize_lufs` (S4-01) does.
pub fn normalize_lufs(
    session: &mut Session,
    range: Range,
    target_lufs: f64,
) -> Result<LufsNormalizeResult> {
    let snapshot = session.current();
    let store = Arc::clone(session.store());
    let sample_rate_hz = session.sample_rate_hz();
    let report = scan_loudness(
        &store,
        &snapshot,
        range,
        sample_rate_hz,
        &CancelToken::new(),
        |_| {},
    )?;

    if report.integrated_lufs <= MIN_INTEGRATED_LUFS {
        return Ok(LufsNormalizeResult::NoOp(LufsNormalizeOutcome::Silent));
    }
    let gain_db = target_lufs - report.integrated_lufs;
    if gain_db.abs() < ALREADY_TOL_DB {
        return Ok(LufsNormalizeResult::NoOp(
            LufsNormalizeOutcome::AlreadyNormalized,
        ));
    }
    let gain = 10f64.powf(gain_db / 20.0);
    let writer = session.chunk_writer();
    let new_pieces = write_gained(
        &store,
        &snapshot,
        sample_rate_hz,
        range,
        gain,
        writer,
        |_| {},
    )?;
    let edit = Edit::new(LABEL_NORMALIZE_LUFS).replace_with(
        range.start,
        range.len_samples(),
        new_pieces,
        MarkerMapping::Identity,
    );
    let step = session.commit_edit(edit)?;
    let predicted_true_peak_dbtp = report.true_peak_dbtp + gain_db;
    Ok(LufsNormalizeResult::Applied {
        step,
        gain_db,
        predicted_true_peak_dbtp,
        exceeds_true_peak_ceiling: predicted_true_peak_dbtp > TRUE_PEAK_WARN_CEILING_DBTP,
    })
}

// --- H-09: the job path (progress, cancel) ------------------------------------------------------
//
// `src-tauri` runs a normalize as a job on its own thread (mirrors export/nr_capture/loudness,
// MEMORY.md S4-04): pass 1 and pass 2 run against an `Arc<ChunkStore>`/`Arc<DocSnapshot>` with no
// `&mut Session` needed, so the job thread never holds the document lock while scanning/writing.
// `plan_normalize_peak`/`plan_normalize_lufs` return a [`Edit`] ready for
// [`Session::commit_edit`] once the caller has reacquired the session (and, per SPEC-010 §2.8,
// checked the session hasn't moved on meanwhile) — or a no-op outcome, exactly like the
// synchronous functions above. Pass 1 covers progress `[0.0, 0.3)`, pass 2 `[0.3, 1.0]`
// (SPEC-010 §2.8).

/// The plan [`plan_normalize_peak`] hands back: either a no-op (nothing to commit) or a
/// ready-to-commit [`Edit`], not yet applied to any session.
#[derive(Debug)]
pub enum NormalizePeakPlan {
    NoOp(NormalizeOutcome),
    Gain(Edit),
}

/// Pass 1 ([`scan_peak_accelerated`]) + decide + pass 2 ([`write_gained`]) for peak normalize,
/// against an `Arc<ChunkStore>`/snapshot instead of a `&mut Session` (H-09's job path). `cancel`
/// stops either pass with [`ProjectError::Cancelled`] within one buffer/chunk (≤ 65 536 samples,
/// SPEC-010 §2.8's ≤ 100 ms). `on_progress` receives a fraction in `[0.0, 1.0]`, monotonically
/// non-decreasing.
pub fn plan_normalize_peak(
    store: &Arc<ChunkStore>,
    snapshot: &DocSnapshot,
    sample_rate_hz: u32,
    range: Range,
    target_db: f64,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(f32),
) -> Result<NormalizePeakPlan> {
    let total = range.len_samples().max(1) as f32;
    on_progress(0.0);
    let peak = scan_peak_accelerated(store, snapshot, range, cancel, |done| {
        on_progress(0.3 * (done as f32 / total));
    })?;
    if cancel.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    match decide(peak, target_db) {
        Decision::NoOp(outcome) => Ok(NormalizePeakPlan::NoOp(outcome)),
        Decision::Gain(gain) => {
            let writer = ChunkWriter::with_cancel(Arc::clone(store), cancel.clone());
            let new_pieces = write_gained(
                store,
                snapshot,
                sample_rate_hz,
                range,
                gain,
                writer,
                |done| on_progress(0.3 + 0.7 * (done as f32 / total)),
            )?;
            let edit = Edit::new(LABEL_NORMALIZE).replace_with(
                range.start,
                range.len_samples(),
                new_pieces,
                MarkerMapping::Identity,
            );
            on_progress(1.0);
            Ok(NormalizePeakPlan::Gain(edit))
        }
    }
}

/// The plan [`plan_normalize_lufs`] hands back (mirrors [`NormalizePeakPlan`]).
#[derive(Debug)]
pub enum NormalizeLufsPlan {
    NoOp(LufsNormalizeOutcome),
    Gain {
        edit: Edit,
        gain_db: f64,
        predicted_true_peak_dbtp: f64,
        exceeds_true_peak_ceiling: bool,
    },
}

/// Pass 1 ([`scan_loudness`], no pyramid acceleration — BS.1770 gating needs every sample) +
/// decide + pass 2 ([`write_gained`]) for LUFS normalize, against an `Arc<ChunkStore>`/snapshot
/// (H-09's job path; mirrors [`plan_normalize_peak`]).
pub fn plan_normalize_lufs(
    store: &Arc<ChunkStore>,
    snapshot: &DocSnapshot,
    sample_rate_hz: u32,
    range: Range,
    target_lufs: f64,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(f32),
) -> Result<NormalizeLufsPlan> {
    let total = range.len_samples().max(1) as f32;
    on_progress(0.0);
    let report = scan_loudness(store, snapshot, range, sample_rate_hz, cancel, |done| {
        on_progress(0.3 * (done as f32 / total));
    })?;
    if cancel.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    if report.integrated_lufs <= MIN_INTEGRATED_LUFS {
        return Ok(NormalizeLufsPlan::NoOp(LufsNormalizeOutcome::Silent));
    }
    let gain_db = target_lufs - report.integrated_lufs;
    if gain_db.abs() < ALREADY_TOL_DB {
        return Ok(NormalizeLufsPlan::NoOp(
            LufsNormalizeOutcome::AlreadyNormalized,
        ));
    }
    let gain = 10f64.powf(gain_db / 20.0);
    let writer = ChunkWriter::with_cancel(Arc::clone(store), cancel.clone());
    let new_pieces = write_gained(
        store,
        snapshot,
        sample_rate_hz,
        range,
        gain,
        writer,
        |done| on_progress(0.3 + 0.7 * (done as f32 / total)),
    )?;
    let edit = Edit::new(LABEL_NORMALIZE_LUFS).replace_with(
        range.start,
        range.len_samples(),
        new_pieces,
        MarkerMapping::Identity,
    );
    on_progress(1.0);
    let predicted_true_peak_dbtp = report.true_peak_dbtp + gain_db;
    Ok(NormalizeLufsPlan::Gain {
        edit,
        gain_db,
        predicted_true_peak_dbtp,
        exceeds_true_peak_ceiling: predicted_true_peak_dbtp > TRUE_PEAK_WARN_CEILING_DBTP,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::validate_range;
    use crate::session::SessionConfig;
    use crate::store::StoreOptions;

    fn dbfs(linear: f64) -> f64 {
        20.0 * linear.log10()
    }

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-normalize-{tag}-{}-{}",
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

    fn session_with(tag: &str, samples: &[f32]) -> (Session, std::path::PathBuf) {
        let dir = tmp_dir(tag);
        let mut config = SessionConfig::new(48_000);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        let mut writer = session.chunk_writer();
        writer.append(samples).unwrap();
        let audio = writer.finish().unwrap();
        session.set_floor(&audio, Vec::new()).unwrap();
        (session, dir)
    }

    fn read_all(session: &Session) -> Vec<f32> {
        let snapshot = session.current();
        let mut out = vec![0.0f32; snapshot.len_samples as usize];
        session.store().read(&snapshot, 0, &mut out).unwrap();
        out
    }

    #[test]
    fn target_linear_matches_the_spec_010_exactness_notes() {
        // SPEC-010 §4 "exactness notes": f32(T) for the favorites.
        assert!((target_linear(-1.0) as f32 - 0.891_250_94).abs() < 1e-6);
        assert!((target_linear(-0.1) as f32 - 0.988_553_1).abs() < 1e-6);
        assert!((target_linear(-3.0) as f32 - 0.707_945_78).abs() < 1e-6);
    }

    /// AC-2's % targets: "50.0 %, 100.0 % and 0.1 % give −6.02 ± 0.01, 0.00 ± 0.01 and
    /// −60.00 ± 0.01 dBFS".
    #[test]
    fn target_pct_to_db_matches_the_spec_010_ac2_examples() {
        assert!((target_pct_to_db(50.0) - (-6.02)).abs() < 0.01);
        assert!((target_pct_to_db(100.0) - 0.0).abs() < 0.01);
        assert!((target_pct_to_db(0.1) - (-60.0)).abs() < 0.01);
    }

    /// SPEC-010 §2.4: "switching units converts the shown value (−1.00 dB ↔ 89.1 %)" — and the
    /// two conversions are inverses of each other, so round-tripping a value through both loses
    /// nothing beyond float precision.
    #[test]
    fn target_pct_and_db_conversions_round_trip() {
        // 89.1% is the value rounded to the dialog's 0.1% step (SPEC-010 §2.4); the exact
        // conversion is 100 * 10^(-1/20) ≈ 89.125.
        assert!((target_db_to_pct(-1.0) - 89.1).abs() < 0.03);
        for db in [-60.0, -23.5, -6.02, -1.0, -0.1, 0.0] {
            let pct = target_db_to_pct(db);
            assert!((target_pct_to_db(pct) - db).abs() < 1e-9);
        }
    }

    #[test]
    fn scan_peak_finds_the_maximum_absolute_sample_and_ignores_silence() {
        let samples = vec![0.1, -0.5, 0.3, -0.9, 0.2];
        let (session, dir) = session_with("scan", &samples);
        let store = session.store();
        let snapshot = session.current();
        let range = validate_range(0, 5, 5).unwrap();
        let peak = scan_peak(store, &snapshot, range).unwrap();
        assert!((peak - 0.9).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_peak_rejects_non_finite_samples() {
        let samples = vec![0.1, f32::NAN, 0.3];
        let (session, dir) = session_with("nan", &samples);
        let range = validate_range(0, 3, 3).unwrap();
        let err = scan_peak(session.store(), &session.current(), range).unwrap_err();
        assert!(matches!(err, ProjectError::NonFiniteSample));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decide_covers_the_spec_010_2_7_table() {
        // Digital silence.
        assert!(matches!(
            decide(0.0, -1.0),
            Decision::NoOp(NormalizeOutcome::Silent)
        ));
        // Near-silence, below the -120 dBFS floor.
        assert!(matches!(
            decide(10f64.powf(-130.0 / 20.0), -1.0),
            Decision::NoOp(NormalizeOutcome::Silent)
        ));
        // -100 dBFS *is* normalized (between -120 and -60, SPEC-010 §2.7).
        assert!(matches!(
            decide(10f64.powf(-100.0 / 20.0), -1.0),
            Decision::Gain(_)
        ));
        // Already at the target within the tolerance.
        assert!(matches!(
            decide(target_linear(-1.0), -1.0),
            Decision::NoOp(NormalizeOutcome::AlreadyNormalized)
        ));
        // A gain clearly above the tolerance.
        match decide(target_linear(-6.0), -1.0) {
            Decision::Gain(g) => assert!((dbfs(g) - 5.0).abs() < 0.01),
            other => panic!("expected a gain, got {other:?}"),
        }
    }

    impl std::fmt::Debug for Decision {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Decision::NoOp(o) => write!(f, "NoOp({o:?})"),
                Decision::Gain(g) => write!(f, "Gain({g})"),
            }
        }
    }

    #[test]
    fn normalize_peak_hits_the_three_favorites_within_one_hundredth_db() {
        let samples = vox_testkit_like_sine();
        for target_db in [-1.0, -0.1, -3.0] {
            let (mut session, dir) = session_with("favorite", &samples);
            let len = session.current().len_samples;
            let range = validate_range(0, len, len).unwrap();
            let result = normalize_peak(&mut session, range, target_db).unwrap();
            let NormalizeResult::Applied(_) = result else {
                panic!("expected the favorite to apply a gain");
            };
            let out = read_all(&session);
            let peak = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            assert!(
                (dbfs(f64::from(peak)) - target_db).abs() < 0.01,
                "target {target_db}, got {} dBFS",
                dbfs(f64::from(peak))
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A simple sine at -12 dBFS peak (no `vox-testkit` dependency in this crate's unit tests —
    /// the integration test in `crates/project/tests` uses testkit's own generators).
    fn vox_testkit_like_sine() -> Vec<f32> {
        let amp = 10f32.powf(-12.0 / 20.0);
        (0..48_000)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn normalize_peak_is_a_no_op_on_silence_and_leaves_no_undo_entry() {
        let samples = vec![0.0f32; 1_000];
        let (mut session, dir) = session_with("silent", &samples);
        let before_rev = session.current().audio_rev;
        let range = validate_range(0, 1_000, 1_000).unwrap();
        let result = normalize_peak(&mut session, range, -1.0).unwrap();
        assert!(matches!(
            result,
            NormalizeResult::NoOp(NormalizeOutcome::Silent)
        ));
        assert_eq!(session.current().audio_rev, before_rev);
        assert_eq!(session.history().undo_depth(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_peak_is_a_no_op_when_already_at_the_target() {
        let amp = target_linear(-1.0) as f32;
        let samples = vec![amp, -amp, amp * 0.5];
        let (mut session, dir) = session_with("already", &samples);
        let before_rev = session.current().audio_rev;
        let range = validate_range(0, 3, 3).unwrap();
        let result = normalize_peak(&mut session, range, -1.0).unwrap();
        assert!(matches!(
            result,
            NormalizeResult::NoOp(NormalizeOutcome::AlreadyNormalized)
        ));
        assert_eq!(session.current().audio_rev, before_rev);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_peak_refuses_non_finite_input_and_changes_nothing() {
        let samples = vec![0.1, f32::NAN, 0.3];
        let (mut session, dir) = session_with("nan-apply", &samples);
        let before = session.current().clone();
        let range = validate_range(0, 3, 3).unwrap();
        let err = normalize_peak(&mut session, range, -1.0).unwrap_err();
        assert!(matches!(err, ProjectError::NonFiniteSample));
        assert_eq!(session.current().audio_rev, before.audio_rev);
        assert_eq!(session.history().undo_depth(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_peak_selection_only_leaves_the_rest_bit_identical() {
        let mut samples = vox_testkit_like_sine();
        samples.truncate(3_000);
        let (mut session, dir) = session_with("selection", &samples);
        let before = read_all(&session);
        let range = validate_range(1_000, 2_000, 3_000).unwrap();
        let result = normalize_peak(&mut session, range, -1.0).unwrap();
        let NormalizeResult::Applied(_) = result else {
            panic!("expected a gain over this selection");
        };
        let after = read_all(&session);
        assert_eq!(after.len(), before.len());
        assert_eq!(&after[..1_000], &before[..1_000], "before the selection");
        assert_eq!(&after[2_000..], &before[2_000..], "after the selection");
        let peak = after[1_000..2_000]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!((dbfs(f64::from(peak)) - (-1.0)).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_peak_preserves_silence_pieces_and_relative_levels() {
        // 1 s of a -12 dBFS sine, then 1 s of `Silence` pieces (SPEC-010 AC-12) — built straight
        // into the undo floor, since `Identity` mapping (used by real `Silence`/Normalize edits)
        // is length-preserving and can't express an insert.
        let samples = vox_testkit_like_sine();
        let dir = tmp_dir("silence-run");
        let mut config = SessionConfig::new(48_000);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        let mut writer = session.chunk_writer();
        writer.append(&samples).unwrap();
        let mut audio = writer.finish().unwrap();
        audio.pieces.extend(Piece::silence_run(48_000));
        audio.len_samples += 48_000;
        session.set_floor(&audio, Vec::new()).unwrap();

        let len = session.current().len_samples;
        assert_eq!(len, samples.len() as u64 + 48_000);
        let range = validate_range(0, len, len).unwrap();
        normalize_peak(&mut session, range, -1.0).unwrap();

        let snapshot = session.current();
        let silence_piece_still_silence = snapshot
            .pieces
            .iter()
            .any(|p| matches!(p.source, Source::Silence) && p.len_samples() >= 40_000);
        assert!(
            silence_piece_still_silence,
            "the inserted Silence run is still a Silence piece: {:?}",
            &*snapshot.pieces
        );
        let out = read_all(&session);
        assert!(
            out[48_000..].iter().all(|&s| s == 0.0),
            "the silent second reads back as +0.0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- S4-01: LUFS normalize ----------------------------------------------------------------

    #[test]
    fn normalize_lufs_hits_the_three_favorites_within_a_tenth_lu() {
        for target_lufs in [-16.0, -19.0, -23.0] {
            let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
            let (mut session, dir) = session_with("lufs-favorite", &samples);
            let len = session.current().len_samples;
            let range = validate_range(0, len, len).unwrap();
            let result = normalize_lufs(&mut session, range, target_lufs).unwrap();
            let LufsNormalizeResult::Applied { .. } = result else {
                panic!("expected the favorite to apply a gain");
            };
            let snapshot = session.current();
            let report = scan_loudness(
                session.store(),
                &snapshot,
                range,
                48_000,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
            assert!(
                (report.integrated_lufs - target_lufs).abs() < 0.1,
                "target {target_lufs}, got {} LUFS",
                report.integrated_lufs
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn normalize_lufs_is_a_no_op_on_silence_and_leaves_no_undo_entry() {
        let samples = vec![0.0f32; 48_000];
        let (mut session, dir) = session_with("lufs-silent", &samples);
        let before_rev = session.current().audio_rev;
        let range = validate_range(0, 48_000, 48_000).unwrap();
        let result = normalize_lufs(&mut session, range, -19.0).unwrap();
        assert!(matches!(
            result,
            LufsNormalizeResult::NoOp(LufsNormalizeOutcome::Silent)
        ));
        assert_eq!(session.current().audio_rev, before_rev);
        assert_eq!(session.history().undo_depth(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_lufs_is_a_no_op_when_already_at_the_target() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let (mut session, dir) = session_with("lufs-already", &samples);
        let len = session.current().len_samples;
        let range = validate_range(0, len, len).unwrap();
        let first = normalize_lufs(&mut session, range, -19.0).unwrap();
        assert!(
            matches!(first, LufsNormalizeResult::Applied { .. }),
            "the first normalize still applies (exactness)"
        );
        let second = normalize_lufs(&mut session, range, -19.0).unwrap();
        assert!(matches!(
            second,
            LufsNormalizeResult::NoOp(LufsNormalizeOutcome::AlreadyNormalized)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_lufs_refuses_non_finite_input_and_changes_nothing() {
        let samples = vec![0.1, f32::NAN, 0.3];
        let (mut session, dir) = session_with("lufs-nan", &samples);
        let before = session.current().clone();
        let range = validate_range(0, 3, 3).unwrap();
        let err = normalize_lufs(&mut session, range, -19.0).unwrap_err();
        assert!(matches!(err, ProjectError::NonFiniteSample));
        assert_eq!(session.current().audio_rev, before.audio_rev);
        assert_eq!(session.history().undo_depth(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_lufs_selection_only_leaves_the_rest_bit_identical() {
        let sine = vox_testkit::signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let mut samples = vec![0.02f32; 48_000];
        samples.extend(sine);
        samples.extend(vec![0.02f32; 48_000]);
        let (mut session, dir) = session_with("lufs-selection", &samples);
        let before = read_all(&session);
        let range = validate_range(48_000, 96_000, samples.len() as u64).unwrap();
        let result = normalize_lufs(&mut session, range, -19.0).unwrap();
        let LufsNormalizeResult::Applied { .. } = result else {
            panic!("expected a gain over this selection");
        };
        let after = read_all(&session);
        assert_eq!(after.len(), before.len());
        assert_eq!(&after[..48_000], &before[..48_000], "before the selection");
        assert_eq!(&after[96_000..], &before[96_000..], "after the selection");
        let snapshot = session.current();
        let report = scan_loudness(
            session.store(),
            &snapshot,
            range,
            48_000,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert!((report.integrated_lufs - (-19.0)).abs() < 0.1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ticket AC: "if the resulting true peak would exceed −1 dBTP, apply the gain anyway and
    /// show a notice" — a sine already peaking near 0 dBFS, normalized to a much louder target,
    /// still applies (no silent limiting) and flags the ceiling exceedance.
    #[test]
    fn normalize_lufs_flags_a_predicted_true_peak_ceiling_exceedance_but_still_applies() {
        let samples = vox_testkit::signal::sine(1000.0, -0.5, 2.0, 48_000).unwrap();
        let (mut session, dir) = session_with("lufs-tp", &samples);
        let len = session.current().len_samples;
        let range = validate_range(0, len, len).unwrap();
        let result = normalize_lufs(&mut session, range, 0.0).unwrap();
        let LufsNormalizeResult::Applied {
            exceeds_true_peak_ceiling,
            gain_db,
            ..
        } = result
        else {
            panic!("expected a gain");
        };
        assert!(
            gain_db > 0.0,
            "0 LUFS is louder than this sine's integrated loudness"
        );
        assert!(
            exceeds_true_peak_ceiling,
            "a sine near 0 dBFS normalized up to 0 LUFS should exceed -1 dBTP"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_lufs_preserves_silence_pieces() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let dir = tmp_dir("lufs-silence-run");
        let mut config = SessionConfig::new(48_000);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        let mut writer = session.chunk_writer();
        writer.append(&samples).unwrap();
        let mut audio = writer.finish().unwrap();
        audio.pieces.extend(Piece::silence_run(48_000));
        audio.len_samples += 48_000;
        session.set_floor(&audio, Vec::new()).unwrap();

        let len = session.current().len_samples;
        let range = validate_range(0, len, len).unwrap();
        normalize_lufs(&mut session, range, -19.0).unwrap();

        let snapshot = session.current();
        let silence_piece_still_silence = snapshot
            .pieces
            .iter()
            .any(|p| matches!(p.source, Source::Silence) && p.len_samples() >= 40_000);
        assert!(
            silence_piece_still_silence,
            "the inserted Silence run is still a Silence piece: {:?}",
            &*snapshot.pieces
        );
        let out = read_all(&session);
        assert!(
            out[samples.len()..].iter().all(|&s| s == 0.0),
            "the silent tail reads back as +0.0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-09: pyramid-accelerated scan (AC-15), job plans (progress, cancel) -----------------

    /// A pool of `n` freshly-committed, full chunks with varied random content, for building
    /// randomized piece tables against without paying for a fresh `ChunkStore` per trial.
    fn chunk_pool(
        store: &Arc<ChunkStore>,
        rng: &mut vox_testkit::prng::Pcg32,
        n: usize,
    ) -> Vec<u32> {
        let mut writer = store.writer();
        for _ in 0..n {
            let amp = 0.05 + (rng.next_u32() % 95) as f32 / 100.0;
            let chunk: Vec<f32> = (0..CHUNK_SAMPLES)
                .map(|_| amp * (rng.next_signed() as f32))
                .collect();
            writer.append(&chunk).unwrap();
        }
        let written = writer.finish().unwrap();
        assert_eq!(
            written.chunks.len(),
            n,
            "each full append commits one chunk"
        );
        written.chunks
    }

    /// AC-15: the pyramid-accelerated peak scan must equal the brute-force scan, bit-exact, over
    /// 1 000 seeded piece tables mixing whole-chunk pieces (the accelerated path), partial-chunk
    /// pieces at random offsets/lengths (the raw fallback, including chunk edges) and `Silence`
    /// runs.
    #[test]
    #[allow(clippy::float_cmp)] // AC-15: bit-exact equality is the point of this test
    fn scan_peak_accelerated_matches_scan_peak_bit_exact_on_seeded_piece_tables() {
        let dir = tmp_dir("accel-bitexact");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut rng = vox_testkit::prng::Pcg32::new(101, 7);
        let pool = chunk_pool(&store, &mut rng, 6);

        for trial in 0..1000 {
            let mut pieces = Vec::new();
            let segments = 1 + rng.next_u32() % 6;
            for _ in 0..segments {
                match rng.next_u32() % 3 {
                    0 => {
                        let len = 1 + rng.next_u32() % 4_000;
                        pieces.push(Piece::silence(len));
                    }
                    1 => {
                        let id = pool[(rng.next_u32() as usize) % pool.len()];
                        pieces.push(Piece::chunk(id, 0, CHUNK_SAMPLES as u32));
                    }
                    _ => {
                        let id = pool[(rng.next_u32() as usize) % pool.len()];
                        let offset = rng.next_u32() % (CHUNK_SAMPLES as u32 - 1);
                        let max_len = CHUNK_SAMPLES as u32 - offset;
                        let len = 1 + rng.next_u32() % max_len;
                        pieces.push(Piece::chunk(id, offset, len));
                    }
                }
            }
            let snapshot = DocSnapshot::new(48_000, pieces, Vec::new());
            let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();
            let brute = scan_peak(&store, &snapshot, range).unwrap();
            let accelerated =
                scan_peak_accelerated(&store, &snapshot, range, &CancelToken::new(), |_| {})
                    .unwrap();
            assert_eq!(
                brute, accelerated,
                "trial {trial}: pyramid-accelerated peak must be bit-exact with brute force"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A whole-chunk piece whose chunk contains a NaN is caught via the pyramid's
    /// `has_non_finite` flag, without a raw read — same error as the brute-force scan.
    #[test]
    fn scan_peak_accelerated_detects_non_finite_in_a_whole_chunk() {
        let dir = tmp_dir("accel-nan-whole");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut samples = vec![0.1f32; CHUNK_SAMPLES];
        samples[1234] = f32::NAN;
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let written = writer.finish().unwrap();
        let id = written.chunks[0];
        let snapshot = DocSnapshot::new(
            48_000,
            vec![Piece::chunk(id, 0, CHUNK_SAMPLES as u32)],
            Vec::new(),
        );
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();
        let err = scan_peak_accelerated(&store, &snapshot, range, &CancelToken::new(), |_| {})
            .unwrap_err();
        assert!(matches!(err, ProjectError::NonFiniteSample));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A *partial* chunk piece whose chunk contains a NaN outside the referenced sub-range is
    /// fine (the raw fallback only reads the piece's own samples); one inside the sub-range is
    /// still caught, exactly like [`scan_peak`].
    #[test]
    fn scan_peak_accelerated_detects_non_finite_in_a_partial_chunk_via_raw_fallback() {
        let dir = tmp_dir("accel-nan-partial");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut samples = vec![0.1f32; CHUNK_SAMPLES];
        samples[100] = f32::NAN; // outside [1000, 2000)
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let written = writer.finish().unwrap();
        let id = written.chunks[0];

        let clean_range_snapshot =
            DocSnapshot::new(48_000, vec![Piece::chunk(id, 1_000, 1_000)], Vec::new());
        let range = validate_range(0, 1_000, 1_000).unwrap();
        let peak = scan_peak_accelerated(
            &store,
            &clean_range_snapshot,
            range,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert!((peak - 0.1).abs() < 1e-6);

        let poisoned_snapshot =
            DocSnapshot::new(48_000, vec![Piece::chunk(id, 50, 1_000)], Vec::new());
        let err = scan_peak_accelerated(
            &store,
            &poisoned_snapshot,
            range,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::NonFiniteSample));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_peak_accelerated_is_cancellable() {
        let samples = vox_testkit_like_sine();
        let (session, dir) = session_with("accel-cancel-session", &samples);
        let range = validate_range(
            0,
            session.current().len_samples,
            session.current().len_samples,
        )
        .unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err =
            scan_peak_accelerated(session.store(), &session.current(), range, &cancel, |_| {})
                .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `plan_normalize_peak` (H-09's job path) must agree with the synchronous `normalize_peak`
    /// on the gain applied, and report progress reaching exactly 1.0.
    #[test]
    fn plan_normalize_peak_matches_normalize_peak_and_reports_full_progress() {
        let samples = vox_testkit_like_sine();
        let (mut session, dir) = session_with("plan-peak", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();

        let mut last_progress = 0.0f32;
        let mut monotonic = true;
        let plan = plan_normalize_peak(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -1.0,
            &CancelToken::new(),
            |fraction| {
                if fraction < last_progress {
                    monotonic = false;
                }
                last_progress = fraction;
            },
        )
        .unwrap();
        assert!(monotonic, "progress must be monotonically non-decreasing");
        assert!((last_progress - 1.0).abs() < 1e-6);
        let NormalizePeakPlan::Gain(edit) = plan else {
            panic!("expected a gain plan");
        };
        let step = session.commit_edit(edit).unwrap();
        let mut out = vec![0.0f32; step.snapshot.len_samples as usize];
        session.store().read(&step.snapshot, 0, &mut out).unwrap();
        let peak = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!((dbfs(f64::from(peak)) - (-1.0)).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_normalize_peak_no_op_on_silence() {
        let samples = vec![0.0f32; 1_000];
        let (session, dir) = session_with("plan-peak-silent", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, 1_000, 1_000).unwrap();
        let plan = plan_normalize_peak(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -1.0,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(matches!(
            plan,
            NormalizePeakPlan::NoOp(NormalizeOutcome::Silent)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cancelling before the scan starts stops pass 1 immediately, before any write — the caller
    /// (the job service) never sees a plan to commit.
    #[test]
    fn plan_normalize_peak_cancel_before_scan_returns_cancelled() {
        let samples = vox_testkit_like_sine();
        let (session, dir) = session_with("plan-peak-cancel-scan", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = plan_normalize_peak(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -1.0,
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cancelling partway through pass 2 (the write) stops the job without ever producing a
    /// gain plan (SPEC-010 §2.8: chunks already written become unreachable store data, reclaimed
    /// by compaction — nothing here commits them to any document).
    #[test]
    fn plan_normalize_peak_cancel_mid_write_returns_cancelled() {
        let samples = vox_testkit_like_sine();
        let (session, dir) = session_with("plan-peak-cancel-write", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();
        let cancel = CancelToken::new();
        let mut calls = 0u32;
        let err = plan_normalize_peak(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -1.0,
            &cancel,
            |fraction| {
                calls += 1;
                // Cancel partway through pass 2 (fraction > 0.3), like a user clicking Cancel
                // mid-write.
                if fraction > 0.5 {
                    cancel.cancel();
                }
            },
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        assert!(calls > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `plan_normalize_lufs` mirrors the peak-plan tests above.
    #[test]
    fn plan_normalize_lufs_matches_normalize_lufs_and_reports_full_progress() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let (mut session, dir) = session_with("plan-lufs", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();

        let mut last_progress = 0.0f32;
        let plan = plan_normalize_lufs(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -19.0,
            &CancelToken::new(),
            |fraction| last_progress = fraction,
        )
        .unwrap();
        assert!((last_progress - 1.0).abs() < 1e-6);
        let NormalizeLufsPlan::Gain { edit, .. } = plan else {
            panic!("expected a gain plan");
        };
        session.commit_edit(edit).unwrap();
        let after = session.current();
        let report = scan_loudness(
            session.store(),
            &after,
            range,
            48_000,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert!((report.integrated_lufs - (-19.0)).abs() < 0.1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_normalize_lufs_cancel_mid_scan_returns_cancelled() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let (session, dir) = session_with("plan-lufs-cancel", &samples);
        let store = Arc::clone(session.store());
        let snapshot = session.current();
        let range = validate_range(0, snapshot.len_samples, snapshot.len_samples).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = plan_normalize_lufs(
            &store,
            &snapshot,
            session.sample_rate_hz(),
            range,
            -19.0,
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
