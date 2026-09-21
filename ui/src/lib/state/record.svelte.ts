import { listen } from "@tauri-apps/api/event";
import { documentState, withUnsavedChangesGuard } from "../document/document.svelte";
import type {
  CalibrationResultDto,
  DefaultFormatDto,
  EventName,
  IpcError,
  JobProgressDto,
  MonitorMode,
  RecordFinishedDto,
  RecordOffsetDto,
  RecordPhaseDto,
  RecordPrefsDto,
  RecordStartedDto,
  RecordStateDto,
} from "../ipc/bindings";
import {
  calibrationCancel,
  calibrationRun,
  recordArm,
  recordGet,
  recordOffsetGet,
  recordOffsetSet,
  recordSetMonitor,
  recordStart,
  recordStartAt,
  recordStop,
} from "../ipc/record_commands";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { registerAction } from "../shortcuts";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { nowMs, PeakBallistics, READOUT_SMOOTHING_TAU_MS, SmoothedDb, ThrottledReadout } from "../meters/ballistics";
import { DISK_WARN_MINUTES } from "../record/format";
import { DEFAULT_RECORD_PREFS, parseOffsetText } from "../record/punch";
import { pushNotice } from "./notices.svelte";
import { selectionState, setSelectionFromResult, setSelectionLocked } from "./selection.svelte";
import { saveSettings, settingsState } from "./settings.svelte";
import { addTelemetryListener } from "./transport.svelte";

// Factory default (SPEC-002 §3) — used only if settings haven't loaded yet (mirrors
// `DocumentMenu.svelte`'s `FALLBACK_FORMAT` for File → New Recording…).
const FALLBACK_FORMAT: DefaultFormatDto = { sample_rate_hz: 48_000, bit_depth: "24" };

/**
 * Record panel store (S1-04, SPEC-002 §2.1–§2.2, §2.7): the engine's record state
 * (`record_state` event + command results), the input meter with UI ballistics, the clip latch
 * and the elapsed take time (from `VXTM` telemetry). Registers the Record action (Shift+R).
 *
 * H-06: also owns the New Recording dialog's prompt state (`NewRecordingDialog.svelte`, opened
 * from `DocumentMenu`'s "New Recording…") — sample rate / bit depth, prefilled from the current
 * default format, become the new document's format and the engine's default format going forward.
 *
 * T-304 (SPEC-022): Record on a document with audio resolves per §2.2 (`record_start_at`: a
 * punch-in over a non-empty selection, else Insert/Overwrite at the selection start or cursor).
 * The store follows the operation's phases (`record_phase`) for the countdown, applies the
 * selection/playhead of `record_finished`, locks selection gestures while recording, and owns the
 * Punch & pre-roll preferences, the recording-offset readout and the calibration dialog.
 */

const IDLE: RecordStateDto = {
  input_device: null,
  input_channel: 1,
  input_status: "not_selected",
  armed: false,
  input_open: false,
  input_rate_hz: null,
  recording: false,
  finishing: false,
  monitor: "off",
  monitoring: false,
  monitor_latency_us: null,
  monitor_dropouts: 0,
  dropout_count: 0,
  disk_remaining_s: null,
};

export interface InputMeterView {
  /** Peak bar (with ballistics). */
  peakDbfs: number;
  /** Peak-hold tick. */
  holdDbfs: number;
  /** 300 ms RMS. */
  rmsDbfs: number;
  /** H-112: the hold value, throttled to ~4-5 Hz for a legible numeric readout (matches the
   * output meter's `peakReadoutDbfs`, `transport.svelte.ts`). */
  peakReadoutDbfs: number;
  /** H-112: the RMS value, smoothed and throttled to ~4-5 Hz for a legible numeric readout
   * (matches the output meter's `rmsReadoutDbfs`). */
  rmsReadoutDbfs: number;
  /** Highest peak since arming or the last reset (numeric readout). */
  maxDbfs: number;
}

/** T-304 (SPEC-022 §2.14): the calibration dialog's state. */
export interface CalibrationView {
  stage: "connect" | "measuring" | "result" | "failed";
  jobId: number | null;
  progress: number;
  verify: boolean;
  result: CalibrationResultDto | null;
  /** The last accepted measurement was applied (Verify becomes available). */
  applied: boolean;
}

const SILENT: InputMeterView = {
  peakDbfs: Number.NEGATIVE_INFINITY,
  holdDbfs: Number.NEGATIVE_INFINITY,
  rmsDbfs: Number.NEGATIVE_INFINITY,
  peakReadoutDbfs: Number.NEGATIVE_INFINITY,
  rmsReadoutDbfs: Number.NEGATIVE_INFINITY,
  maxDbfs: Number.NEGATIVE_INFINITY,
};

let state = $state<RecordStateDto>({ ...IDLE });
let meter = $state<InputMeterView>({ ...SILENT });
let clipLatched = $state(false);
let elapsedSamples = $state(0);
/** A Record press is being handled (prompt open or start in flight). */
let starting = false;
/** The New Recording dialog's prompt (H-06); `null` when closed. */
let newRecordingPrompt = $state<DefaultFormatDto | null>(null);
/** H-11 (SPEC-002 §2.5): the "Only N min of disk space left. Record anyway?" prompt, `null` when
 * closed. `minutes` is floored for display. */
let lowDiskPrompt = $state<{ minutes: number; resolve: (ok: boolean) => void } | null>(null);
/** T-304: the running record operation (`null`: none, or a plain new recording). */
let op = $state<RecordStartedDto | null>(null);
/** T-304: its latest phase (`record_phase`). */
let phase = $state<RecordPhaseDto | null>(null);
/** T-304: the heard document position during an operation (telemetry playhead). */
let heardSamples = $state(0);
/** T-304: the current device setup's recording offset (SPEC-022 §2.13). */
let offset = $state<RecordOffsetDto | null>(null);
/** T-304: the calibration dialog (`null`: closed). */
let calibration = $state<CalibrationView | null>(null);
const ballistics = new PeakBallistics();
// H-112: same readout-pacing classes the output meter uses (`transport.svelte.ts`) — reused, not
// reimplemented (`ui/src/lib/meters/ballistics.ts`).
const rmsSmoothed = new SmoothedDb(Number.NEGATIVE_INFINITY, READOUT_SMOOTHING_TAU_MS);
const peakReadout = new ThrottledReadout(Number.NEGATIVE_INFINITY);
const rmsReadout = new ThrottledReadout(Number.NEGATIVE_INFINITY);

/** Read-only accessor for components. */
export function recordState(): {
  readonly state: RecordStateDto;
  readonly meter: InputMeterView;
  readonly clipLatched: boolean;
  readonly elapsedSamples: number;
  readonly newRecordingPrompt: DefaultFormatDto | null;
  readonly lowDiskPrompt: { readonly minutes: number } | null;
  readonly op: RecordStartedDto | null;
  readonly phase: RecordPhaseDto | null;
  readonly heardSamples: number;
  readonly offset: RecordOffsetDto | null;
  readonly calibration: CalibrationView | null;
  readonly prefs: RecordPrefsDto;
} {
  return {
    get state() {
      return state;
    },
    get meter() {
      return meter;
    },
    get clipLatched() {
      return clipLatched;
    },
    get elapsedSamples() {
      return elapsedSamples;
    },
    get newRecordingPrompt() {
      return newRecordingPrompt;
    },
    get lowDiskPrompt() {
      return lowDiskPrompt;
    },
    get op() {
      return op;
    },
    get phase() {
      return phase;
    },
    get heardSamples() {
      return heardSamples;
    },
    get offset() {
      return offset;
    },
    get calibration() {
      return calibration;
    },
    get prefs() {
      return recordPrefs();
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

function applyState(next: RecordStateDto): void {
  if (!next.input_open && state.input_open) {
    // Disarmed: the meter restarts empty on the next arm ("highest peak since arming").
    ballistics.reset();
    rmsSmoothed.reset(Number.NEGATIVE_INFINITY);
    peakReadout.reset(Number.NEGATIVE_INFINITY);
    rmsReadout.reset(Number.NEGATIVE_INFINITY);
    meter = { ...SILENT };
  }
  state = next;
  // T-304 (SPEC-022 §2.11): no selection gestures while a take or an operation runs.
  setSelectionLocked(next.recording || next.finishing || op !== null);
}

async function run(command: () => Promise<RecordStateDto>): Promise<void> {
  try {
    applyState(await command());
  } catch (err) {
    report(err);
  }
}

/** One telemetry frame: input meter, clip latch, elapsed take time. */
export function onInputTelemetry(frame: TelemetryFrame, atMs: number = nowMs()): void {
  if ((frame.flags & VXTM_FLAGS.IN_CLIP) !== 0) {
    clipLatched = true;
  }
  if ((frame.flags & VXTM_FLAGS.RECORDING) !== 0) {
    if (op) {
      // T-304: during an operation the telemetry playhead is the heard document position.
      heardSamples = frame.playheadSample;
      elapsedSamples = Math.max(0, frame.playheadSample - op.at_samples);
    } else {
      elapsedSamples = frame.playheadSample;
    }
  }
  ballistics.update(frame.inPeakDbfs, atMs);
  rmsSmoothed.update(frame.inRmsDbfs, atMs);
  peakReadout.update(ballistics.hold, atMs);
  rmsReadout.update(rmsSmoothed.value, atMs);
  const next: InputMeterView = {
    peakDbfs: ballistics.bar,
    holdDbfs: ballistics.hold,
    rmsDbfs: frame.inRmsDbfs,
    peakReadoutDbfs: peakReadout.value,
    rmsReadoutDbfs: rmsReadout.value,
    maxDbfs: Math.max(meter.maxDbfs, frame.inPeakDbfs),
  };
  // H-43: an unchanged meter (e.g. no input open: every frame reads −∞) is not written again, so
  // nothing that reads it re-renders.
  if (
    next.peakDbfs !== meter.peakDbfs ||
    next.holdDbfs !== meter.holdDbfs ||
    next.rmsDbfs !== meter.rmsDbfs ||
    next.peakReadoutDbfs !== meter.peakReadoutDbfs ||
    next.rmsReadoutDbfs !== meter.rmsReadoutDbfs ||
    next.maxDbfs !== meter.maxDbfs
  ) {
    meter = next;
  }
}

/** The Input (arm) toggle; locked on while recording. */
export function toggleArm(): Promise<void> {
  if (state.recording || state.finishing) {
    return Promise.resolve();
  }
  return run(() => recordArm(!state.armed));
}

export function setMonitor(mode: MonitorMode): Promise<void> {
  return run(() => recordSetMonitor(mode));
}

/** Clicking the clip lamp clears it. */
export function clearClip(): void {
  clipLatched = false;
}

/** Clicking the peak readout resets it. */
export function resetMaxPeak(): void {
  meter = { ...meter, maxDbfs: Number.NEGATIVE_INFINITY };
}

/** Shows the low-disk confirm prompt and resolves once the user answers. */
function askLowDisk(minutes: number): Promise<boolean> {
  return new Promise((resolve) => {
    lowDiskPrompt = { minutes, resolve };
  });
}

/** The `LowDiskDialog` component calls this with the user's choice. */
export function resolveLowDiskPrompt(ok: boolean): void {
  const prompt = lowDiskPrompt;
  lowDiskPrompt = null;
  prompt?.resolve(ok);
}

/**
 * H-11 (SPEC-002 §2.5): below `DISK_WARN_MINUTES` remaining, Record confirms first ("Only N min
 * of disk space left. Record anyway?"). Resolves `true` when it's fine to proceed (plenty of
 * space, the query failed so there is nothing to warn about, or the user confirmed anyway).
 */
async function confirmLowDiskIfNeeded(): Promise<boolean> {
  const remaining = state.disk_remaining_s;
  if (remaining === null || remaining >= DISK_WARN_MINUTES * 60) {
    return true;
  }
  return askLowDisk(Math.floor(remaining / 60));
}

async function startRecording(replace: boolean, format?: DefaultFormatDto): Promise<void> {
  await run(async () => {
    const next = await recordStart(replace, format);
    // H-10 item 2 (SPEC-002 AC-2): the lamp clears when the take actually starts, not merely
    // when it is attempted — a `record_start` that fails (no input device, already recording,
    // …) must not silently drop a real clip warning the input already latched.
    clipLatched = false;
    elapsedSamples = 0;
    return next;
  });
}

/**
 * T-304 (SPEC-022 §2.2): Record on a document with audio — the backend resolves it (punch-in over
 * a non-empty selection, Insert/Overwrite at the selection start or the cursor). Rule 3: with
 * punch-on-selection off, the selection is cleared when recording starts.
 */
async function startAtCursorOrSelection(): Promise<void> {
  const sel = selectionState().current;
  const selection: [number, number] | null =
    sel && sel.endSample > sel.startSample ? [sel.startSample, sel.endSample] : null;
  try {
    const started = await recordStartAt(selection);
    clipLatched = false;
    elapsedSamples = 0;
    heardSamples = started.at_samples - started.preroll_samples;
    op = started.op === "new" ? null : started;
    if (op && op.op !== "punch" && selection) {
      setSelectionFromResult(null);
    }
    applyState(started.state);
  } catch (err) {
    report(err);
  }
}

/**
 * Record button / Shift+R: stops a running recording or operation (Stop semantics per phase,
 * SPEC-022 §2.10); on an empty document starts a new recording at the current default format,
 * no dialog (SPEC-002 §2.2); on a document with audio records at the cursor or punches the
 * selection (SPEC-022 §2.2 — File → New Recording… still replaces the document).
 */
export async function toggleRecord(): Promise<void> {
  if (state.recording) {
    await run(recordStop);
    return;
  }
  if (state.finishing || starting) {
    return;
  }
  starting = true;
  try {
    if (documentState().current.len_samples > 0) {
      if (await confirmLowDiskIfNeeded()) {
        await startAtCursorOrSelection();
      }
    } else if (await confirmLowDiskIfNeeded()) {
      await startRecording(false);
    }
  } finally {
    starting = false;
  }
}

/**
 * File → New Recording…: opens the format prompt (`NewRecordingDialog`), prefilled from the
 * current default format.
 */
export function openNewRecordingPrompt(defaultFormat: DefaultFormatDto): void {
  newRecordingPrompt = defaultFormat;
}

export function cancelNewRecordingPrompt(): void {
  newRecordingPrompt = null;
}

/**
 * Confirms the New Recording dialog: replaces the document (after the unsaved-changes guard, like
 * the Record button) with a fresh one at the chosen format and starts recording into it. Saving
 * the format as the new default (SPEC-002 §2.2 "remembered in settings") is the dialog's job —
 * see `NewRecordingDialog.svelte`, which calls `saveSettings` before this.
 */
export async function confirmNewRecordingPrompt(format: DefaultFormatDto): Promise<void> {
  newRecordingPrompt = null;
  if (state.recording || state.finishing || starting) {
    return;
  }
  starting = true;
  try {
    await withUnsavedChangesGuard(async () => {
      if (await confirmLowDiskIfNeeded()) {
        await startRecording(true, format);
      }
    });
  } finally {
    starting = false;
  }
}

// --- T-304: Punch & pre-roll preferences, recording offset, calibration ---------------------------

/** The Punch & pre-roll preferences (SPEC-022 §2.3), from the settings. */
export function recordPrefs(): RecordPrefsDto {
  return settingsState().current?.record ?? DEFAULT_RECORD_PREFS;
}

/** Saves a change to the Punch & pre-roll preferences (locked while recording). */
export async function setRecordPrefs(patch: Partial<RecordPrefsDto>): Promise<void> {
  if (state.recording || state.finishing) {
    return;
  }
  await saveSettings({ record: { ...recordPrefs(), ...patch } });
}

/** Refreshes the recording-offset readout for the current device setup. */
export async function refreshOffset(): Promise<void> {
  try {
    offset = await recordOffsetGet();
  } catch (err) {
    report(err);
  }
}

/**
 * Manual recording-offset entry (SPEC-022 §2.13): ms or "N smp"; returns `false` (nothing saved)
 * when the text doesn't parse.
 */
export async function setManualOffset(text: string): Promise<boolean> {
  const ms = parseOffsetText(text, offset?.device_rate_hz ?? 0);
  if (ms === null) {
    return false;
  }
  try {
    offset = await recordOffsetSet(ms, "manual", null);
    return true;
  } catch (err) {
    report(err);
    return false;
  }
}

/** Opens the calibration wizard (SPEC-022 §2.14, step 1 "Connect"). */
export function openCalibration(): void {
  calibration = {
    stage: "connect",
    jobId: null,
    progress: 0,
    verify: false,
    result: null,
    applied: false,
  };
  void refreshOffset();
}

/** Starts a measurement (or a Verify pass with the stored offset applied). */
export async function startCalibration(verify = false): Promise<void> {
  if (!calibration) {
    return;
  }
  calibration = { ...calibration, stage: "measuring", progress: 0, verify, result: null, jobId: null };
  try {
    const jobId = await calibrationRun(verify);
    if (calibration && calibration.jobId === null) {
      calibration = { ...calibration, jobId };
    }
  } catch (err) {
    if (calibration) {
      calibration = { ...calibration, stage: "connect" };
    }
    report(err);
  }
}

/** Apply: stores the accepted measurement for this device setup (never a rejected one). */
export async function applyCalibration(): Promise<void> {
  const result = calibration?.result;
  if (!calibration || !result || !result.accepted || result.verify) {
    return;
  }
  try {
    offset = await recordOffsetSet(result.offset_ms, "calibrated", result.confidence);
    calibration = { ...calibration, applied: true };
  } catch (err) {
    report(err);
  }
}

/** Cancel / Close: aborts a running measurement and closes the wizard. */
export async function closeCalibration(): Promise<void> {
  const running = calibration?.stage === "measuring";
  calibration = null;
  if (running) {
    try {
      await calibrationCancel();
    } catch (err) {
      report(err);
    }
  }
}

function onJobProgress(p: JobProgressDto): void {
  if (!calibration || p.kind !== "calibration") {
    return;
  }
  if (calibration.jobId !== null && calibration.jobId !== p.job_id) {
    return;
  }
  switch (p.state) {
    case "running":
      calibration = { ...calibration, jobId: p.job_id, progress: p.fraction };
      break;
    case "failed":
      calibration = { ...calibration, stage: "failed" };
      break;
    case "cancelled":
      calibration = { ...calibration, stage: "connect" };
      break;
    case "done":
      calibration = { ...calibration, progress: 1 };
      break;
  }
}

function onCalibrationResult(r: CalibrationResultDto): void {
  if (!calibration || (calibration.jobId !== null && calibration.jobId !== r.job_id)) {
    return;
  }
  calibration = { ...calibration, jobId: r.job_id, stage: "result", result: r, progress: 1 };
}

function onRecordPhase(p: RecordPhaseDto): void {
  if (op && p.take_id !== op.take_id) {
    return;
  }
  phase = p;
}

function onRecordFinished(f: RecordFinishedDto): void {
  if (op && f.take_id !== op.take_id) {
    return;
  }
  op = null;
  phase = null;
  setSelectionLocked(state.recording || state.finishing);
  if (f.committed && f.result) {
    // SPEC-022 §2.10: a punch keeps [S, E) selected; Insert/Overwrite leave no selection (the
    // backend already moved the playhead).
    setSelectionFromResult(f.result.selection);
  }
}

/** Wires the store (keymap action, `record_state` events, telemetry); returns the teardown. */
export function initRecord(): () => void {
  const cleanups: Array<() => void> = [
    registerAction("record.toggle", () => void toggleRecord()),
    addTelemetryListener((frame) => onInputTelemetry(frame)),
  ];
  let disposed = false;
  const subscribe = <T>(name: EventName, handler: (payload: T) => void): void => {
    listen<T>(name, (e) => handler(e.payload))
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          cleanups.push(unlisten);
        }
      })
      .catch(() => {});
  };
  subscribe<RecordStateDto>("record_state", applyState);
  subscribe<RecordPhaseDto>("record_phase", onRecordPhase);
  subscribe<RecordFinishedDto>("record_finished", onRecordFinished);
  subscribe<JobProgressDto>("job_progress", onJobProgress);
  subscribe<CalibrationResultDto>("calibration_result", onCalibrationResult);
  recordGet()
    .then((next) => {
      if (!disposed) {
        applyState(next);
      }
    })
    .catch(report);
  return () => {
    disposed = true;
    for (const cleanup of cleanups) {
      try {
        cleanup();
      } catch {
        // A failed unlisten during teardown is harmless.
      }
    }
  };
}

/** Test/teardown helper. */
export function resetRecordForTest(): void {
  state = { ...IDLE };
  meter = { ...SILENT };
  clipLatched = false;
  elapsedSamples = 0;
  starting = false;
  newRecordingPrompt = null;
  lowDiskPrompt = null;
  op = null;
  phase = null;
  heardSamples = 0;
  offset = null;
  calibration = null;
  ballistics.reset();
  rmsSmoothed.reset(Number.NEGATIVE_INFINITY);
  peakReadout.reset(Number.NEGATIVE_INFINITY);
  rmsReadout.reset(Number.NEGATIVE_INFINITY);
  setSelectionLocked(false);
}

/** Test helper: sets the record state directly, without going through the engine/events. */
export function applyRecordStateForTest(patch: Partial<RecordStateDto>): void {
  state = { ...state, ...patch };
}
