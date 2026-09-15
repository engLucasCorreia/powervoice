/**
 * H-37 (SPEC-006 §2.12 amendment): the loop region's overlay geometry, shared by the waveform and
 * spectral panes (WebGL2 via `overlayGeometry.ts`, and their Canvas2D fallbacks) — a brace strip
 * along the top edge between the loop start and end, plus a boundary line at each end, in the
 * `--wave-loop` token. The selection fill already shades the region itself (the loop region is
 * the selection while loop is on), so the loop marks only its extent. Pure and canvas-free.
 */

import { pixelAtSample } from "../waveform/coords";

/** Height of the brace strip along the top edge (CSS px; scale by the device ratio for a
 * device-pixel overlay). */
export const LOOP_STRIP_PX = 3;

export interface LoopGeometry {
  /** The brace strip `[x0, x1)`, clipped to the viewport (`null`: off-screen). */
  strip: { x0: number; x1: number } | null;
  /** Boundary line x positions (unclipped pixel positions that fall within the viewport). */
  lines: number[];
}

/** Where the loop `[startSample, endSample)` lands in a viewport of `viewportPx` pixels. */
export function loopGeometry(
  loop: { startSample: number; endSample: number },
  startSample: number,
  samplesPerPixel: number,
  viewportPx: number,
): LoopGeometry {
  const lx0 = pixelAtSample(loop.startSample, startSample, samplesPerPixel);
  const lx1 = pixelAtSample(loop.endSample, startSample, samplesPerPixel);
  const x0 = Math.max(0, lx0);
  const x1 = Math.min(viewportPx, lx1);
  return {
    strip: x1 > x0 ? { x0, x1 } : null,
    lines: [lx0, lx1].filter((px) => px >= -1 && px <= viewportPx + 1),
  };
}

/** The engine's `loop_range` tuple as the `{ startSample, endSample }` shape views use. */
export function loopFromRange(range: readonly [number, number] | null | undefined): {
  startSample: number;
  endSample: number;
} | null {
  return range && range[1] > range[0] ? { startSample: range[0], endSample: range[1] } : null;
}
