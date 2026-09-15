/**
 * Collision-free placement of the analyzer's peak labels (H-42, SPEC-007 §8.2), in the spirit of
 * `ui/axisLabels.ts` but in two dimensions: each label tries a fixed list of spots around its
 * peak marker (above, above-right, above-left, right, left, below) and takes the first one that
 * is inside the plot, clear of every label already placed and of any reserved rectangle (the
 * hover readout, the peak markers themselves). Labels are placed in priority order (the loudest
 * peak first); one that fits nowhere is dropped — its marker still shows.
 */
import { rectsOverlap, type Rect } from "../ui/axisLabels";

export type LabelSpot = "above" | "above-right" | "above-left" | "right" | "left" | "below";

export const LABEL_SPOTS: readonly LabelSpot[] = [
  "above",
  "above-right",
  "above-left",
  "right",
  "left",
  "below",
];

export interface LabelAnchor {
  /** Marker position in plot px. */
  x: number;
  y: number;
  /** Label box size, px. */
  width: number;
  height: number;
}

export interface PlacedLabel<T extends LabelAnchor> {
  item: T;
  spot: LabelSpot;
  rect: Rect;
}

export interface PlaceOptions {
  width: number;
  height: number;
  /** Space between marker and label, px (default 6). */
  gapPx?: number;
  /** Minimum clear space between labels, px (default 3). */
  spacingPx?: number;
  /** Rectangles no label may touch. */
  reserved?: readonly Rect[];
}

function rectFor(a: LabelAnchor, spot: LabelSpot, gap: number): Rect {
  const { x, y, width: w, height: h } = a;
  switch (spot) {
    case "above":
      return { x: x - w / 2, y: y - gap - h, width: w, height: h };
    case "above-right":
      return { x: x + gap / 2, y: y - gap - h, width: w, height: h };
    case "above-left":
      return { x: x - gap / 2 - w, y: y - gap - h, width: w, height: h };
    case "right":
      return { x: x + gap, y: y - h / 2, width: w, height: h };
    case "left":
      return { x: x - gap - w, y: y - h / 2, width: w, height: h };
    default:
      return { x: x - w / 2, y: y + gap, width: w, height: h };
  }
}

function inside(r: Rect, width: number, height: number): boolean {
  return r.x >= 0 && r.y >= 0 && r.x + r.width <= width && r.y + r.height <= height;
}

/** Places `items` (highest priority first); returns the ones that fit, in the same order. */
export function placePeakLabels<T extends LabelAnchor>(
  items: readonly T[],
  options: PlaceOptions,
): PlacedLabel<T>[] {
  const gap = options.gapPx ?? 6;
  const spacing = options.spacingPx ?? 3;
  const reserved = options.reserved ?? [];
  const placed: PlacedLabel<T>[] = [];
  for (const item of items) {
    for (const spot of LABEL_SPOTS) {
      const rect = rectFor(item, spot, gap);
      if (!inside(rect, options.width, options.height)) {
        continue;
      }
      if (reserved.some((r) => rectsOverlap(rect, r, spacing))) {
        continue;
      }
      if (placed.some((p) => rectsOverlap(rect, p.rect, spacing))) {
        continue;
      }
      placed.push({ item, spot, rect });
      break;
    }
  }
  return placed;
}
