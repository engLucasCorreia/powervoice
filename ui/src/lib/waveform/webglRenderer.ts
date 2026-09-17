/**
 * WebGL2 draw orchestration for the waveform view (H-13, SPEC-006 §2.13/§4.5). Thin: the actual
 * vertex math lives in `webglGeometry.ts` (min/max columns, raw polyline) and
 * `../render/overlayGeometry.ts` (selection/markers/playhead), both pure and unit-tested; this
 * class only clears the canvas, uploads whichever geometry the caller built, and issues the draw
 * calls, batching the column fill and every overlay into as few calls as SPEC-006 §4.5 asks for
 * (one `TRIANGLES` call for fills; raw-sample mode adds a `TRIANGLES` call for the polyline-as-
 * quads (H-31) plus `POINTS` for its dots).
 *
 * Coordinate spaces: vertex positions from `webglGeometry.ts`/`overlayGeometry.ts` are in the same
 * **CSS-pixel** space the Canvas2D fallback draws in (`ctx.setTransform(dpr, ...)` there does the
 * same job `uResolutionPx` does here) — `gl.viewport` is still sized to the full device-pixel
 * backing store, so the GPU does the DPR upscale exactly like the 2D canvas's transform does. Only
 * `gl_PointSize` (dot radius) needs an explicit `devicePixelRatio` factor, since it's specified in
 * real framebuffer pixels regardless of the coordinate trick.
 */

import { QuadProgram } from "../render/glProgram";
import type { Rgba } from "../render/quads";
import type { RawPolylineGeometry } from "./webglGeometry";

export type WaveformGlContent =
  | { mode: "columns"; vertices: Float32Array }
  | { mode: "raw"; geometry: RawPolylineGeometry };

export interface WaveformGlDrawOptions {
  /** Real framebuffer size (`canvas.width`/`height`, already DPR-scaled) — used only for
   * `gl.viewport`. */
  backingWidthPx: number;
  backingHeightPx: number;
  /** CSS-pixel coordinate space that every vertex position (from `webglGeometry.ts`/
   * `overlayGeometry.ts`) is expressed in. */
  cssWidthPx: number;
  cssHeightPx: number;
  devicePixelRatio: number;
  background: Rgba;
  /** H-79: the selection fill, drawn right after the clear and *before* `content` — painting a
   * same-hue wash on top of the wave (the original bug) is what this ordering avoids; the wave
   * then draws fully opaque on top of it. `null`/absent: no selection. */
  underlay?: Float32Array | null;
  content: WaveformGlContent | null;
  /** Selection border + markers + playhead + (while recording) the record-head line, pre-batched
   * by the caller with `overlayGeometry.ts` (H-13: one shared builder for both renderers). */
  overlay: Float32Array | null;
}

export class WaveformGlRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly quads: QuadProgram;

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.quads = new QuadProgram(gl);
  }

  draw(opts: WaveformGlDrawOptions): void {
    const gl = this.gl;
    gl.viewport(0, 0, opts.backingWidthPx, opts.backingHeightPx);
    const [br, bg, bb, ba] = opts.background;
    gl.clearColor(br, bg, bb, ba);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const w = opts.cssWidthPx;
    const h = opts.cssHeightPx;

    if (opts.underlay && opts.underlay.length > 0) {
      this.quads.draw(opts.underlay, w, h, gl.TRIANGLES);
    }

    if (opts.content?.mode === "columns") {
      this.quads.draw(opts.content.vertices, w, h, gl.TRIANGLES);
    } else if (opts.content?.mode === "raw") {
      const { line, dots } = opts.content.geometry;
      if (line.length > 0) {
        // H-31: `line` is already a batch of `widthPx`-wide quads (2 triangles each), built by
        // `buildRawPolyline` — not a `LINE_STRIP` (which WebGL can't reliably draw wider than 1px).
        this.quads.draw(line, w, h, gl.TRIANGLES);
      }
      if (dots.length > 0) {
        this.quads.draw(dots, w, h, gl.POINTS, 3 * opts.devicePixelRatio);
      }
    }

    if (opts.overlay && opts.overlay.length > 0) {
      this.quads.draw(opts.overlay, w, h, gl.TRIANGLES);
    }
  }

  dispose(): void {
    this.quads.dispose();
  }
}
