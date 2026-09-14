/**
 * Canvas-free layout math for the app shell's resizable regions (H-24): column widths (Markers |
 * editor | Rack), the bottom dock's height, and the draggable splitters that resize them. Kept
 * out of `App.svelte` so it's unit-testable under jsdom (MEMORY.md: jsdom has no `ResizeObserver`
 * and stubs `clientWidth`/`clientHeight` at 0 — none of that matters here since these are pure
 * number functions).
 */

/** Keyboard step size for an arrow-key press on a splitter (H-24 ticket: "keyboard arrows move"). */
export const SPLITTER_KEYBOARD_STEP_PX = 16;

/** Clamps `value` into `[min, max]` (degrades gracefully if `max < min`, e.g. a tiny window). */
export function clampSize(value: number, min: number, max: number): number {
  const lo = Math.min(min, max);
  const hi = Math.max(min, max);
  return Math.min(Math.max(value, lo), hi);
}

/**
 * The new size of a side column after dragging its splitter by `deltaPx` (H-24 item 2: "draggable
 * vertical splitters"). `reverse` is `true` for a splitter on the *right* edge of its column (the
 * Rack column's left splitter), where dragging left (negative delta) grows the column.
 */
export function resizeColumnPx(
  startSizePx: number,
  deltaPx: number,
  reverse: boolean,
  minPx: number,
  maxPx: number,
): number {
  const signed = reverse ? -deltaPx : deltaPx;
  return clampSize(startSizePx + signed, minPx, maxPx);
}

/**
 * The next size after one keyboard arrow-key step (H-24: "keyboard arrows move"). `direction`:
 * `1` grows the region, `-1` shrinks it.
 */
export function stepSizePx(
  current: number,
  direction: 1 | -1,
  minPx: number,
  maxPx: number,
  stepPx: number = SPLITTER_KEYBOARD_STEP_PX,
): number {
  return clampSize(current + direction * stepPx, minPx, maxPx);
}

/** H-24 item 1: the bottom dock is `[120, 60 % of the main area]`, and the workspace above it
 * never drops below 40 % — since `60 % + 40 % = 100 %`, capping the dock at `60%` of the total
 * height is sufficient to guarantee the workspace's floor without a separate check. Degrades
 * gracefully (returns `minDockPx` clamped to what little height exists) for a pathologically
 * short window rather than producing a negative or NaN size. */
export function clampDockHeightPx(
  totalHeightPx: number,
  desiredDockHeightPx: number,
  minDockPx = 120,
  maxDockFraction = 0.6,
): number {
  if (!(totalHeightPx > 0)) {
    return Math.max(0, desiredDockHeightPx);
  }
  const maxByFraction = totalHeightPx * maxDockFraction;
  const max = Math.max(0, maxByFraction);
  const min = Math.min(minDockPx, max);
  return clampSize(desiredDockHeightPx, min, max);
}

/** H-24 item 2: a column's width is clamped to `[minPx, maxAvailablePx]` — `maxAvailablePx` comes
 * from the caller (usually most of the window width, leaving room for the other regions). */
export function clampColumnWidthPx(widthPx: number, minPx: number, maxAvailablePx: number): number {
  return clampSize(widthPx, minPx, Math.max(minPx, maxAvailablePx));
}
