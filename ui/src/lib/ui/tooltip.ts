/**
 * Tooltip timing + coordination (H-25), shared by every Tooltip instance:
 * - hover opens after `TOOLTIP_DELAY_MS`; moving to a neighbour within `TOOLTIP_SKIP_MS` of the
 *   previous tooltip closing opens instantly (scrubbing along a toolbar reads every label);
 * - only one tooltip is open at a time.
 * Uses `Date.now()` so Vitest fake timers drive it.
 */
export const TOOLTIP_DELAY_MS = 500;
export const TOOLTIP_SKIP_MS = 300;

let lastClosedAt = Number.NEGATIVE_INFINITY;
let activeClose: (() => void) | null = null;
let counter = 0;

export function nextTooltipId(): string {
  counter += 1;
  return `pv-tooltip-${counter}`;
}

export function openDelayMs(delayMs: number = TOOLTIP_DELAY_MS): number {
  return Date.now() - lastClosedAt < TOOLTIP_SKIP_MS ? 0 : delayMs;
}

/** Registers the tooltip now opening; closes whichever one was open. */
export function claimTooltip(close: () => void): void {
  if (activeClose && activeClose !== close) {
    activeClose();
  }
  activeClose = close;
}

export function releaseTooltip(close: () => void): void {
  if (activeClose === close) {
    activeClose = null;
  }
  lastClosedAt = Date.now();
}

export function resetTooltipsForTest(): void {
  lastClosedAt = Number.NEGATIVE_INFINITY;
  activeClose = null;
}

export interface TooltipPosition {
  left: number;
  top: number;
  placement: "top" | "bottom";
}

const GAP_PX = 6;
const EDGE_PX = 8;

/**
 * Places a tooltip of size `tip` centred on `anchor`, below it by default, flipping above when it
 * would leave the viewport, and clamping horizontally inside it. Pure (viewport passed in).
 */
export function placeTooltip(
  anchor: { left: number; top: number; width: number; height: number },
  tip: { width: number; height: number },
  viewport: { width: number; height: number },
  preferred: "top" | "bottom",
): TooltipPosition {
  const below = anchor.top + anchor.height + GAP_PX;
  const above = anchor.top - GAP_PX - tip.height;
  let placement = preferred;
  if (placement === "bottom" && below + tip.height > viewport.height - EDGE_PX && above >= EDGE_PX) {
    placement = "top";
  } else if (placement === "top" && above < EDGE_PX) {
    placement = "bottom";
  }
  const centre = anchor.left + anchor.width / 2;
  const maxLeft = Math.max(EDGE_PX, viewport.width - EDGE_PX - tip.width);
  const left = Math.min(maxLeft, Math.max(EDGE_PX, centre - tip.width / 2));
  return { left, top: placement === "bottom" ? below : above, placement };
}
