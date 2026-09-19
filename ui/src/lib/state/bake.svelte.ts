import { listen } from "@tauri-apps/api/event";
import type { EventName, IpcError, JobProgressDto, RackSlotDto } from "../ipc/bindings";
import { editBakeCancel, editBakeStart } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { rackHasNoiseOnlyOn } from "../export/export.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { rackState } from "../rack/rack.svelte";
import { startJobStatusPoll } from "./jobStatusPoll";
import { pushNotice } from "./notices.svelte";
import { recordState } from "./record.svelte";
import { hasSelection, selectionState } from "./selection.svelte";

/**
 * Bake rack store (T-602; SPEC-004 §2.2, OD-4): Effects → Bake Rack renders the live rack into
 * the selection (or the whole file with none), as one undo entry, then the rack is reset. The
 * backend does the work as a job (`edit_bake_start`/`edit_bake_cancel`, `job_progress` kind
 * `bake`); the committed document and history arrive as `document_changed`/`history_state`, and
 * the "done" toast as a `notice` — so this store only tracks the job for the progress dialog.
 *
 * Confirmation: SPEC-014 §2.6 — when a non-bypassed Noise Reduction slot has "Output noise only"
 * on, Bake asks first (Continue / Cancel), like Export. Otherwise it runs at once: the bake is
 * undoable, like every other destructive edit.
 */

export interface BakeJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let job = $state<BakeJobState | null>(null);
let confirm = $state<{ range: [number, number] } | null>(null);
/** `true` between `edit_bake_start` being sent and its job id arriving. */
let starting = false;
/** `job_progress` events of a bake that arrived before its job id did (a short bake can finish
 * before the start command resolves); applied once the id is known. H-96: gated on `starting`
 * alone, not on `job` being null — a *previous*, already-finished bake can still be sitting in
 * `job` (not yet dismissed) when a new one starts, and its stale, non-matching id must not stop
 * the new job's own early events from being buffered (see `applyBakeJobProgress`). */
let early: JobProgressDto[] = [];
/** H-96 item 2: stops the belt-and-braces `job_status` recovery poll for the current job. */
let stopStatusPoll: (() => void) | null = null;
let unlistenProgress: (() => void) | null = null;

/** Read-only accessor for components. */
export function bakeState(): {
  readonly job: BakeJobState | null;
  readonly confirmOpen: boolean;
} {
  return {
    get job() {
      return job;
    },
    get confirmOpen() {
      return confirm !== null;
    },
  };
}

/** Whether a rack has anything to bake: at least one slot that isn't bypassed (whole-rack A/B is
 * a listening aid that offline renders ignore, SPEC-012 §2.3). */
export function rackIsActive(slots: RackSlotDto[]): boolean {
  return slots.some((slot) => !slot.bypass);
}

/** `[start, end)` of the current selection, or the whole open document with none; `null` with
 * no document (or an empty one). */
function scope(): [number, number] | null {
  if (hasSelection()) {
    const current = selectionState().current!;
    return [current.startSample, current.endSample];
  }
  const doc = documentState().current;
  return doc.sample_rate_hz > 0 && doc.len_samples > 0 ? [0, doc.len_samples] : null;
}

/** Effects → Bake Rack is enabled: a document with audio, not recording, a rack with an active
 * slot, and no bake already running. */
export function canBake(): boolean {
  const record = recordState().state;
  return (
    scope() !== null &&
    !record.recording &&
    !record.finishing &&
    rackIsActive(rackState().state.slots) &&
    (!job || job.state !== "running")
  );
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** Effects → Bake Rack. */
export async function startBake(): Promise<void> {
  if (!canBake()) {
    return;
  }
  const range = scope()!;
  if (rackHasNoiseOnlyOn(rackState().state.slots)) {
    confirm = { range };
    return;
  }
  await run(range);
}

/** SPEC-014 §2.6 "Cancel": nothing happens. */
export function cancelBakeConfirm(): void {
  confirm = null;
}

/** SPEC-014 §2.6 "Continue": bakes the scope that was confirmed. */
export async function continueBakeConfirm(): Promise<void> {
  const pending = confirm;
  confirm = null;
  if (pending) {
    await run(pending.range);
  }
}

async function run(range: [number, number]): Promise<void> {
  await ensureListening();
  starting = true;
  early = [];
  try {
    const started = await editBakeStart(range[0], range[1]);
    job = { jobId: started.job_id, fraction: 0, state: "running" };
    starting = false;
    const buffered = early;
    early = [];
    for (const payload of buffered) {
      applyBakeJobProgress(payload);
    }
    stopStatusPoll?.();
    stopStatusPoll = startJobStatusPoll(
      started.job_id,
      () => job !== null && job.jobId === started.job_id && job.state === "running",
      applyBakeJobProgress,
    );
  } catch (err) {
    report(err);
  } finally {
    starting = false;
    early = [];
  }
}

/** Applies one `job_progress` event (kind `bake` only) — pure, so it's directly testable. H-96:
 * buffers while `starting` regardless of whether `job` currently holds a *previous*, already-
 * finished bake (not yet dismissed) — gating on `!job` instead would let that stale job's
 * non-matching id fall through to the `payload.job_id !== job.jobId` check below and silently
 * drop the new job's own early events. */
export function applyBakeJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "bake") {
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

async function ensureListening(): Promise<void> {
  if (unlistenProgress) {
    return;
  }
  try {
    unlistenProgress = await listen<JobProgressDto>("job_progress" satisfies EventName, (event) =>
      applyBakeJobProgress(event.payload),
    );
  } catch {
    // Not inside a Tauri window (Vitest): tests drive `applyBakeJobProgress` directly.
  }
}

/** Clears a finished job (the progress dialog calls it on Done/Cancelled/Failed). */
export function dismissBakeJob(): void {
  stopStatusPoll?.();
  stopStatusPoll = null;
  job = null;
}

/** The progress dialog's Cancel (`edit_bake_cancel`, best-effort). */
export function cancelBakeJob(): void {
  if (job && job.state === "running") {
    void editBakeCancel(job.jobId).catch(report);
  }
}

/** Test/teardown helper. */
export function resetBakeForTest(): void {
  job = null;
  confirm = null;
  starting = false;
  early = [];
  stopStatusPoll?.();
  stopStatusPoll = null;
  unlistenProgress?.();
  unlistenProgress = null;
}
