/**
 * Where the tour's spotlight and step card go (T-709), pure so it is tested without layout.
 *
 * The spotlight is the target grown by a little padding and clipped to the window. The card goes
 * on the step's preferred side of it, centred on the target, and moves to the opposite side, then
 * to the two perpendicular sides, when it doesn't fit; the vertical/horizontal maths (flip, shift
 * back inside the window, scroll when too tall) is H-26's `placePopover`. A target that leaves no
 * room on any side (the editor fills most of the window) gets the card inside its bottom-right
 * corner; no target at all gets a centred card.
 */
import { placePopover, type AnchorRect, type Size } from "../ui/placement";

export type TourPlacement = "bottom" | "top" | "right" | "left";
export type ResolvedTourPlacement = TourPlacement | "inside" | "center";

export interface PlacedCard {
  left: number;
  top: number;
  placement: ResolvedTourPlacement;
  /** Set when the card is taller than the room it has; its content scrolls. */
  maxHeight: number | null;
  /** Where the card's pointer sits along the edge facing the target (px from the card's
   * left/top edge), or `null` when the card doesn't sit beside the target. */
  arrowPx: number | null;
}

export interface TourPlacementOptions {
  /** Distance between the spotlight and the card. */
  gapPx?: number;
  /** Minimum distance from the window edge. */
  edgePx?: number;
}

/** Padding around the target inside the spotlight ring. */
export const SPOTLIGHT_PAD_PX = 6;
const DEFAULT_GAP_PX = 14;
const DEFAULT_EDGE_PX = 12;
/** The pointer never sits closer than this to a card corner (the corner radius plus room). */
const ARROW_INSET_PX = 20;

/** Used until the card has been laid out once (and in tests, where nothing has a size). */
export const CARD_FALLBACK_SIZE: Size = { width: 352, height: 208 };

const FALLBACK_ORDER: Record<TourPlacement, readonly TourPlacement[]> = {
  bottom: ["bottom", "top", "right", "left"],
  top: ["top", "bottom", "right", "left"],
  right: ["right", "left", "bottom", "top"],
  left: ["left", "right", "bottom", "top"],
};

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

/**
 * The cut-out around `target`: grown by `padPx`, clipped to the window. `null` when no part of the
 * target is on screen.
 */
export function spotlightRect(target: AnchorRect, viewport: Size, padPx = SPOTLIGHT_PAD_PX): AnchorRect | null {
  const left = Math.max(0, target.left - padPx);
  const top = Math.max(0, target.top - padPx);
  const right = Math.min(viewport.width, target.left + target.width + padPx);
  const bottom = Math.min(viewport.height, target.top + target.height + padPx);
  if (right - left <= 0 || bottom - top <= 0) {
    return null;
  }
  return { left, top, width: right - left, height: bottom - top };
}

function fits(side: TourPlacement, spot: AnchorRect, card: Size, viewport: Size, gap: number, edge: number): boolean {
  switch (side) {
    case "bottom":
      return viewport.height - edge - (spot.top + spot.height + gap) >= card.height;
    case "top":
      return spot.top - gap - edge >= card.height;
    case "right":
      return viewport.width - edge - (spot.left + spot.width + gap) >= card.width;
    case "left":
      return spot.left - gap - edge >= card.width;
  }
}

/** Places the step card next to `spot` (or centred when there is no target). */
export function placeTourCard(
  spot: AnchorRect | null,
  card: Size,
  viewport: Size,
  preferred: TourPlacement = "bottom",
  options: TourPlacementOptions = {},
): PlacedCard {
  const gap = options.gapPx ?? DEFAULT_GAP_PX;
  const edge = options.edgePx ?? DEFAULT_EDGE_PX;
  const available = viewport.height - 2 * edge;
  const tallMaxHeight = card.height > available ? Math.max(0, available) : null;

  if (!spot) {
    const height = tallMaxHeight ?? card.height;
    return {
      left: clamp((viewport.width - card.width) / 2, edge, viewport.width - edge - card.width),
      top: clamp((viewport.height - height) / 2, edge, viewport.height - edge - height),
      placement: "center",
      maxHeight: tallMaxHeight,
      arrowPx: null,
    };
  }

  const side = FALLBACK_ORDER[preferred].find((s) => fits(s, spot, card, viewport, gap, edge));

  if (side === undefined) {
    const height = tallMaxHeight ?? card.height;
    return {
      left: clamp(spot.left + spot.width - edge - card.width, edge, viewport.width - edge - card.width),
      top: clamp(spot.top + spot.height - edge - height, edge, viewport.height - edge - height),
      placement: "inside",
      maxHeight: tallMaxHeight,
      arrowPx: null,
    };
  }

  if (side === "bottom" || side === "top") {
    // An anchor as wide as the card and centred on the target, placed "-start": the card is
    // centred on the target, then shifted back inside the window by `placePopover`.
    const anchor: AnchorRect = {
      left: spot.left + spot.width / 2 - card.width / 2,
      top: spot.top,
      width: card.width,
      height: spot.height,
    };
    const placed = placePopover(anchor, card, viewport, side === "bottom" ? "bottom-start" : "top-start", {
      gapPx: gap,
      edgePx: edge,
    });
    const arrow = clamp(spot.left + spot.width / 2 - placed.left, ARROW_INSET_PX, card.width - ARROW_INSET_PX);
    return { left: placed.left, top: placed.top, placement: side, maxHeight: placed.maxHeight, arrowPx: arrow };
  }

  const placed = placePopover(spot, card, viewport, side === "right" ? "right-start" : "left-start", {
    gapPx: gap,
    edgePx: edge,
    alignOffsetPx: spot.height / 2 - card.height / 2,
  });
  const height = placed.maxHeight ?? card.height;
  const arrow = clamp(spot.top + spot.height / 2 - placed.top, ARROW_INSET_PX, height - ARROW_INSET_PX);
  return { left: placed.left, top: placed.top, placement: side, maxHeight: placed.maxHeight, arrowPx: arrow };
}
