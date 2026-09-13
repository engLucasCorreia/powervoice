import type { IpcError } from "../ipc/bindings";
import { editNormalizeLufs } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "./notices.svelte";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";

/**
 * LUFS normalize store (S4-01, PROMPT §3.3): the three integrated-loudness favorites and the
 * Normalize (LUFS)… dialog. Same scope convention as peak normalize (`normalize.svelte.ts`): the
 * current non-empty selection, or the whole file when there is none. A no-op (silent scope,
 * already at the target) or an applied gain whose predicted true peak exceeds −1 dBTP is reported
 * by the backend as a `notice` event (`notices.svelte.ts`'s `initNotices`), not by this module.
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

let dialogOpen = $state(false);
let dialog = $state<DialogState>({ text: formatTarget(DEFAULT_TARGET_LUFS), valid: true });

/** Read-only accessor for components. */
export function normalizeLufsState(): {
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

function formatTarget(lufs: number): string {
  return lufs.toFixed(1);
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
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

/** `true` when a LUFS normalize command can run right now (a document with audio is open). */
export function canNormalizeLufs(): boolean {
  return scope() !== null;
}

async function run(targetLufs: number): Promise<void> {
  const range = scope();
  if (!range) {
    return;
  }
  try {
    const result = await editNormalizeLufs(range[0], range[1], targetLufs);
    setSelectionFromResult(result.selection);
  } catch (err) {
    if (isIpcError(err)) {
      pushNotice(noticeFromIpcError(err));
    }
  }
}

/** A favorite toolbar button / Favorites menu item (one click, no dialog). */
export const normalizeLufsFavorite = (targetLufs: number): Promise<void> => run(targetLufs);

/** Parses the dialog's LUFS text field (locale-neutral, `−` accepted as minus) into a finite
 * value within `[TARGET_MIN_LUFS, TARGET_MAX_LUFS]`, or `null`. */
export function parseTargetLufs(text: string): number | null {
  const normalized = text.trim().replace(/−/g, "-");
  if (normalized === "") {
    return null;
  }
  const value = Number(normalized);
  if (!Number.isFinite(value) || value < TARGET_MIN_LUFS || value > TARGET_MAX_LUFS) {
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

/** Test/teardown helper. */
export function resetNormalizeLufsForTest(): void {
  dialogOpen = false;
  dialog = { text: formatTarget(DEFAULT_TARGET_LUFS), valid: true };
}

/**
 * Wires the store. LUFS normalize has no keymap actions and no events of its own — its notices
 * arrive through the shared `notice` event. Returns a no-op teardown, for symmetry with the other
 * `init*` feature modules `App.svelte` mounts.
 */
export function initNormalizeLufs(): () => void {
  return () => {};
}
