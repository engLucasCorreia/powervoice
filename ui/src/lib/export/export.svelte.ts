import { listen } from "@tauri-apps/api/event";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type {
  EventName,
  ExportFormatDto,
  ExportRangeDto,
  ExportRequestDto,
  IpcError,
  JobProgressDto,
  RackSlotDto,
} from "../ipc/bindings";
import { exportCancel, exportFormats, exportStart } from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { rackState } from "../rack/rack.svelte";
import { pushNotice } from "../state/notices.svelte";

/**
 * Export dialog store (S4-04, H-08): the format/rate/range settings prompt, MP3 availability,
 * an optional "Output noise only" confirmation (SPEC-014 §2.6), and the running job's progress
 * (via `job_progress` events, ADR-003). The native destination picker is `tauri-plugin-dialog`'s
 * save dialog, like `document.svelte.ts`'s Save As.
 */

export interface ExportPrompt {
  suggestedName: string;
}

export interface ExportJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

/** A pending export whose format/rate/range are chosen, awaiting the "Output noise only"
 * confirmation (SPEC-014 §2.6) before the native save dialog opens. */
export interface NoiseOnlyConfirmState {
  format: ExportFormatDto;
  sampleRateHz: number;
  range: ExportRangeDto | null;
}

let prompt = $state<ExportPrompt | null>(null);
let mp3Available = $state(false);
let job = $state<ExportJobState | null>(null);
let noiseOnlyConfirm = $state<NoiseOnlyConfirmState | null>(null);
let unlistenProgress: (() => void) | null = null;

/** Read-only accessor for components. */
export function exportState(): {
  readonly prompt: ExportPrompt | null;
  readonly mp3Available: boolean;
  readonly job: ExportJobState | null;
  readonly noiseOnlyConfirm: NoiseOnlyConfirmState | null;
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
    get noiseOnlyConfirm() {
      return noiseOnlyConfirm;
    },
  };
}

/** SPEC-014 §2.6: a non-bypassed Noise Reduction slot with "Output noise only" (`noise_only`) on
 * — a pure function over the rack DTO, directly testable without the rack store. */
export function slotIsNoiseOnly(slot: RackSlotDto): boolean {
  if (slot.bypass) {
    return false;
  }
  const index = slot.params.findIndex((p) => p.key === "noise_only");
  if (index === -1) {
    return false;
  }
  return (slot.values[index]?.value ?? 0) !== 0;
}

/** SPEC-014 §2.6: "export and bake show a confirmation when any non-bypassed NR slot has
 * [Output noise only] on". */
export function rackHasNoiseOnlyOn(slots: RackSlotDto[]): boolean {
  return slots.some(slotIsNoiseOnly);
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
 * Confirms the export prompt: when the live rack has a non-bypassed NR slot outputting noise
 * only (SPEC-014 §2.6), shows a confirmation first (`noiseOnlyConfirm`/`continueNoiseOnlyExport`/
 * `cancelNoiseOnlyExport`); otherwise goes straight to the native save dialog, then starts the
 * job if a path was chosen. `range` is `null` for the whole file, or the chosen selection
 * (H-08 — the dialog's Whole file/Selection choice).
 */
export async function confirmExport(
  format: ExportFormatDto,
  sampleRateHz: number,
  range: ExportRangeDto | null = null,
): Promise<void> {
  if (rackHasNoiseOnlyOn(rackState().state.slots)) {
    prompt = null;
    noiseOnlyConfirm = { format, sampleRateHz, range };
    return;
  }
  await startExport(format, sampleRateHz, range);
}

/** SPEC-014 §2.6 "Cancel": drops the pending export, returning to no dialog. */
export function cancelNoiseOnlyExport(): void {
  noiseOnlyConfirm = null;
}

/** SPEC-014 §2.6 "Export anyway": proceeds to the native save dialog with the confirmed
 * settings. */
export async function continueNoiseOnlyExport(): Promise<void> {
  const pending = noiseOnlyConfirm;
  noiseOnlyConfirm = null;
  if (!pending) {
    return;
  }
  await startExport(pending.format, pending.sampleRateHz, pending.range);
}

async function startExport(
  format: ExportFormatDto,
  sampleRateHz: number,
  range: ExportRangeDto | null,
): Promise<void> {
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
  const request: ExportRequestDto = { path, format, sample_rate_hz: sampleRateHz, range };
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
  noiseOnlyConfirm = null;
  unlistenProgress?.();
  unlistenProgress = null;
}
