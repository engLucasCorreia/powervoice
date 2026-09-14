import { listen } from "@tauri-apps/api/event";
import { documentState, withUnsavedChangesGuard } from "../document/document.svelte";
import type { DefaultFormatDto, EventName, IpcError, MonitorMode, RecordStateDto } from "../ipc/bindings";
import { recordArm, recordGet, recordSetMonitor, recordStart, recordStop } from "../ipc/record_commands";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { PeakBallistics } from "../record/ballistics";
import { DISK_WARN_MINUTES } from "../record/format";
import { pushNotice } from "./notices.svelte";
import { settingsState } from "./settings.svelte";
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
  /** Highest peak since arming or the last reset (numeric readout). */
  maxDbfs: number;
}

const SILENT: InputMeterView = {
  peakDbfs: Number.NEGATIVE_INFINITY,
  holdDbfs: Number.NEGATIVE_INFINITY,
  rmsDbfs: Number.NEGATIVE_INFINITY,
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
const ballistics = new PeakBallistics();

/** Read-only accessor for components. */
export function recordState(): {
  readonly state: RecordStateDto;
  readonly meter: InputMeterView;
  readonly clipLatched: boolean;
  readonly elapsedSamples: number;
  readonly newRecordingPrompt: DefaultFormatDto | null;
  readonly lowDiskPrompt: { readonly minutes: number } | null;
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
    meter = { ...SILENT };
  }
  state = next;
}

async function run(command: () => Promise<RecordStateDto>): Promise<void> {
  try {
    applyState(await command());
  } catch (err) {
    report(err);
  }
}

const nowMs = (): number => (typeof performance !== "undefined" ? performance.now() : Date.now());

/** One telemetry frame: input meter, clip latch, elapsed take time. */
export function onInputTelemetry(frame: TelemetryFrame, atMs: number = nowMs()): void {
  if ((frame.flags & VXTM_FLAGS.IN_CLIP) !== 0) {
    clipLatched = true;
  }
  if ((frame.flags & VXTM_FLAGS.RECORDING) !== 0) {
    elapsedSamples = frame.playheadSample;
  }
  ballistics.update(frame.inPeakDbfs, atMs);
  meter = {
    peakDbfs: ballistics.bar,
    holdDbfs: ballistics.hold,
    rmsDbfs: frame.inRmsDbfs,
    maxDbfs: Math.max(meter.maxDbfs, frame.inPeakDbfs),
  };
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
 * Record button / Shift+R: stops a running recording; otherwise starts a new one at the current
 * default format, no dialog (SPEC-002 §2.2: "Record with no document uses the default"). A
 * document with audio instead opens the New Recording dialog (H-10 item 7, SPEC-002 §2.2 parity
 * with File → New Recording…) after the standard unsaved-changes prompt (S1-03's guard) — so the
 * owner picks the format for the replacement take instead of it silently reusing the default.
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
      await withUnsavedChangesGuard(() => {
        openNewRecordingPrompt(settingsState().current?.default_format ?? FALLBACK_FORMAT);
        return Promise.resolve();
      });
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

/** Wires the store (keymap action, `record_state` events, telemetry); returns the teardown. */
export function initRecord(): () => void {
  const cleanups: Array<() => void> = [
    registerAction("record.toggle", () => void toggleRecord()),
    addTelemetryListener((frame) => onInputTelemetry(frame)),
  ];
  let disposed = false;
  listen<RecordStateDto>("record_state" satisfies EventName, (e) => applyState(e.payload))
    .then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    })
    .catch(() => {});
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
  ballistics.reset();
}

/** Test helper: sets the record state directly, without going through the engine/events. */
export function applyRecordStateForTest(patch: Partial<RecordStateDto>): void {
  state = { ...state, ...patch };
}
