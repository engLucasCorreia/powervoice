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
/** T-206: the *other* edge during an in-progress handle drag (SPEC-006 §2.9's grab handles) —
 * distinct from `dragAnchorSample` (a plain click-drag has no existing selection yet). */
let handleFixedSample: number | null = null;
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
    handleFixedSample = null;
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

/** Mousedown on a selection handle (SPEC-006 §2.9): starts a drag of one boundary, keeping the
 * opposite boundary (`fixedSample`) fixed. */
export function beginHandleDrag(fixedSample: number): void {
  if (locked) {
    return;
  }
  handleFixedSample = fixedSample;
  dragAnchorSample = null;
}

/** `true` while a handle drag (as opposed to a plain click-drag) is in progress. */
export function isHandleDragging(): boolean {
  return handleFixedSample !== null;
}

/** Pointer move during a handle drag: live-updates the selection, keeping the fixed edge in
 * place. Reuses {@link normalizeSelection}, so dragging past the fixed edge naturally swaps which
 * edge is "start" (SPEC-006 AC-8). */
export function handleDragTo(sample: number): void {
  if (handleFixedSample === null || locked) {
    return;
  }
  selection = normalizeSelection(handleFixedSample, sample);
}

/** Mouseup: ends the handle drag, returning the fixed edge it was anchored to (`null` if no
 * handle drag was in progress) so the caller can apply a final (possibly snapped) position
 * against it. */
export function endHandleDrag(): number | null {
  const fixed = handleFixedSample;
  handleFixedSample = null;
  return fixed;
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
  handleFixedSample = null;
  locked = false;
}
