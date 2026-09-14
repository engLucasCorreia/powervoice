import {
  extendSelection,
  isEmptySelection,
  normalizeSelection,
  selectAll,
  type SelectionRange,
} from "../waveform/selection";

/**
 * Selection store (S2-01, SPEC-006 §2.2/§2.9): the current time selection, in document samples —
 * view/UI state, never part of the undo history (SPEC-004 §2.2), read fresh on open. The
 * waveform view drives this from pointer events; the Edit menu/keymap and the edit commands
 * (`state/edit.svelte.ts`) read it to build `edit_*` command arguments.
 *
 * T-304 (SPEC-022 §2.11): while a record operation runs, selection **gestures** are ignored so
 * the punch range can't be confused with a new selection; results the backend reports
 * (`setSelectionFromResult`) still apply.
 */

let selection = $state<SelectionRange | null>(null);
/** The fixed edge of an in-progress click-drag (SPEC-006 §2.9): the mousedown sample. */
let dragAnchorSample: number | null = null;
/** T-304: gestures are ignored (a record operation runs). */
let locked = false;

/** Read-only accessor for components. */
export function selectionState(): { readonly current: SelectionRange | null } {
  return {
    get current() {
      return selection;
    },
  };
}

/** `true` when there is no selection, or it's empty (SPEC-008 §2.2). */
export function hasSelection(): boolean {
  return !isEmptySelection(selection);
}

/** T-304 (SPEC-022 §2.11): locks (or unlocks) selection gestures while recording. */
export function setSelectionLocked(value: boolean): void {
  locked = value;
  if (value) {
    dragAnchorSample = null;
  }
}

/** Whether selection gestures are currently ignored. */
export function isSelectionLocked(): boolean {
  return locked;
}

/** Mousedown: starts a new click-drag anchored at `sample`, clearing any prior selection. */
export function beginDrag(sample: number): void {
  if (locked) {
    return;
  }
  dragAnchorSample = sample;
  selection = null;
}

/** Pointer move during a click-drag (SPEC-006 §2.9: live-updating, anchor-normalized). */
export function dragTo(sample: number): void {
  if (dragAnchorSample === null || locked) {
    return;
  }
  selection = normalizeSelection(dragAnchorSample, sample);
}

/** Mouseup: ends the current drag (the last `dragTo` already set the final selection). */
export function endDrag(): void {
  dragAnchorSample = null;
}

/** Shift+click at `sample`, with `cursorSample` the current edit cursor (SPEC-006 §2.9). */
export function shiftClickTo(sample: number, cursorSample: number): void {
  if (locked) {
    return;
  }
  selection = extendSelection(selection, sample, cursorSample);
  dragAnchorSample = null;
}

/** Ctrl+A / double-click: selects the entire document (SPEC-006 §2.9). */
export function selectAllOf(lenSamples: number): void {
  if (locked) {
    return;
  }
  selection = selectAll(lenSamples);
  dragAnchorSample = null;
}

/** Esc: clears the selection (the cursor/playhead is untouched, SPEC-006 §2.9). */
export function clearSelection(): void {
  if (locked) {
    return;
  }
  selection = null;
  dragAnchorSample = null;
}

/** Applies a selection an `edit_*`/undo/redo command reported (SPEC-008 §2.3's post-op table). */
export function setSelectionFromResult(range: [number, number] | null): void {
  selection = range ? { startSample: range[0], endSample: range[1] } : null;
  dragAnchorSample = null;
}

/** Test/teardown helper. */
export function resetSelectionForTest(): void {
  selection = null;
  dragAnchorSample = null;
  locked = false;
}
