import type { IpcError } from "../ipc/bindings";
import { editNormalizePeak } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "./notices.svelte";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";

/**
 * Normalize store (S2-02, SPEC-010): the three one-click favorites and the Normalize… dialog.
 * Scope is the current non-empty selection, or the whole file when there is none (SPEC-010
 * §2.1 — LOCKED, PROMPT §2). A no-op (silent scope, or already at the target) is reported by the
 * backend as a `notice` event (`notices.svelte.ts`'s `initNotices`), not by this module.
 */

/** The three favorite targets, in PROMPT §3.3 order (SPEC-010 §2.1/§2.5). */
export const FAVORITE_TARGETS_DB = [-1, -0.1, -3] as const;

/** dB-mode range of the Normalize… dialog (SPEC-010 §2.4; % mode is deferred). */
export const TARGET_MIN_DB = -60;
export const TARGET_MAX_DB = 0;
const DEFAULT_TARGET_DB = -1;

interface DialogState {
  /** The raw text field value, so an invalid in-progress edit doesn't snap back. */
  text: string;
  valid: boolean;
}

let dialogOpen = $state(false);
let dialog = $state<DialogState>({ text: formatTarget(DEFAULT_TARGET_DB), valid: true });

/** Read-only accessor for components. */
export function normalizeState(): {
  readonly dialogOpen: boolean;
  readonly dialogText: string;
  readonly dialogValid: boolean;
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
  };
}

function formatTarget(db: number): string {
  return db.toFixed(2);
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
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

/** `true` when a normalize command can run right now (a document with audio is open). */
export function canNormalize(): boolean {
  return scope() !== null;
}

async function run(targetDb: number): Promise<void> {
  const range = scope();
  if (!range) {
    return;
  }
  try {
    const result = await editNormalizePeak(range[0], range[1], targetDb);
    setSelectionFromResult(result.selection);
  } catch (err) {
    if (isIpcError(err)) {
      pushNotice(noticeFromIpcError(err));
    }
  }
}

/** A favorite toolbar button / Favorites menu item (SPEC-010 §2.1: one click, no dialog). */
export const normalizeFavorite = (targetDb: number): Promise<void> => run(targetDb);

/** Parses the dialog's dB-mode text field (SPEC-010 §2.4: locale-neutral, `−` accepted as minus)
 * into a finite value within `[TARGET_MIN_DB, TARGET_MAX_DB]`, or `null`. */
export function parseTargetDb(text: string): number | null {
  const normalized = text.trim().replace(/−/g, "-");
  if (normalized === "") {
    return null;
  }
  const value = Number(normalized);
  if (!Number.isFinite(value) || value < TARGET_MIN_DB || value > TARGET_MAX_DB) {
    return null;
  }
  return value;
}

/** Effects → Normalize… / the Favorites menu's "Normalize…" (SPEC-010 §2.4). */
export function openNormalizeDialog(): void {
  if (!canNormalize()) {
    return;
  }
  dialog = { text: dialog.text, valid: parseTargetDb(dialog.text) !== null };
  dialogOpen = true;
}

export function closeNormalizeDialog(): void {
  dialogOpen = false;
}

export function setNormalizeDialogText(text: string): void {
  dialog = { text, valid: parseTargetDb(text) !== null };
}

/** Enter / the dialog's Apply button. A no-op while the field is invalid. */
export async function applyNormalizeDialog(): Promise<void> {
  const value = parseTargetDb(dialog.text);
  if (value === null) {
    return;
  }
  dialogOpen = false;
  await run(value);
}

/** Test/teardown helper. */
export function resetNormalizeForTest(): void {
  dialogOpen = false;
  dialog = { text: formatTarget(DEFAULT_TARGET_DB), valid: true };
}

/**
 * Wires the store. Normalize has no keymap actions (SPEC-010 §2.5: no default shortcuts) and no
 * events of its own — its notices arrive through the shared `notice` event
 * (`notices.svelte.ts`'s `initNotices`). Returns a no-op teardown, for symmetry with the other
 * `init*` feature modules `App.svelte` mounts.
 */
export function initNormalize(): () => void {
  return () => {};
}
