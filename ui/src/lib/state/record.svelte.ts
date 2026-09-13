import { listen } from "@tauri-apps/api/event";
import { documentState, withUnsavedChangesGuard } from "../document/document.svelte";
import type { EventName, IpcError, MonitorMode, RecordStateDto } from "../ipc/bindings";
import { recordArm, recordGet, recordSetMonitor, recordStart, recordStop } from "../ipc/record_commands";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { PeakBallistics } from "../record/ballistics";
import { pushNotice } from "./notices.svelte";
import { addTelemetryListener } from "./transport.svelte";

/**
 * Record panel store (S1-04, SPEC-002 §2.1–§2.2, §2.7): the engine's record state
 * (`record_state` event + command results), the input meter with UI ballistics, the clip latch
 * and the elapsed take time (from `VXTM` telemetry). Registers the Record action (Shift+R).
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
const ballistics = new PeakBallistics();

/** Read-only accessor for components. */
export function recordState(): {
  readonly state: RecordStateDto;
  readonly meter: InputMeterView;
  readonly clipLatched: boolean;
  readonly elapsedSamples: number;
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

async function startRecording(replace: boolean): Promise<void> {
  // A lit lamp after a take always means that take clipped (SPEC-002 §2.1).
  clipLatched = false;
  elapsedSamples = 0;
  await run(() => recordStart(replace));
}

/**
 * Record button / Shift+R: stops a running recording; otherwise starts a new one. A document with
 * audio is replaced after the standard unsaved-changes prompt (SPEC-002 §2.2, S1-03's guard).
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
      await withUnsavedChangesGuard(() => startRecording(true));
    } else {
      await startRecording(false);
    }
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
  ballistics.reset();
}
