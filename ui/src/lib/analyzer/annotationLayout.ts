/**
 * Collision-aware annotation layout for the "Explain My Voice" graph (H-93, in the spirit of
 * `ui/axisLabels.ts` and `analyzer/peakLabels.ts`, extended to arbitrary-sized cards that may
 * move well away from their marker). Pure geometry: no Svelte, no IPC, no drawing.
 *
 * The inviolable rule: a label may move, an **anchor never does**. Every anchor here is already a
 * plot-pixel position (the caller maps frequency/level through the shared axis helpers before
 * building items) — this module only decides where the label *box* goes and where its leader line
 * attaches, never what the anchor coordinate is.
 *
 * Placement is a deterministic, priority-ordered greedy search: for each item, in priority order,
 * try a fixed, expanding sequence of candidate boxes around its anchor (above/below/right/left, at
 * increasing distance and sideways shift) and take the first one that is inside the plot rect,
 * clear of every hard-reserved rect, clear of every label already placed, and — preferably — clear
 * of the "avoid" rects (significant curve peaks) too. An item that fits nowhere is dropped; the
 * caller is told which, in priority order, so it can show them in a side panel instead.
 */
import { rectsOverlap, type Rect } from "../ui/axisLabels";

export interface AnnotationAnchor {
  /** Anchor position in plot px. This is the real measured point — it never moves. */
  x: number;
  y: number;
}

export interface AnnotationItem {
  /** Stable id, so callers (and tests) can match input items to placed/dropped output. */
  id: string;
  anchor: AnnotationAnchor;
  /** Label box size, px. */
  width: number;
  height: number;
  /** Lower number = more important. Ties keep the input order. */
  priority: number;
}

export interface LeaderLine {
  /** Where the leader line touches the label box (the box's point nearest the anchor). */
  from: { x: number; y: number };
  /** The anchor point, unchanged from `item.anchor`. */
  to: { x: number; y: number };
}

export interface PlacedAnnotation<T extends AnnotationItem = AnnotationItem> {
  item: T;
  rect: Rect;
  leader: LeaderLine;
}

export interface AnnotationLayoutOptions {
  /** Plot rectangle every label box and leader-line attachment point must stay within. */
  rect: Rect;
  /** Rects to avoid when there is a free choice (e.g. the curve's significant peaks). Soft. */
  avoid?: readonly Rect[];
  /** Rects no label may ever touch (e.g. a legend, the hover readout). Hard. */
  reserved?: readonly Rect[];
  /** Minimum clear space between two label boxes, px (default 4). */
  spacingPx?: number;
  /** Inset from the plot rect's edges that labels may not enter, px (default 0). */
  edgeGapPx?: number;
  /** Clear space kept between an anchor and its own label box, px (default 10). */
  anchorGapPx?: number;
  /** Maximum number of labels attempted; the rest are reported as dropped (default 7). */
  maxLabels?: number;
}

export interface AnnotationLayoutResult<T extends AnnotationItem = AnnotationItem> {
  /** Successfully placed labels, in the priority order they were placed. */
  placed: PlacedAnnotation<T>[];
  /** Items that did not make the cap, or could not be placed, in priority order. */
  dropped: T[];
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi);
}

function insideRect(r: Rect, bounds: Rect): boolean {
  return (
    r.x >= bounds.x - 1e-6 &&
    r.y >= bounds.y - 1e-6 &&
    r.x + r.width <= bounds.x + bounds.width + 1e-6 &&
    r.y + r.height <= bounds.y + bounds.height + 1e-6
  );
}

/** The point on (or in) `rect` closest to `anchor`: where the leader line attaches. */
function attachmentPoint(rect: Rect, anchor: AnnotationAnchor): { x: number; y: number } {
  return {
    x: clamp(anchor.x, rect.x, rect.x + rect.width),
    y: clamp(anchor.y, rect.y, rect.y + rect.height),
  };
}

function leaderFor(anchor: AnnotationAnchor, rect: Rect): LeaderLine {
  return { from: attachmentPoint(rect, anchor), to: { x: anchor.x, y: anchor.y } };
}

const TIERS = [0, 1, 2, 3, 4, 5, 6, 7];
const SHIFTS = [0, 1, -1, 2, -2, 3, -3, 4, -4];

/**
 * Candidate boxes around `anchor`, nearest first: above and below at increasing distance (tiers)
 * and sideways shift, then right and left the same way. Closest, least-shifted spots come first,
 * so a label prefers sitting close and centred over its anchor and only drifts further when it
 * must to avoid a collision.
 */
function candidatesFor(anchor: AnnotationAnchor, width: number, height: number, gapPx: number, spacingPx: number): Rect[] {
  const out: Rect[] = [];
  const vStep = height + spacingPx;
  const hStep = width + spacingPx;
  for (const tier of TIERS) {
    for (const shift of SHIFTS) {
      const x = anchor.x + shift * hStep - width / 2;
      out.push({ x, y: anchor.y - gapPx - height - tier * vStep, width, height }); // above
      out.push({ x, y: anchor.y + gapPx + tier * vStep, width, height }); // below
    }
  }
  for (const tier of TIERS) {
    for (const shift of SHIFTS) {
      const y = anchor.y + shift * vStep - height / 2;
      out.push({ x: anchor.x + gapPx + tier * hStep, y, width, height }); // right
      out.push({ x: anchor.x - gapPx - width - tier * hStep, y, width, height }); // left
    }
  }
  return out;
}

function fitsAmong(rect: Rect, bounds: Rect, reserved: readonly Rect[], placed: readonly Rect[], spacingPx: number): boolean {
  if (!insideRect(rect, bounds)) {
    return false;
  }
  if (reserved.some((r) => rectsOverlap(rect, r))) {
    return false;
  }
  return !placed.some((r) => rectsOverlap(rect, r, spacingPx));
}

/**
 * Finds a box for one item: prefers candidates clear of the `avoid` rects, but falls back to
 * allowing them (never a `reserved` rect, another label, or the plot edge) rather than dropping
 * the label outright.
 */
function findSlot(
  item: AnnotationItem,
  bounds: Rect,
  reserved: readonly Rect[],
  avoid: readonly Rect[],
  placed: readonly Rect[],
  spacingPx: number,
  anchorGapPx: number,
): Rect | undefined {
  const candidates = candidatesFor(item.anchor, item.width, item.height, anchorGapPx, spacingPx);
  let fallback: Rect | undefined;
  for (const rect of candidates) {
    if (!fitsAmong(rect, bounds, reserved, placed, spacingPx)) {
      continue;
    }
    const hitsAvoid = avoid.some((r) => rectsOverlap(rect, r));
    if (!hitsAvoid) {
      return rect;
    }
    fallback ??= rect;
  }
  return fallback;
}

/**
 * Places `items` (highest priority — lowest `priority` number — first), each keeping its own
 * anchor untouched. Returns the placed labels with their leader lines, and the items that did not
 * make the cap or could not be placed, both in priority order.
 */
export function layoutAnnotations<T extends AnnotationItem>(
  items: readonly T[],
  options: AnnotationLayoutOptions,
): AnnotationLayoutResult<T> {
  const spacingPx = options.spacingPx ?? 4;
  const edgeGapPx = options.edgeGapPx ?? 0;
  const anchorGapPx = options.anchorGapPx ?? 10;
  const maxLabels = Math.max(0, options.maxLabels ?? 7);
  const reserved = options.reserved ?? [];
  const avoid = options.avoid ?? [];
  const bounds: Rect = {
    x: options.rect.x + edgeGapPx,
    y: options.rect.y + edgeGapPx,
    width: Math.max(0, options.rect.width - 2 * edgeGapPx),
    height: Math.max(0, options.rect.height - 2 * edgeGapPx),
  };

  const sorted = items
    .map((item, index) => ({ item, index }))
    .sort((a, b) => a.item.priority - b.item.priority || a.index - b.index)
    .map((entry) => entry.item);

  const attempted = sorted.slice(0, maxLabels);
  const overflow = sorted.slice(maxLabels);

  const placed: PlacedAnnotation<T>[] = [];
  const failed: T[] = [];

  for (const item of attempted) {
    const rect = findSlot(
      item,
      bounds,
      reserved,
      avoid,
      placed.map((p) => p.rect),
      spacingPx,
      anchorGapPx,
    );
    if (rect) {
      placed.push({ item, rect, leader: leaderFor(item.anchor, rect) });
    } else {
      failed.push(item);
    }
  }

  return { placed, dropped: [...failed, ...overflow] };
}
