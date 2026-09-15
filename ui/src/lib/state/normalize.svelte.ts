import { listen } from "@tauri-apps/api/event";
import type {
  EventName,
  IpcError,
  JobProgressDto,
  NormalizeResultDto,
  NormalizeTargetUnit,
} from "../ipc/bindings";
import { editNormalizePeakCancel, editNormalizePeakStart } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "./notices.svelte";
import { saveSettings, settingsState } from "./settings.svelte";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";
import { formatNumber, parseNumber } from "../ui/units";

/**
 * Normalize store (S2-02, SPEC-010): the three one-click favorites and the Normalize… dialog.
 * Scope is the current non-empty selection, or the whole file when there is none (SPEC-010
 * §2.1 — LOCKED, PROMPT §2). A no-op (silent scope, or already at the target) is reported by the
 * backend as a `notice` event (`notices.svelte.ts`'s `initNotices`), not by this module.
 *
 * H-09: normalize runs as a job (`edit_normalize_peak_start`/`_cancel`); progress arrives as
 * `job_progress` events (kind `normalize_peak`) and the finished edit result as `normalize_result`
 * (mirrors `export.svelte.ts`/`loudness.svelte.ts`'s job stores). The dialog also gained a %
 * mode (SPEC-010 §2.4) and remembers its last applied value/unit via `settings.svelte.ts`.
 */

/** The three favorite targets, in PROMPT §3.3 order (SPEC-010 §2.1/§2.5). */
export const FAVORITE_TARGETS_DB = [-1, -0.1, -3] as const;

/** dB-mode range of the Normalize… dialog (SPEC-010 §2.4). */
export const TARGET_MIN_DB = -60;
export const TARGET_MAX_DB = 0;
/** %-mode range (SPEC-010 §2.4: 100 % = 0 dBFS, the upper bound so clipping stays impossible). */
export const TARGET_MIN_PCT = 0.1;
export const TARGET_MAX_PCT = 100.0;

interface DialogState {
  /** The raw text field value, so an invalid in-progress edit doesn't snap back. */
  text: string;
  unit: NormalizeTargetUnit;
  valid: boolean;
}

export interface NormalizeJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
}

let dialogOpen = $state(false);
let dialog = $state<DialogState>({ text: formatTarget(-1, "db"), unit: "db", valid: true });
let job = $state<NormalizeJobState | null>(null);
let unlistenProgress: (() => void) | null = null;
let unlistenResult: (() => void) | null = null;

/** Read-only accessor for components. */
export function normalizeState(): {
  readonly dialogOpen: boolean;
  readonly dialogText: string;
  readonly dialogUnit: NormalizeTargetUnit;
  readonly dialogValid: boolean;
  readonly job: NormalizeJobState | null;
} {
  return {
    get dialogOpen() {
      return dialogOpen;
    },
    get dialogText() {
      return dialog.text;
    },
    get dialogUnit() {
      return dialog.unit;
    },
    get dialogValid() {
      return dialog.valid;
    },
    get job() {
      return job;
    },
  };
}

// H-28 item 4: U+2212 (not the ASCII hyphen `toFixed` writes) via `units.ts::formatNumber`, so
// the target field's minus sign matches every other numeric readout in the app.
function formatTarget(value: number, unit: NormalizeTargetUnit): string {
  return formatNumber(value, unit === "db" ? 2 : 1);
}

/** `T = 10^(target_db / 20)` in %, i.e. `100 * T` (mirrors `vox_project::target_db_to_pct`). */
export function targetDbToPct(db: number): number {
  return 100 * Math.pow(10, db / 20);
}

/** The inverse of {@link targetDbToPct} (mirrors `vox_project::target_pct_to_db`). */
export function targetPctToDb(pct: number): number {
  return 20 * Math.log10(pct / 100);
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** `[start, end)` of the current selection, or the whole open document with none (SPEC-010
 * §2.1); `null` with no document open (the toolbar/menu disable every command then). */
function scope(): [number, number] | null {
  if (hasSelection()) {
    const current = selectionState().current!;
    return [current.startSample, current.endSample];
  }
  const doc = documentState().current;
  return doc.sample_rate_hz > 0 && doc.len_samples > 0 ? [0, doc.len_samples] : null;
}

/** `true` when a normalize command can run right now (a document with audio is open, and no
 * normalize job is already running — SPEC-010 §2.1's "disabled ... while another document job
 * runs"). */
export function canNormalize(): boolean {
  return scope() !== null && (!job || job.state !== "running");
}

async function run(targetDb: number | null, targetPct: number | null): Promise<void> {
  const range = scope();
  if (!range) {
    return;
  }
  try {
    const started = await editNormalizePeakStart(range[0], range[1], targetDb, targetPct);
    job = { jobId: started.job_id, fraction: 0, state: "running" };
    await ensureListening();
  } catch (err) {
    report(err);
  }
}

/** A favorite toolbar button / Favorites menu item (SPEC-010 §2.1: one click, no dialog). */
export const normalizeFavorite = (targetDb: number): Promise<void> => run(targetDb, null);

/** Parses `text` in `unit` (SPEC-010 §2.4: locale-neutral) into a finite value within that unit's
 * range, or `null`. H-28 item 4: `units.ts::parseNumber` accepts both the ASCII `-` and the
 * U+2212 minus `formatTarget` now writes (plus the other dash variants a copied readout uses). */
export function parseNormalizeTarget(text: string, unit: NormalizeTargetUnit): number | null {
  const value = parseNumber(text);
  if (value === null) {
    return null;
  }
  const [min, max] = unit === "db" ? [TARGET_MIN_DB, TARGET_MAX_DB] : [TARGET_MIN_PCT, TARGET_MAX_PCT];
  return value >= min && value <= max ? value : null;
}

/** Backward-compatible alias (dB mode only) for existing tests/call sites. */
export function parseTargetDb(text: string): number | null {
  return parseNormalizeTarget(text, "db");
}

function lastApplied(): { value: number; unit: NormalizeTargetUnit } {
  const saved = settingsState().current?.normalize_dialog;
  return saved ?? { value: -1, unit: "db" };
}

/** Effects → Normalize… / the Favorites menu's "Normalize…" (SPEC-010 §2.4): reopens with the
 * last applied value and unit. */
export function openNormalizeDialog(): void {
  if (!canNormalize()) {
    return;
  }
  const { value, unit } = lastApplied();
  dialog = { text: formatTarget(value, unit), unit, valid: true };
  dialogOpen = true;
}

export function closeNormalizeDialog(): void {
  dialogOpen = false;
}

export function setNormalizeDialogText(text: string): void {
  dialog = { text, unit: dialog.unit, valid: parseNormalizeTarget(text, dialog.unit) !== null };
}

/** The dialog's dB/% toggle: converts the shown value (SPEC-010 §2.4: "−1.00 dB ↔ 89.1 %"),
 * rounded to the new unit's step. Leaves an already-invalid value invalid (still switches unit,
 * so the user can fix it there instead). */
export function setNormalizeDialogUnit(unit: NormalizeTargetUnit): void {
  if (unit === dialog.unit) {
    return;
  }
  const current = parseNormalizeTarget(dialog.text, dialog.unit);
  if (current === null) {
    dialog = { text: dialog.text, unit, valid: false };
    return;
  }
  const converted = dialog.unit === "db" ? targetDbToPct(current) : targetPctToDb(current);
  const [min, max] = unit === "db" ? [TARGET_MIN_DB, TARGET_MAX_DB] : [TARGET_MIN_PCT, TARGET_MAX_PCT];
  const clamped = Math.min(max, Math.max(min, converted));
  dialog = { text: formatTarget(clamped, unit), unit, valid: true };
}

/** Enter / the dialog's Apply button. A no-op while the field is invalid. Remembers the applied
 * value and unit (SPEC-010 §2.4, `settings.svelte.ts`). */
export async function applyNormalizeDialog(): Promise<void> {
  const value = parseNormalizeTarget(dialog.text, dialog.unit);
  if (value === null) {
    return;
  }
  const unit = dialog.unit;
  dialogOpen = false;
  void saveSettings({ normalize_dialog: { value, unit } }).catch(report);
  await run(unit === "db" ? value : null, unit === "pct" ? value : null);
}

/** Applies one `job_progress` event to the store (kind `normalize_peak` only) — a pure function
 * so it's directly testable (mirrors `export.svelte.ts`'s `applyJobProgress`). */
export function applyNormalizeJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "normalize_peak" || !job || payload.job_id !== job.jobId) {
    return;
  }
  job = { jobId: payload.job_id, fraction: payload.fraction, state: payload.state };
}

/** Applies a finished `normalize_result` event (kind `normalize_peak` only): updates the
 * selection from the committed edit (mirrors the old synchronous command's return value). */
export function applyNormalizeResult(payload: NormalizeResultDto): void {
  if (payload.kind !== "normalize_peak" || !job || payload.job_id !== job.jobId) {
    return;
  }
  setSelectionFromResult(payload.result.selection);
}

async function ensureListening(): Promise<void> {
  if (!unlistenProgress) {
    try {
      unlistenProgress = await listen<JobProgressDto>(
        "job_progress" satisfies EventName,
        (event) => applyNormalizeJobProgress(event.payload),
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
        (event) => applyNormalizeResult(event.payload),
      );
    } catch {
      // See above.
    }
  }
}

/** Dismisses a finished job's progress panel (Done/Cancelled/Failed). */
export function dismissNormalizeJob(): void {
  job = null;
}

/** Cancels the running job (`edit_normalize_peak_cancel`, best-effort). */
export function cancelNormalizeJob(): void {
  if (job && job.state === "running") {
    void editNormalizePeakCancel(job.jobId).catch(report);
  }
}

/** Test/teardown helper. */
export function resetNormalizeForTest(): void {
  dialogOpen = false;
  dialog = { text: formatTarget(-1, "db"), unit: "db", valid: true };
  job = null;
  unlistenProgress?.();
  unlistenProgress = null;
  unlistenResult?.();
  unlistenResult = null;
}

/**
 * Wires the store. Normalize has no keymap actions (SPEC-010 §2.5: no default shortcuts); its
 * one-off notices arrive through the shared `notice` event (`notices.svelte.ts`'s `initNotices`).
 * Returns a teardown that stops listening for job events.
 */
export function initNormalize(): () => void {
  return () => {
    unlistenProgress?.();
    unlistenProgress = null;
    unlistenResult?.();
    unlistenResult = null;
  };
}
