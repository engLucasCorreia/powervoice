/**
 * Where a popover or menu goes (H-26), pure so it can be tested without layout: below its
 * anchor by default, flipped above when there's no room below, submenus to the right flipped to
 * the left, and always shifted back inside the viewport. When it fits on neither side it takes
 * the roomier one and gets a `maxHeight`, so its content scrolls instead of leaving the window.
 */
export type Placement = "bottom-start" | "bottom-end" | "top-start" | "top-end" | "right-start" | "left-start";

export interface AnchorRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface PlacedPopover {
  left: number;
  top: number;
  /** The side actually used after flipping. */
  placement: Placement;
  /** Set when the popover is taller than the room it has; its content should scroll. */
  maxHeight: number | null;
}

export interface PlacementOptions {
  /** Distance from the anchor. */
  gapPx?: number;
  /** Minimum distance from the viewport edge. */
  edgePx?: number;
  /** Vertical nudge for side placements (a submenu's first row lines up with its parent row). */
  alignOffsetPx?: number;
}

const DEFAULT_GAP_PX = 4;
const DEFAULT_EDGE_PX = 8;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

export function placePopover(
  anchor: AnchorRect,
  popup: Size,
  viewport: Size,
  placement: Placement,
  options: PlacementOptions = {},
): PlacedPopover {
  const gap = options.gapPx ?? DEFAULT_GAP_PX;
  const edge = options.edgePx ?? DEFAULT_EDGE_PX;
  const right = anchor.left + anchor.width;
  const bottom = anchor.top + anchor.height;

  if (placement === "right-start" || placement === "left-start") {
    const roomRight = viewport.width - edge - (right + gap);
    const roomLeft = anchor.left - gap - edge;
    let side: "right-start" | "left-start" = placement;
    if (side === "right-start" && popup.width > roomRight && roomLeft > roomRight) {
      side = "left-start";
    } else if (side === "left-start" && popup.width > roomLeft && roomRight > roomLeft) {
      side = "right-start";
    }
    const rawLeft = side === "right-start" ? right + gap : anchor.left - gap - popup.width;
    const left = clamp(rawLeft, edge, viewport.width - edge - popup.width);
    const available = viewport.height - 2 * edge;
    const maxHeight = popup.height > available ? available : null;
    const height = maxHeight ?? popup.height;
    const top = clamp(anchor.top + (options.alignOffsetPx ?? 0), edge, viewport.height - edge - height);
    return { left, top, placement: side, maxHeight };
  }

  const alignEnd = placement.endsWith("-end");
  const roomBelow = viewport.height - edge - (bottom + gap);
  const roomAbove = anchor.top - gap - edge;
  let below = placement.startsWith("bottom");
  if (below && popup.height > roomBelow && roomAbove > roomBelow) {
    below = false;
  } else if (!below && popup.height > roomAbove && roomBelow > roomAbove) {
    below = true;
  }
  const room = below ? roomBelow : roomAbove;
  const maxHeight = popup.height > room ? Math.max(0, room) : null;
  const height = maxHeight ?? popup.height;
  const top = below ? bottom + gap : anchor.top - gap - height;
  const rawLeft = alignEnd ? right - popup.width : anchor.left;
  const left = clamp(rawLeft, edge, viewport.width - edge - popup.width);
  const side = `${below ? "bottom" : "top"}-${alignEnd ? "end" : "start"}` as Placement;
  return { left, top, placement: side, maxHeight };
}
