import { listen } from "@tauri-apps/api/event";
import type { EventName, IpcError, JobProgressDto } from "../ipc/bindings";
import { nrCaptureCancel, nrCaptureStart } from "../ipc/commands";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { recordState } from "../state/record.svelte";
import { hasSelection, selectionState } from "../state/selection.svelte";
import { lastFocusedSlotIndex } from "./rack.svelte";

/**
 * Capture Noise Print (S3-06, SPEC-014 §2.3): Shift+P and the NR slot panel's Capture button.
 * Mirrors `export.svelte.ts`'s job-progress shape (`job_progress`, kind `nr_capture`); the
 * resulting print itself is reflected through the existing `rack_changed`/`rack.svelte.ts` path
 * (the slot's `noise_profile` status), not tracked separately here.
 */

export interface NrCaptureJob {
  jobId: number;
  /** The target slot, authoritative even when the backend just inserted it. */
  slot: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let job = $state<NrCaptureJob | null>(null);
let unlistenProgress: (() => void) | null = null;

/** Read-only accessor for components. */
export function nrCaptureState(): { readonly job: NrCaptureJob | null } {
  return {
    get job() {
      return job;
    },
  };
}

/** True while slot `index` has a capture job running (the panel's spinner/Cancel). */
export function isCapturing(index: number): boolean {
  return job !== null && job.state === "running" && job.slot === index;
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** SPEC-014 §2.3 "Enabled when": a non-empty selection and not recording. */
export function canCapture(): boolean {
  return hasSelection() && !recordState().state.recording;
}

/**
 * Starts a capture (SPEC-014 §2.3). `hintSlot` is the slot whose Capture button was clicked
 * (unambiguous); omitted (Shift+P), it falls back to the last-focused rack slot, which the
 * backend validates and otherwise resolves itself (first NR slot, else a new one). A no-op
 * without a selection, while recording, or while a capture is already running.
 */
export async function startCapture(hintSlot: number | null = null): Promise<void> {
  if (!canCapture() || (job !== null && job.state === "running")) {
    return;
  }
  const sel = selectionState().current;
  if (!sel) {
    return;
  }
  const hint = hintSlot ?? lastFocusedSlotIndex();
  try {
    const started = await nrCaptureStart(hint, sel.startSample, sel.endSample);
    job = { jobId: started.job_id, slot: started.slot, fraction: 0, state: "running" };
    await ensureListening();
  } catch (err) {
    report(err);
  }
}

/** Applies one `job_progress` event (kind `nr_capture`) — a pure function, directly testable
 * without a real Tauri event transport (mirrors `export.svelte.ts::applyJobProgress`). */
export function applyJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "nr_capture" || job === null || payload.job_id !== job.jobId) {
    return;
  }
  job = { ...job, fraction: payload.fraction, state: payload.state };
}

async function ensureListening(): Promise<void> {
  if (unlistenProgress) {
    return;
  }
  try {
    unlistenProgress = await listen<JobProgressDto>("job_progress" satisfies EventName, (e) =>
      applyJobProgress(e.payload),
    );
  } catch {
    // Not running inside a real Tauri window (e.g. Vitest) — the job still reports through its
    // command result and `rack_changed`.
  }
}

/** Cancels the running job (`nr_capture_cancel`, best-effort — leaves any previous print
 * unchanged). */
export function cancelCapture(): void {
  if (job) {
    void nrCaptureCancel(job.jobId).catch(report);
  }
}

/** Wires the Shift+P keymap action. Returns the teardown. */
export function initNrCapture(): () => void {
  const unregister = registerAction("nr.capture_noise_print", () => void startCapture(null));
  return () => {
    unregister();
    unlistenProgress?.();
    unlistenProgress = null;
  };
}

/** Test/teardown helper. */
export function resetNrCaptureForTest(): void {
  job = null;
  unlistenProgress?.();
  unlistenProgress = null;
}
