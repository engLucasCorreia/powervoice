import { listen } from "@tauri-apps/api/event";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type { EventName, ExportFormatDto, ExportRequestDto, IpcError, JobProgressDto } from "../ipc/bindings";
import { exportCancel, exportFormats, exportStart } from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";

/**
 * Export dialog store (S4-04): the format/rate settings prompt, MP3 availability, and the
 * running job's progress (via `job_progress` events, ADR-003). The native destination picker is
 * `tauri-plugin-dialog`'s save dialog, like `document.svelte.ts`'s Save As.
 *
 * **Ticket deviation:** the dialog only offers "whole file" — there is no selection model in the
 * app yet (S2-01 is concurrent, unmerged). `exportStart`'s request already carries an optional
 * range end to end (see `../export.rs`), so wiring a selection through here is additive later.
 */

export interface ExportPrompt {
  suggestedName: string;
}

export interface ExportJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let prompt = $state<ExportPrompt | null>(null);
let mp3Available = $state(false);
let job = $state<ExportJobState | null>(null);
let unlistenProgress: (() => void) | null = null;

/** Read-only accessor for components. */
export function exportState(): {
  readonly prompt: ExportPrompt | null;
  readonly mp3Available: boolean;
  readonly job: ExportJobState | null;
} {
  return {
    get prompt() {
      return prompt;
    },
    get mp3Available() {
      return mp3Available;
    },
    get job() {
      return job;
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

/** File → Export… : opens the dialog and refreshes MP3 availability. */
export function openExportDialog(suggestedName: string): void {
  prompt = { suggestedName };
  void exportFormats()
    .then((formats) => {
      mp3Available = formats.mp3_available;
    })
    .catch(report);
}

export function cancelExportDialog(): void {
  prompt = null;
}

export function extensionFor(format: ExportFormatDto): "wav" | "flac" | "mp3" {
  return format.kind;
}

function filtersFor(format: ExportFormatDto): { name: string; extensions: string[] }[] {
  const ext = extensionFor(format);
  return [{ name: ext.toUpperCase(), extensions: [ext] }];
}

/**
 * Confirms the export prompt: shows the native save dialog, then starts the job if a path was
 * chosen. Whole-file only for now (see the module doc's deviation note).
 */
export async function confirmExport(format: ExportFormatDto, sampleRateHz: number): Promise<void> {
  const suggested = prompt?.suggestedName ?? "untitled";
  const ext = extensionFor(format);
  const path = await saveFileDialog({
    defaultPath: `${suggested}.${ext}`,
    filters: filtersFor(format),
  });
  if (typeof path !== "string") {
    return;
  }
  prompt = null;
  const request: ExportRequestDto = { path, format, sample_rate_hz: sampleRateHz, range: null };
  try {
    const started = await exportStart(request);
    job = { jobId: started.job_id, fraction: 0, state: "running" };
    await ensureListening();
  } catch (err) {
    report(err);
  }
}

/** Applies one `job_progress` event to the store — a pure function so it's directly testable
 * without a real Tauri event transport (mirrors `document.svelte.ts`'s untested `listen` wiring:
 * the side effect is best-effort, the logic it drives is not). */
export function applyJobProgress(payload: JobProgressDto): void {
  if (!job || payload.job_id !== job.jobId) {
    return;
  }
  job = { jobId: payload.job_id, fraction: payload.fraction, state: payload.state };
}

async function ensureListening(): Promise<void> {
  if (unlistenProgress) {
    return;
  }
  try {
    unlistenProgress = await listen<JobProgressDto>(
      "job_progress" satisfies EventName,
      (event) => applyJobProgress(event.payload),
    );
  } catch {
    // Not running inside a real Tauri window (e.g. Vitest) — the job still reports through its
    // command results.
  }
}

/** Dismisses a finished job's progress panel (Done/Cancelled/Failed). */
export function dismissExportJob(): void {
  job = null;
}

/** Cancels the running job (`export_cancel`, best-effort). */
export function cancelExportJob(): void {
  if (job) {
    void exportCancel(job.jobId).catch(report);
  }
}

/** Test/teardown helper. */
export function resetExportStateForTest(): void {
  prompt = null;
  mp3Available = false;
  job = null;
  unlistenProgress?.();
  unlistenProgress = null;
}
