/**
 * Selection / marker / playhead overlay geometry (H-13), shared by the waveform and spectrogram
 * WebGL2 renderers (SPEC-006 §4.5, SPEC-007 §4.7's "overlays use the SPEC-006 §2.12 tokens"): both
 * panes draw the same three kinds of overlay over their own content, at the same pixel positions
 * as their Canvas2D fallback (`pixelAtSample` — SPEC-006 §4.1's "one implementation, shared by
 * every call site"). Pure and canvas-free, so both renderers build identical geometry from the
 * same view state instead of two hand-written copies that could drift apart.
 */

import { pixelAtSample } from "../waveform/coords";
import { LOOP_STRIP_PX, loopGeometry } from "./loopOverlay";
import { QuadBatch, type Rgba } from "./quads";

export interface OverlayMarker {
  pos_samples: number;
  len_samples: number;
}

export interface OverlayColors {
  selectionFill: Rgba;
  marker: Rgba;
  markerRegionFill: Rgba;
  playhead: Rgba;
  /** H-37: the loop brace/boundaries (required when `loop` is set). */
  loop?: Rgba;
}

export interface OverlayInput {
  startSample: number;
  samplesPerPixel: number;
  viewportPx: number;
  heightPx: number;
  selection: { startSample: number; endSample: number } | null;
  /** H-37: the active loop region (`null`/absent: not looping). */
  loop?: { startSample: number; endSample: number } | null;
  /** H-37: the loop brace strip's height in this batch's pixel space. Default `LOOP_STRIP_PX`. */
  loopStripPx?: number;
  markers: readonly OverlayMarker[];
  /** `null` hides the playhead (e.g. no document open). */
  playheadSample: number | null;
  colors: OverlayColors;
  /** `"flags-and-regions"` (default, the waveform's own look): a region marker also gets a filled
   * band and a triangular flag at each end. `"lines"` (the spectral pane's look, matching its own
   * existing Canvas2D `drawOverlays`): every marker is just its start-position line, regardless of
   * length — kept distinct so the spectral pane's WebGL2 path doesn't grow overlay detail its own
   * Canvas2D fallback never had (SPEC-006 §4.5: "exactly one visual output" per view). */
  markerStyle?: "flags-and-regions" | "lines";
  /** Width of the marker and playhead lines, in the same pixel space as the rest (T-708: the
   * theme's `--pv-stroke-content` — 2 px in High Contrast). Default 1. */
  lineWidthPx?: number;
}

/** Builds the selection fill + marker regions/flags + playhead line as one batch, in the same
 * left-to-right/back-to-front order the Canvas2D fallback paints them (selection under markers
 * under the playhead), clipped to `[0, viewportPx]` exactly like `drawSelection`/`drawMarkers`/
 * `drawPlayhead`. */
export function buildOverlayBatch(input: OverlayInput): QuadBatch {
  const batch = new QuadBatch();
  const { startSample, samplesPerPixel, viewportPx, heightPx, colors } = input;
  if (viewportPx <= 0 || heightPx <= 0 || samplesPerPixel <= 0) {
    return batch;
  }
  const markerStyle = input.markerStyle ?? "flags-and-regions";
  const lineWidthPx = input.lineWidthPx ?? 1;

  if (input.selection) {
    const x0 = Math.max(0, pixelAtSample(input.selection.startSample, startSample, samplesPerPixel));
    const x1 = Math.min(viewportPx, pixelAtSample(input.selection.endSample, startSample, samplesPerPixel));
    batch.rect(x0, 0, x1, heightPx, colors.selectionFill);
  }

  if (input.loop && colors.loop) {
    const geometry = loopGeometry(input.loop, startSample, samplesPerPixel, viewportPx);
    if (geometry.strip) {
      const stripPx = Math.min(input.loopStripPx ?? LOOP_STRIP_PX, heightPx);
      batch.rect(geometry.strip.x0, 0, geometry.strip.x1, stripPx, colors.loop);
    }
    for (const px of geometry.lines) {
      batch.vLine(px, 0, heightPx, colors.loop, lineWidthPx);
    }
  }

  for (const marker of input.markers) {
    const startPx = pixelAtSample(marker.pos_samples, startSample, samplesPerPixel);
    if (markerStyle === "flags-and-regions" && marker.len_samples > 0) {
      const endPx = pixelAtSample(marker.pos_samples + marker.len_samples, startSample, samplesPerPixel);
      if (endPx >= 0 && startPx <= viewportPx) {
        batch.rect(startPx, 0, endPx, heightPx, colors.markerRegionFill);
        batch.flag(endPx, colors.marker);
      }
    }
    if (startPx >= -6 && startPx <= viewportPx + 6) {
      batch.vLine(startPx, 0, heightPx, colors.marker, lineWidthPx);
      if (markerStyle === "flags-and-regions") {
        batch.flag(startPx, colors.marker);
      }
    }
  }

  if (input.playheadSample !== null) {
    const px = pixelAtSample(input.playheadSample, startSample, samplesPerPixel);
    if (px >= -1 && px <= viewportPx + 1) {
      batch.vLine(px, 0, heightPx, colors.playhead, lineWidthPx);
    }
  }

  return batch;
}
