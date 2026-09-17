/**
 * Node-drag pixel math (S3-07, SPEC-015 §2.6.4): "the node follows the pointer at once"; Shift
 * scales the pointer delta by 0.1 ("fine"). Pure so it's testable without a DOM.
 */

/** The node's pixel position when the drag started. */
export interface DragStart {
  /** x position on the frequency axis. */
  freqPx: number;
  /** y position on the gain axis; `null` for HP/LP, which move horizontally only. */
  gainPx: number | null;
}

/** The node's new pixel position for a pointer that has moved `(deltaX, deltaY)` since the drag
 * started, applying the ×0.1 fine scale when `fine` is true (SPEC-015 §2.6.4 "Shift = fine"). */
export function dragPosition(
  start: DragStart,
  deltaX: number,
  deltaY: number,
  fine: boolean,
): { freqPx: number; gainPx: number | null } {
  const k = fine ? 0.1 : 1;
  return {
    freqPx: start.freqPx + deltaX * k,
    gainPx: start.gainPx === null ? null : start.gainPx + deltaY * k,
  };
}

/** `1` for a wheel notch up (`deltaY < 0`), `-1` down, `0` for no movement — SPEC-015 §2.6.4
 * "wheel over a node". Shared by the Q factor below and the HP/LP slope step (H-86), so both
 * agree on which way is "up". */
export function wheelNotches(deltaY: number): -1 | 0 | 1 {
  return deltaY > 0 ? -1 : deltaY < 0 ? 1 : 0;
}

/** `2^(±1/6)` per wheel notch (Shift: `2^(±1/24)`), SPEC-015 §2.6.4 "wheel over a node". */
export function wheelQFactor(deltaY: number, fine: boolean): number {
  const notches = wheelNotches(deltaY);
  const exponent = fine ? 1 / 24 : 1 / 6;
  return 2 ** (notches * exponent);
}
