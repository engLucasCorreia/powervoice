/**
 * Time-selection math (SPEC-006 §2.9): pure functions over document samples, shared by pointer
 * handling and tests. The selection itself is stored in document samples and is never re-derived
 * from pixels after creation (SPEC-006 AC-7) — only `pixelAtSample`/`sampleAtPixel` (`coords.ts`)
 * convert it to/from pixels for display and hit-testing.
 */

export interface SelectionRange {
  startSample: number;
  endSample: number;
}

/**
 * Normalizes a click-drag from `a` to `b` (either order) into `[start, end)` with `start < end`
 * (SPEC-006 §2.9: "dragging past [the anchor] in either direction is allowed, the selection is
 * always normalized"). An empty drag (`a === b`) yields no selection (`null`), matching a plain
 * click.
 */
export function normalizeSelection(a: number, b: number): SelectionRange | null {
  const start = Math.min(a, b);
  const end = Math.max(a, b);
  return start < end ? { startSample: start, endSample: end } : null;
}

/**
 * Shift+click (SPEC-006 §2.9): extends the existing selection's far edge (the edge farther from
 * `sample`) out to `sample` — i.e. the far edge is the anchor, and the selection is redrawn
 * between it and `sample`. With no existing selection, behaves like a plain click-drag anchored
 * at `cursorSample` (the current edit cursor / playhead).
 */
export function extendSelection(
  current: SelectionRange | null,
  sample: number,
  cursorSample: number,
): SelectionRange | null {
  if (!current) {
    return normalizeSelection(cursorSample, sample);
  }
  const distanceToStart = Math.abs(sample - current.startSample);
  const distanceToEnd = Math.abs(sample - current.endSample);
  const anchor = distanceToStart > distanceToEnd ? current.startSample : current.endSample;
  return normalizeSelection(anchor, sample);
}

/** Ctrl+A / double-click (SPEC-006 §2.9): selects `[0, lenSamples)` exactly; `null` for an empty
 * document. */
export function selectAll(lenSamples: number): SelectionRange | null {
  return lenSamples > 0 ? { startSample: 0, endSample: lenSamples } : null;
}

/** `true` when `selection` is `null` or empty (SPEC-008 §2.2: "no selection" includes an empty
 * one — `S === E` counts as the cursor at `S`). */
export function isEmptySelection(selection: SelectionRange | null): boolean {
  return selection === null || selection.startSample === selection.endSample;
}

/** SPEC-006 §2.9/§3 `selection_handle_hit_px`: a selection boundary's grab-handle hit width. */
export const SELECTION_HANDLE_HIT_PX = 6;

/**
 * Hit-tests a pointer at device pixel `px` against a selection's two boundary handles (SPEC-006
 * §2.9, AC-8: "a handle's hit target is exactly `selection_handle_hit_px` wide, centered on the
 * boundary pixel"). Returns which edge was hit, or `null`. When both handles' hit zones overlap
 * (a very narrow/zoomed-out selection) the nearer one wins, `"start"` on an exact tie.
 */
export function hitTestHandle(
  px: number,
  startPx: number,
  endPx: number,
  hitPx: number = SELECTION_HANDLE_HIT_PX,
): "start" | "end" | null {
  const half = hitPx / 2;
  const distStart = Math.abs(px - startPx);
  const distEnd = Math.abs(px - endPx);
  const hitStart = distStart <= half;
  const hitEnd = distEnd <= half;
  if (hitStart && (!hitEnd || distStart <= distEnd)) {
    return "start";
  }
  if (hitEnd) {
    return "end";
  }
  return null;
}
