/**
 * H-115 ticket §1: the modal's size. The shared `Dialog` kit's `xl` size is a fixed 880×min(680,
 * 85vh) px box (H-102 already overrode the height once, to `min(840px, 90vh)`) — a *pixel* cap,
 * which is why the owner's 3756×2121 display showed "a small box in the middle of the screen":
 * 880 px is a small fraction of a 4K-class monitor no matter what the height does.
 *
 * The fix is to size from the viewport instead of a pixel ceiling: `max(floor, vw/vh)` only ever
 * grows past the floor, never gets capped by one — big by default on a large display, and usable
 * on a laptop because of the floor. `maximised` pushes both dimensions to nearly the full
 * viewport (H-115's "maximise / full-screen control").
 *
 * Returned as CSS `width`/`height` (and matching `max-width`/`max-height`, since `Dialog.svelte`'s
 * own base rule caps `.pv-dialog` at `90vw`/`85vh` — without overriding those too, a size above
 * that base cap would be clipped straight back down to it).
 */
export interface ExplainDialogSize {
  width: string;
  height: string;
}

const DEFAULT_WIDTH_FLOOR_PX = 880;
const DEFAULT_HEIGHT_FLOOR_PX = 600;
const DEFAULT_WIDTH_VW = 88;
const DEFAULT_HEIGHT_VH = 86;
const MAXIMIZED_VW = 98;
const MAXIMIZED_VH = 96;

export function explainDialogSize(maximized: boolean): ExplainDialogSize {
  if (maximized) {
    return { width: `${MAXIMIZED_VW}vw`, height: `${MAXIMIZED_VH}vh` };
  }
  return {
    width: `max(${DEFAULT_WIDTH_FLOOR_PX}px, ${DEFAULT_WIDTH_VW}vw)`,
    height: `max(${DEFAULT_HEIGHT_FLOOR_PX}px, ${DEFAULT_HEIGHT_VH}vh)`,
  };
}

/** `Dialog`'s `style` prop wants one CSS-text string; this is the one seam between the pure sizes
 * above and the template. */
export function explainDialogStyle(maximized: boolean): string {
  const size = explainDialogSize(maximized);
  return `width: ${size.width}; max-width: ${size.width}; height: ${size.height}; max-height: ${size.height};`;
}
