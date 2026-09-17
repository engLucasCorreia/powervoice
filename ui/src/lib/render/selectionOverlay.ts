/**
 * H-79 (SPEC-006 §2.12 Amendment 2): the time selection's overlay geometry — the fill wash and
 * the boundary lines at each edge — shared by the waveform and spectral panes' WebGL2 and Canvas2D
 * paths, exactly like `loopOverlay.ts`. Pure and canvas-free.
 *
 * Before H-79 the selection was only ever a filled rectangle: no boundary line, and (on the
 * waveform pane) painted *after* the wave content, so a wash the same hue as the wave swallowed
 * it. The fix has two parts, both driven from this one geometry:
 *  - the waveform pane now draws the fill *before* its wave content (see `WaveformView.svelte`'s
 *    `drawWebgl2`/`drawCanvas2d`) and the boundary lines *after* it, alongside the other overlays;
 *  - the spectral pane (whose content is an opaque tile image, not a thin line — a translucent
 *    tint over it doesn't hide anything the way it hid the waveform) keeps drawing the fill after
 *    its content, unchanged, and simply gains the same boundary lines for a consistent look.
 */

import { pixelAtSample } from "../waveform/coords";

export interface SelectionGeometry {
  /** The fill rect `[x0, x1)`, clipped to the viewport (`null`: no selection, or off-screen). */
  fill: { x0: number; x1: number } | null;
  /** Boundary line x positions (unclipped pixel positions that fall within the viewport). */
  lines: number[];
}

/** Where the selection `[startSample, endSample)` lands in a viewport of `viewportPx` pixels. */
export function selectionGeometry(
  selection: { startSample: number; endSample: number } | null,
  startSample: number,
  samplesPerPixel: number,
  viewportPx: number,
): SelectionGeometry {
  if (!selection) {
    return { fill: null, lines: [] };
  }
  const sx0 = pixelAtSample(selection.startSample, startSample, samplesPerPixel);
  const sx1 = pixelAtSample(selection.endSample, startSample, samplesPerPixel);
  const x0 = Math.max(0, sx0);
  const x1 = Math.min(viewportPx, sx1);
  return {
    fill: x1 > x0 ? { x0, x1 } : null,
    lines: [sx0, sx1].filter((px) => px >= -1 && px <= viewportPx + 1),
  };
}
