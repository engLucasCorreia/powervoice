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

/**
 * T-701/A-020: Left/Right Arrow — moves the whole selection by `deltaSamples` (negative = left),
 * keeping its length exactly unchanged, clamped to `[0, lenSamples)` (the clamp shortens the
 * *movement*, not the selection, by capping `deltaSamples` at whichever edge would otherwise run
 * past the document — SPEC-006 §4.1's "the document" bound). Only meaningful for a non-empty
 * `current`; callers with no selection nudge the cursor/playhead instead (SPEC-003's `seek`).
 */
export function nudgeSelectionRange(
  current: SelectionRange,
  deltaSamples: number,
  lenSamples: number,
): SelectionRange {
  let delta = deltaSamples;
  if (delta < 0) {
    delta = Math.max(delta, -current.startSample);
  } else if (delta > 0) {
    delta = Math.min(delta, lenSamples - current.endSample);
  }
  return { startSample: current.startSample + delta, endSample: current.endSample + delta };
}

/**
 * T-701/A-020: Shift+Left/Right Arrow — extends the selection by `stepSamples` from the edge in
 * `direction` (`1` = right/end edge, `-1` = left/start edge), clamped to `[0, lenSamples]`. This
 * only ever grows the selection (the ticket's own example, "Shift+arrow extends" — no Audition
 * default was found to confirm or contradict this, see `shortcuts/actions.ts`), never shrinks it:
 * Shift+Right always pushes the end edge further right, Shift+Left always pushes the start edge
 * further left. With no current selection (or an empty one), starts a new one between
 * `cursorSample` (the anchor — unaffected by a later zero-crossing snap of the *moved* edge, same
 * as `extendSelection`'s no-current-selection branch above) and one step in `direction`.
 */
export function extendSelectionEdge(
  current: SelectionRange | null,
  cursorSample: number,
  direction: 1 | -1,
  stepSamples: number,
  lenSamples: number,
): SelectionRange | null {
  if (!current || isEmptySelection(current)) {
    const target = Math.max(0, Math.min(cursorSample + direction * stepSamples, lenSamples));
    return normalizeSelection(cursorSample, target);
  }
  if (direction === 1) {
    return { startSample: current.startSample, endSample: Math.min(lenSamples, current.endSample + stepSamples) };
  }
  return { startSample: Math.max(0, current.startSample - stepSamples), endSample: current.endSample };
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
