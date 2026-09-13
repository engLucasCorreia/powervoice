import { listen } from "@tauri-apps/api/event";
import type {
  EventName,
  IpcError,
  JobProgressDto,
  LoudnessReportDto,
  LoudnessSourceDto,
} from "../ipc/bindings";
import { loudnessAnalyzeCancel, loudnessAnalyzeStart } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { hasSelection, selectionState } from "../state/selection.svelte";

/**
 * Loudness analysis store (S4-01): the bottom-dock Loudness panel's "Analyze" job (source
 * toggle, progress, finished report). Scope is the current selection, or the whole file when
 * there is none — same convention as normalize. The job runs through `loudness_analyze_start`
 * (`job_progress` events tagged `kind: "loudness_analyze"`, then a `loudness_report` event
 * carrying the [`LoudnessReportDto`]).
 */

export interface LoudnessJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let source = $state<LoudnessSourceDto>("processed");
let job = $state<LoudnessJobState | null>(null);
let report = $state<LoudnessReportDto | null>(null);
let unlistenProgress: (() => void) | null = null;
let unlistenReport: (() => void) | null = null;

/** Read-only accessor for components. */
export function loudnessState(): {
  readonly source: LoudnessSourceDto;
  readonly job: LoudnessJobState | null;
  readonly report: LoudnessReportDto | null;
} {
  return {
    get source() {
      return source;
    },
    get job() {
      return job;
    },
    get report() {
      return report;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function reportError(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** `[start, end)` of the current selection, or the whole open document with none; `null` with no
 * document open (the panel disables Analyze then). */
function scope(): [number, number] | null {
  if (hasSelection()) {
    const current = selectionState().current!;
    return [current.startSample, current.endSample];
  }
  const doc = documentState().current;
  return doc.sample_rate_hz > 0 && doc.len_samples > 0 ? [0, doc.len_samples] : null;
}

/** `true` when Analyze can run right now. */
export function canAnalyzeLoudness(): boolean {
  return scope() !== null;
}

export function setLoudnessSource(next: LoudnessSourceDto): void {
  source = next;
}

/** Applies one `job_progress` event to the store — pure, so it's directly testable (mirrors
 * `export.svelte.ts`'s `applyJobProgress`). Ignores events for other jobs/kinds. */
export function applyLoudnessJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "loudness_analyze" || !job || payload.job_id !== job.jobId) {
    return;
  }
  job = { jobId: payload.job_id, fraction: payload.fraction, state: payload.state };
}

/** Applies a finished `loudness_report` event — pure, like `applyLoudnessJobProgress`. */
export function applyLoudnessReport(payload: LoudnessReportDto): void {
  if (!job || payload.job_id !== job.jobId) {
    return;
  }
  report = payload;
}

async function ensureListening(): Promise<void> {
  if (!unlistenProgress) {
    try {
      unlistenProgress = await listen<JobProgressDto>(
        "job_progress" satisfies EventName,
        (event) => applyLoudnessJobProgress(event.payload),
      );
    } catch {
      // Not running inside a real Tauri window (e.g. Vitest) — the job still reports through
      // its command result flow for tests that drive the store functions directly.
    }
  }
  if (!unlistenReport) {
    try {
      unlistenReport = await listen<LoudnessReportDto>(
        "loudness_report" satisfies EventName,
        (event) => applyLoudnessReport(event.payload),
      );
    } catch {
      // See above.
    }
  }
}

/** The Loudness panel's "Analyze" button. */
export async function startLoudnessAnalyze(): Promise<void> {
  const range = scope();
  if (!range) {
    return;
  }
  try {
    const started = await loudnessAnalyzeStart({
      start_sample: range[0],
      end_sample: range[1],
      source,
    });
    job = { jobId: started.job_id, fraction: 0, state: "running" };
    await ensureListening();
  } catch (err) {
    reportError(err);
  }
}

/** Cancels the running analysis job (best-effort). */
export function cancelLoudnessAnalyze(): void {
  if (job) {
    void loudnessAnalyzeCancel(job.jobId).catch(reportError);
  }
}

/** Test/teardown helper. */
export function resetLoudnessForTest(): void {
  source = "processed";
  job = null;
  report = null;
  unlistenProgress?.();
  unlistenProgress = null;
  unlistenReport?.();
  unlistenReport = null;
}

/** Wires the store (App.svelte mounts this like every other `init*` feature module). */
export function initLoudness(): () => void {
  void ensureListening();
  return () => {
    unlistenProgress?.();
    unlistenProgress = null;
    unlistenReport?.();
    unlistenReport = null;
  };
}
