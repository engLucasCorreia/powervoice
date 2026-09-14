/**
 * H-25 (owner report after H-24: "the Rack column is not visible at this window width"): the
 * Markers | editor | Rack row must always fit its container. Given the measured main-area width
 * and the panels' requested widths (0 = collapsed), returns the widths to render:
 * 1. requested widths if the editor keeps `editorMinPx`;
 * 2. else both panels shrink proportionally;
 * 3. if that would take a panel below `usableMinPx`, Markers drops out first (the Rack holds the
 *    effect chain — more important while working), then the Rack narrows to what's left and
 *    finally hides (the editor never gives up its minimum). Pure; persisted preferences are untouched, so widening the window restores them.
 */
export interface FitInput {
  mainPx: number;
  markersPx: number;
  rackPx: number;
  editorMinPx?: number;
  splitterPx?: number;
  usableMinPx?: number;
}

export interface FitResult {
  markersPx: number;
  rackPx: number;
}

export function fitSideColumns({
  mainPx,
  markersPx,
  rackPx,
  editorMinPx = 360,
  splitterPx = 12,
  usableMinPx = 160,
}: FitInput): FitResult {
  if (mainPx <= 0) {
    return { markersPx, rackPx };
  }
  const available = mainPx - splitterPx - editorMinPx;
  const requested = markersPx + rackPx;
  if (requested <= available) {
    return { markersPx, rackPx };
  }
  if (requested > 0 && available > 0) {
    const factor = available / requested;
    const m = Math.floor(markersPx * factor);
    const r = Math.floor(rackPx * factor);
    if ((markersPx === 0 || m >= usableMinPx) && (rackPx === 0 || r >= usableMinPx)) {
      return { markersPx: m, rackPx: r };
    }
  }
  // Too narrow for both: Markers goes first.
  if (rackPx === 0) {
    return { markersPx: available >= usableMinPx ? Math.min(markersPx, available) : 0, rackPx: 0 };
  }
  // What's left for the Rack alone; below `usableMinPx` it hides rather than squeezing the editor.
  return { markersPx: 0, rackPx: available >= usableMinPx ? Math.min(rackPx, available) : 0 };
}
