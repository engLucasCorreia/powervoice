import { listen } from "@tauri-apps/api/event";
import type { EventName, IpcError, JobProgressDto, NormalizeResultDto } from "../ipc/bindings";
import { editNormalizeLufsCancel, editNormalizeLufsStart } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { startJobStatusPoll } from "./jobStatusPoll";
import { pushNotice } from "./notices.svelte";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";
import { formatNumber, parseNumber } from "../ui/units";

/**
 * LUFS normalize store (S4-01, PROMPT §3.3): the three integrated-loudness favorites and the
 * Normalize (LUFS)… dialog. Same scope convention as peak normalize (`normalize.svelte.ts`): the
 * current non-empty selection, or the whole file when there is none. A no-op (silent scope,
 * already at the target) or an applied gain whose predicted true peak exceeds −1 dBTP is reported
 * by the backend as a `notice` event (`notices.svelte.ts`'s `initNotices`), not by this module.
 *
 * H-09: runs as a job (`edit_normalize_lufs_start`/`_cancel`), mirroring `normalize.svelte.ts`
 * (progress via `job_progress` kind `normalize_lufs`, the finished edit via `normalize_result`).
 * No % mode (SPEC-010 §2.4 is peak-only; LUFS has no percent-of-full-scale reading).
 */

/** The three favorite targets, PROMPT §3.3 order. */
export const FAVORITE_TARGETS_LUFS = [-16, -19, -23] as const;

/** Custom-target range of the Normalize (LUFS)… dialog (mirrors the backend's own bounds). */
export const TARGET_MIN_LUFS = -60;
export const TARGET_MAX_LUFS = 0;
const DEFAULT_TARGET_LUFS = -19;

interface DialogState {
  /** The raw text field value, so an invalid in-progress edit doesn't snap back. */
  text: string;
  valid: boolean;
}

export interface NormalizeLufsJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let dialogOpen = $state(false);
let dialog = $state<DialogState>({ text: formatTarget(DEFAULT_TARGET_LUFS), valid: true });
let job = $state<NormalizeLufsJobState | null>(null);
let unlistenProgress: (() => void) | null = null;
let unlistenResult: (() => void) | null = null;
/** H-96: see `export.svelte.ts`'s own `starting`/`early` for what this covers. */
let starting = false;
let early: JobProgressDto[] = [];
/** H-96 item 2: stops the belt-and-braces `job_status` recovery poll for the current job. */
let stopStatusPoll: (() => void) | null = null;

/** Read-only accessor for components. */
export function normalizeLufsState(): {
  readonly dialogOpen: boolean;
  readonly dialogText: string;
  readonly dialogValid: boolean;
  readonly job: NormalizeLufsJobState | null;
} {
  return {
    get dialogOpen() {
      return dialogOpen;
    },
    get dialogText() {
      return dialog.text;
    },
    get dialogValid() {
      return dialog.valid;
    },
    get job() {
      return job;
    },
  };
}

// H-28 item 4: U+2212 (not the ASCII hyphen `toFixed` writes) via `units.ts::formatNumber`.
function formatTarget(lufs: number): string {
  return formatNumber(lufs, 1);
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** `[start, end)` of the current selection, or the whole open document with none; `null` with no
 * document open. */
function scope(): [number, number] | null {
  if (hasSelection()) {
    const current = selectionState().current!;
    return [current.startSample, current.endSample];
  }
  const doc = documentState().current;
  return doc.sample_rate_hz > 0 && doc.len_samples > 0 ? [0, doc.len_samples] : null;
}

/** `true` when a LUFS normalize command can run right now (a document with audio is open, and no
 * job is already running). */
export function canNormalizeLufs(): boolean {
  return scope() !== null && (!job || job.state !== "running");
}

async function run(targetLufs: number): Promise<void> {
  const range = scope();
  if (!range) {
    return;
  }
  // H-96: subscribe *before* starting the job (see `export.svelte.ts::startExport`'s comment).
  await ensureListening();
  starting = true;
  early = [];
  try {
    const started = await editNormalizeLufsStart(range[0], range[1], targetLufs);
    job = { jobId: started.job_id, fraction: 0, state: "running" };
    starting = false;
    const buffered = early;
    early = [];
    for (const payload of buffered) {
      applyNormalizeLufsJobProgress(payload);
    }
    stopStatusPoll?.();
    stopStatusPoll = startJobStatusPoll(
      started.job_id,
      () => job !== null && job.jobId === started.job_id && job.state === "running",
      applyNormalizeLufsJobProgress,
    );
  } catch (err) {
    report(err);
  } finally {
    starting = false;
    early = [];
  }
}

/** A favorite toolbar button / Favorites menu item (one click, no dialog). */
export const normalizeLufsFavorite = (targetLufs: number): Promise<void> => run(targetLufs);

/** Parses the dialog's LUFS text field (locale-neutral) into a finite value within
 * `[TARGET_MIN_LUFS, TARGET_MAX_LUFS]`, or `null`. H-28 item 4: `units.ts::parseNumber` accepts
 * both the ASCII `-` and the U+2212 minus `formatTarget` now writes. */
export function parseTargetLufs(text: string): number | null {
  const value = parseNumber(text);
  if (value === null || value < TARGET_MIN_LUFS || value > TARGET_MAX_LUFS) {
    return null;
  }
  return value;
}

/** Effects → Normalize (LUFS)… / the Favorites menu's "Normalize (LUFS)…". */
export function openNormalizeLufsDialog(): void {
  if (!canNormalizeLufs()) {
    return;
  }
  dialog = { text: dialog.text, valid: parseTargetLufs(dialog.text) !== null };
  dialogOpen = true;
}

export function closeNormalizeLufsDialog(): void {
  dialogOpen = false;
}

export function setNormalizeLufsDialogText(text: string): void {
  dialog = { text, valid: parseTargetLufs(text) !== null };
}

/** Enter / the dialog's Apply button. A no-op while the field is invalid. */
export async function applyNormalizeLufsDialog(): Promise<void> {
  const value = parseTargetLufs(dialog.text);
  if (value === null) {
    return;
  }
  dialogOpen = false;
  await run(value);
}

/** Applies one `job_progress` event to the store (kind `normalize_lufs` only). H-96: while a job
 * is starting (the command sent, its id not yet known locally), a matching event is buffered
 * rather than dropped — see `run`. */
export function applyNormalizeLufsJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "normalize_lufs") {
    return;
  }
  if (starting) {
    early.push(payload);
    return;
  }
  if (!job || payload.job_id !== job.jobId) {
    return;
  }
  job = { jobId: payload.job_id, fraction: payload.fraction, state: payload.state };
}

/** Applies a finished `normalize_result` event (kind `normalize_lufs` only). */
export function applyNormalizeLufsResult(payload: NormalizeResultDto): void {
  if (payload.kind !== "normalize_lufs" || !job || payload.job_id !== job.jobId) {
    return;
  }
  setSelectionFromResult(payload.result.selection);
}

async function ensureListening(): Promise<void> {
  if (!unlistenProgress) {
    try {
      unlistenProgress = await listen<JobProgressDto>(
        "job_progress" satisfies EventName,
        (event) => applyNormalizeLufsJobProgress(event.payload),
      );
    } catch {
      // Not running inside a real Tauri window (e.g. Vitest) — tests drive the store functions
      // directly instead.
    }
  }
  if (!unlistenResult) {
    try {
      unlistenResult = await listen<NormalizeResultDto>(
        "normalize_result" satisfies EventName,
        (event) => applyNormalizeLufsResult(event.payload),
      );
    } catch {
      // See above.
    }
  }
}

/** Dismisses a finished job's progress panel (Done/Cancelled/Failed). */
export function dismissNormalizeLufsJob(): void {
  stopStatusPoll?.();
  stopStatusPoll = null;
  job = null;
}

/** Cancels the running job (`edit_normalize_lufs_cancel`, best-effort). */
export function cancelNormalizeLufsJob(): void {
  if (job && job.state === "running") {
    void editNormalizeLufsCancel(job.jobId).catch(report);
  }
}

/** Test/teardown helper. */
export function resetNormalizeLufsForTest(): void {
  dialogOpen = false;
  dialog = { text: formatTarget(DEFAULT_TARGET_LUFS), valid: true };
  job = null;
  starting = false;
  early = [];
  stopStatusPoll?.();
  stopStatusPoll = null;
  unlistenProgress?.();
  unlistenProgress = null;
  unlistenResult?.();
  unlistenResult = null;
}

/**
 * Wires the store. LUFS normalize has no keymap actions and no one-off events of its own beyond
 * the shared job/result events — its notices arrive through the shared `notice` event. Returns a
 * teardown that stops listening for job events.
 */
export function initNormalizeLufs(): () => void {
  return () => {
    unlistenProgress?.();
    unlistenProgress = null;
    unlistenResult?.();
    unlistenResult = null;
  };
}
