/**
 * Vertical (amplitude) zoom state math (H-35, SPEC-006 §2.4/§2.6): pure functions for the
 * `verticalZoom` linear scale factor consumed by `amplitudeAxis.ts`'s ruler tick math and
 * `coords.ts`'s `columnYRange` (both implement the same `y = centerY − amplitude × verticalZoom ×
 * halfHeightPx` formula, SPEC-006 §2.4). Mirrors `coords.ts`'s horizontal zoom-step math
 * (`clampSamplesPerPixel`/`zoomStep`), but power-of-two steps instead of horizontal's `√2`
 * (SPEC-006 §2.4: "power-of-two steps by keyboard/menu, continuous by drag on the ruler gutter" —
 * only the keyboard/menu step is in this ticket's scope; drag-to-zoom the amplitude gutter is a
 * later ticket, same as `amplitudeAxis.ts`'s own "out of scope" note).
 */

/** SPEC-006 §2.4: "`verticalZoom` ranges 1× to 256×". */
export const MIN_VERTICAL_ZOOM = 1;
export const MAX_VERTICAL_ZOOM = 256;

/** SPEC-006 §2.4: "default 1× (±1.0 spans the full waveform canvas height)". */
export const DEFAULT_VERTICAL_ZOOM = 1;

/** SPEC-006 §2.4: "power-of-two steps by keyboard/menu". */
export const VERTICAL_ZOOM_STEP_FACTOR = 2;

/** Clamps `zoom` into the valid SPEC-006 §2.4 range, `[1, 256]`. A non-finite input (shouldn't
 * happen from any caller in this codebase, but keeps this function total) falls back to the
 * default instead of propagating `NaN`/`Infinity` into the renderer. */
export function clampVerticalZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) {
    return DEFAULT_VERTICAL_ZOOM;
  }
  return Math.min(Math.max(zoom, MIN_VERTICAL_ZOOM), MAX_VERTICAL_ZOOM);
}

/**
 * The next `verticalZoom` after one step — one `Alt+=`/`Alt+-` keypress, or one `Alt+wheel` notch
 * (SPEC-006 §2.6) — clamped to `[1, 256]`. `direction`: `1` zooms in (larger, quieter signal fills
 * more of the canvas height), `-1` zooms out (smaller, back toward 1×).
 */
export function verticalZoomStep(current: number, direction: 1 | -1): number {
  const next =
    direction === 1 ? current * VERTICAL_ZOOM_STEP_FACTOR : current / VERTICAL_ZOOM_STEP_FACTOR;
  return clampVerticalZoom(next);
}
