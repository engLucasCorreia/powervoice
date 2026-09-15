import { listen } from "@tauri-apps/api/event";
import type { ClipboardChangedDto, EditResultDto, EditTargetDto, EventName, IpcError, HistoryStateDto } from "../ipc/bindings";
import {
  editCopy,
  editCut,
  editDelete,
  editPaste,
  editSilence,
  editTrim,
  historyRedo,
  historyUndo,
} from "../ipc/commands";
import { registerAction } from "../shortcuts";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";
import { pushNotice } from "./notices.svelte";
import { transportState } from "./transport.svelte";

/**
 * Edit commands store (S2-01, SPEC-008): cut/copy/paste/delete/trim/silence and undo/redo, the
 * Edit menu's history state (`history_state` event) and the clipboard's availability
 * (`clipboard_changed` event). Registers the Edit keymap actions (Ctrl+X/C/V, Delete, Ctrl+T,
 * Ctrl+Z, Ctrl+Shift+Z). Selection lives in `selection.svelte.ts`; edit results feed back into it
 * (SPEC-008 §2.3's post-op selection/cursor table) — `document_changed` (already wired by
 * `document.svelte.ts`) covers `audio_rev`/`len_samples`/dirty.
 */

const IDLE_HISTORY: HistoryStateDto = {
  can_undo: false,
  can_redo: false,
  undo_label: null,
  redo_label: null,
  undo_label_params: {},
  redo_label_params: {},
};

const EMPTY_CLIPBOARD: ClipboardChangedDto = {
  len_samples: null,
  sample_rate_hz: null,
};

let history = $state<HistoryStateDto>({ ...IDLE_HISTORY });
let clipboard = $state<ClipboardChangedDto>({ ...EMPTY_CLIPBOARD });

/** Read-only accessor for components (the Edit menu). */
export function editState(): {
  readonly history: HistoryStateDto;
  readonly clipboard: ClipboardChangedDto;
} {
  return {
    get history() {
      return history;
    },
    get clipboard() {
      return clipboard;
    },
  };
}

/** `true` once something has been cut or copied (enables Paste, SPEC-008 §2.2). */
export function hasClipboard(): boolean {
  return clipboard.len_samples !== null;
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

function applyResult(result: EditResultDto): void {
  setSelectionFromResult(result.selection);
}

async function run(command: () => Promise<EditResultDto>): Promise<void> {
  try {
    applyResult(await command());
  } catch (err) {
    report(err);
  }
}

/** `[start, end)` of the current non-empty selection, or `null` (SPEC-008 §2.2). */
function selectedRange(): [number, number] | null {
  return hasSelection()
    ? [selectionState().current!.startSample, selectionState().current!.endSample]
    : null;
}

/** Cut, Copy, Delete, Trim and Silence are no-ops with no (or an empty) selection (SPEC-008 §2.2)
 * — the Edit menu/keymap disable them, but a direct call is also a safe no-op. */
function withSelection(op: (start: number, end: number) => Promise<EditResultDto>): Promise<void> {
  const range = selectedRange();
  return range ? run(() => op(range[0], range[1])) : Promise.resolve();
}

export const cut = (): Promise<void> => withSelection(editCut);
export const copy = (): Promise<void> => withSelection(editCopy);
export const deleteSelection = (): Promise<void> => withSelection(editDelete);
export const trim = (): Promise<void> => withSelection(editTrim);
export const silence = (): Promise<void> => withSelection(editSilence);

/** Paste at the cursor (no selection) or over the current selection (SPEC-008 §2.1). A no-op
 * with an empty clipboard. */
export function paste(): Promise<void> {
  if (!hasClipboard()) {
    return Promise.resolve();
  }
  const range = selectedRange();
  const target: EditTargetDto = range
    ? { kind: "range", start_samples: range[0], end_samples: range[1] }
    : { kind: "cursor", at_samples: transportState().playheadSamples };
  return run(() => editPaste(target));
}

export const undo = (): Promise<void> => run(historyUndo);
export const redo = (): Promise<void> => run(historyRedo);

/**
 * Wires the store: keymap actions and the `history_state`/`clipboard_changed` events. Returns
 * the teardown.
 */
export async function initEdit(): Promise<() => void> {
  const cleanups: Array<() => void> = [
    registerAction("edit.cut", () => void cut()),
    registerAction("edit.copy", () => void copy()),
    registerAction("edit.paste", () => void paste()),
    registerAction("edit.delete", () => void deleteSelection()),
    registerAction("edit.trim", () => void trim()),
    registerAction("history.undo", () => void undo()),
    registerAction("history.redo", () => void redo()),
  ];
  try {
    const unlisten = await listen<HistoryStateDto>("history_state" satisfies EventName, (e) => {
      history = e.payload;
    });
    cleanups.push(unlisten);
  } catch {
    // Without the event, history state only follows command results (still functional).
  }
  try {
    const unlisten = await listen<ClipboardChangedDto>(
      "clipboard_changed" satisfies EventName,
      (e) => {
        clipboard = e.payload;
      },
    );
    cleanups.push(unlisten);
  } catch {
    // Same fallback as above.
  }
  return () => {
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
export function resetEditForTest(): void {
  history = { ...IDLE_HISTORY };
  clipboard = { ...EMPTY_CLIPBOARD };
}
