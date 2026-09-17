/**
 * Marker drag math (H-57, SPEC-009 §2.5): flag hit-testing and the drag/magnet/clamp geometry,
 * pure functions shared by `WaveformView`'s pointer handling and its tests. Kept separate from
 * `selection.ts` because markers have their own hit box, their own magnet targets (cursor,
 * selection, other markers — never zero-crossing, SPEC-009 §2.5) and their own clamp rules
 * (a region's edges can't cross).
 *
 * No drift at any zoom: every function here computes an *absolute* target sample from the
 * pointer's current position (never accumulates per-frame deltas), so repeated small moves never
 * compound rounding error.
 */

import { clampStartSample, pixelAtSample } from "./coords";

/** The marker shape this module needs (a subset of `MarkerDto`). */
export interface DragMarker {
  id: number;
  pos_samples: number;
  len_samples: number;
}

/** SPEC-009 §2.5/§3 `flag_hit_min_px`: the flag hit box is 10 px wide (±5 px around the line). */
export const FLAG_HIT_HALF_WIDTH_PX = 5;
/** SPEC-009 §3 `flag_hit_min_px`: 12 px high, at the top edge of the canvas. */
export const FLAG_HIT_HEIGHT_PX = 12;
/** SPEC-009 §3 `drag_threshold_px`: press-and-move-≥-this starts a drag; less is a click. */
export const DRAG_THRESHOLD_PX = 3;
/** SPEC-009 §3 `marker_magnet_px` (= SPEC-006's `selection_handle_hit_px`). */
export const MARKER_MAGNET_PX = 6;

/** Which part of a marker a flag hit belongs to. */
export type MarkerFlagEdge = "point" | "start" | "end";

export interface MarkerFlagHit {
  id: number;
  edge: MarkerFlagEdge;
}

/**
 * Hit-tests a pointer at device pixel `(px, py)` against every marker's flag(s) (SPEC-009 §2.5):
 * a point marker has one flag at `pos`; a region has a start flag at `pos` and an end flag at
 * `pos + len`. The flag hit box is `2 * FLAG_HIT_HALF_WIDTH_PX` wide and `FLAG_HIT_HEIGHT_PX`
 * high, anchored at the top of the canvas (`py` in `[0, FLAG_HIT_HEIGHT_PX]`) — the marker line
 * below it is not a hit target (SPEC-009 §2.5: dragging near a marker still selects). At equal
 * distance the earlier sample wins (deterministic, matches the magnet tie-break).
 */
export function hitTestMarkerFlag(
  px: number,
  py: number,
  markers: readonly DragMarker[],
  startSample: number,
  samplesPerPixel: number,
): MarkerFlagHit | null {
  if (py < 0 || py > FLAG_HIT_HEIGHT_PX) {
    return null;
  }
  let best: MarkerFlagHit | null = null;
  let bestDist = Infinity;
  let bestSample = Infinity;
  for (const marker of markers) {
    const candidates: Array<[number, MarkerFlagEdge]> =
      marker.len_samples > 0
        ? [
            [marker.pos_samples, "start"],
            [marker.pos_samples + marker.len_samples, "end"],
          ]
        : [[marker.pos_samples, "point"]];
    for (const [sample, edge] of candidates) {
      const flagPx = pixelAtSample(sample, startSample, samplesPerPixel);
      const dist = Math.abs(px - flagPx);
      if (dist > FLAG_HIT_HALF_WIDTH_PX) {
        continue;
      }
      if (dist < bestDist || (dist === bestDist && sample < bestSample)) {
        bestDist = dist;
        bestSample = sample;
        best = { id: marker.id, edge };
      }
    }
  }
  return best;
}

/**
 * SPEC-009 §2.5's magnet: snaps `rawSample` to the nearest of `targets` within `magnetPx`
 * (converted through `samplesPerPixel`, so the magnet is a *pixel* distance regardless of zoom).
 * The nearest target wins; at an equal distance, the earlier sample wins. Returns `rawSample`
 * unchanged when nothing is within range (or `targets` is empty).
 */
export function snapToMarkerMagnet(
  rawSample: number,
  targets: readonly number[],
  samplesPerPixel: number,
  magnetPx: number = MARKER_MAGNET_PX,
): number {
  let best: number | null = null;
  let bestDistPx = Infinity;
  for (const target of targets) {
    const distPx = Math.abs(target - rawSample) / samplesPerPixel;
    if (distPx > magnetPx) {
      continue;
    }
    if (best === null || distPx < bestDistPx || (distPx === bestDistPx && target < best)) {
      best = target;
      bestDistPx = distPx;
    }
  }
  return best ?? rawSample;
}

/** The magnet targets for a drag of `draggedId` (SPEC-009 §2.5): the cursor (stopped/paused
 * only), the selection's two edges, and every *other* marker's start/end — the dragged marker's
 * own edges are always excluded. */
export function markerMagnetTargets(
  draggedId: number,
  markers: readonly DragMarker[],
  cursorSample: number | null,
  selection: { startSample: number; endSample: number } | null,
): number[] {
  const targets: number[] = [];
  if (cursorSample !== null) {
    targets.push(cursorSample);
  }
  if (selection) {
    targets.push(selection.startSample, selection.endSample);
  }
  for (const marker of markers) {
    if (marker.id === draggedId) {
      continue;
    }
    targets.push(marker.pos_samples);
    if (marker.len_samples > 0) {
      targets.push(marker.pos_samples + marker.len_samples);
    }
  }
  return targets;
}

/** The live (uncommitted) preview of a drag, in document samples — always `pos_samples`/
 * `len_samples`, the same shape a `Marker`/`MarkerDto` uses. */
export interface MarkerDragPreview {
  pos_samples: number;
  len_samples: number;
}

/**
 * The result of dragging a point marker's flag (SPEC-009 §2.5): `pos` moves to `rawSample`
 * (after the magnet, applied by the caller), clamped to `[0, lenSamplesDoc]`.
 */
export function dragPointMarker(rawSample: number, lenSamplesDoc: number): MarkerDragPreview {
  const pos = Math.max(0, Math.min(rawSample, lenSamplesDoc));
  return { pos_samples: pos, len_samples: 0 };
}

/**
 * The result of dragging a region's start flag: the start moves to `rawSample`, the end
 * (`fixedEnd`) stays — clamped so the edges can't cross (`len >= 1` throughout, SPEC-009 §2.5).
 */
export function dragRegionStart(rawSample: number, fixedEnd: number): MarkerDragPreview {
  const pos = Math.max(0, Math.min(rawSample, fixedEnd - 1));
  return { pos_samples: pos, len_samples: fixedEnd - pos };
}

/**
 * The result of dragging a region's end flag: the end moves to `rawSample`, the start
 * (`fixedStart`) stays — clamped so `len >= 1` and the end never passes the document length.
 */
export function dragRegionEnd(
  rawSample: number,
  fixedStart: number,
  lenSamplesDoc: number,
): MarkerDragPreview {
  const end = Math.max(fixedStart + 1, Math.min(rawSample, lenSamplesDoc));
  return { pos_samples: fixedStart, len_samples: end - fixedStart };
}

/**
 * Shift+drag (SPEC-009 §2.5): the whole region moves, keeping `len`. `rawSample` is the new
 * position of whichever edge was grabbed (`grabbedEdge`); the other edge follows by the same
 * delta. Clamped to `[0, lenSamplesDoc]` by capping the delta, never by changing `len`.
 */
export function dragRegionWhole(
  rawSample: number,
  original: { pos_samples: number; len_samples: number },
  grabbedEdge: "start" | "end",
  lenSamplesDoc: number,
): MarkerDragPreview {
  const grabbedOriginal =
    grabbedEdge === "start" ? original.pos_samples : original.pos_samples + original.len_samples;
  let delta = rawSample - grabbedOriginal;
  delta = Math.max(delta, -original.pos_samples);
  delta = Math.min(delta, lenSamplesDoc - (original.pos_samples + original.len_samples));
  return { pos_samples: original.pos_samples + delta, len_samples: original.len_samples };
}

// --- H-64 (SPEC-009 §2.5/§3 `drag_autoscroll_rate`): auto-scroll while dragging -------------------

/** SPEC-009 §3 `drag_autoscroll_rate`: one viewport width per second. */
export const DRAG_AUTOSCROLL_RATE_VIEWPORTS_PER_S = 1;

/**
 * SPEC-009 §2.5: "while the pointer is beyond the left or right canvas edge during a drag" —
 * `px` is the pointer's raw device-pixel x relative to the canvas (as {@link pixelAtSample}'s
 * inverse would read it, *not* clamped to `[0, viewportPx]`, unlike a document-sample lookup).
 * `-1`/`1` beyond the left/right edge, `0` inside it (or with no known viewport width yet).
 */
export function markerAutoscrollDirection(px: number, viewportPx: number): -1 | 0 | 1 {
  if (viewportPx <= 0) {
    return 0;
  }
  if (px < 0) {
    return -1;
  }
  if (px > viewportPx) {
    return 1;
  }
  return 0;
}

/**
 * One frame of auto-scroll (SPEC-009 §2.5, `drag_autoscroll_rate`): advances `startSample` by
 * `direction * rate * viewportSamples * dtSeconds` — "one viewport width per second" — and clamps
 * the result through {@link clampStartSample}, the same clamp every other viewport write in this
 * codebase uses, so a drag can never scroll past either end of the document ("stops at the
 * document edges"). A `direction` of `0`, a non-positive `dtSeconds` (the first tick after a
 * scroll starts, before two timestamps exist) or an unknown viewport width (`viewportPx <= 0`)
 * are no-ops, returning `startSample` unchanged.
 */
export function advanceMarkerAutoscroll(
  startSample: number,
  direction: -1 | 0 | 1,
  dtSeconds: number,
  samplesPerPixel: number,
  lenSamples: number,
  viewportPx: number,
  rate: number = DRAG_AUTOSCROLL_RATE_VIEWPORTS_PER_S,
): number {
  if (direction === 0 || dtSeconds <= 0 || viewportPx <= 0) {
    return startSample;
  }
  const viewportSamples = viewportPx * samplesPerPixel;
  const next = startSample + direction * rate * viewportSamples * dtSeconds;
  return clampStartSample(next, samplesPerPixel, lenSamples, viewportPx);
}
