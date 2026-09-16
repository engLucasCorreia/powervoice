/**
 * Spectrogram tile grid (SPEC-007 §2.6, §4.3; mirrors `vox_dsp::spectro` and
 * `vox_engine::spectro`): FFT Auto size, the zoom → hop rule, and which tiles a viewport needs.
 * Pure functions, shared by the requester and the renderer (T-207).
 */

export const TILE_FRAMES = 256;
export const MIN_FFT_SIZE = 256;
export const MAX_FFT_SIZE = 16_384;
export const FFT_SIZES = [256, 512, 1024, 2048, 4096, 8192, 16_384] as const;
/** The engine serves at most this many tiles per request (SPEC-007 §4.6). */
export const MAX_TILES_PER_REQUEST = 64;
/** ADR-003 `window` = Hann. */
export const WINDOW_HANN = 0;

/** SPEC-007 §2.6 Auto: the power of two nearest to `2048 · fs / 48 000`, clamped to 256…16 384. */
export function autoFftSize(sampleRateHz: number): number {
  const target = (2048 * sampleRateHz) / 48_000;
  let best: number = MIN_FFT_SIZE;
  for (const n of FFT_SIZES) {
    if (Math.abs(n - target) < Math.abs(best - target)) {
      best = n;
    }
  }
  return best;
}

/**
 * SPEC-007 §4.3: `hop = max(pow2_floor(sppDev), N/16)`, with `sppDev` = document samples per
 * **device** pixel (1–2 frames per device column when zoomed out, ≥ 93.75 % overlap zoomed in).
 */
export function hopForZoom(samplesPerDevicePixel: number, fftSize: number): number {
  const spp =
    Number.isFinite(samplesPerDevicePixel) && samplesPerDevicePixel >= 1
      ? Math.min(samplesPerDevicePixel, 2 ** 31)
      : 1;
  const floor = 2 ** Math.floor(Math.log2(spp));
  return Math.max(floor, fftSize / 16);
}

/**
 * H-12 (HiDPI): the STFT-frame boundaries `[lo, hi)` of each **device**-pixel column of a
 * `backingWidthPx`-wide (i.e. `Math.round(viewportPx * dpr)`) spectrogram canvas — one column per
 * physical pixel, not per CSS pixel, so the pane doesn't look blurry/blocky on a HiDPI display.
 * `startSample`/`samplesPerPixel` are in CSS-pixel (document) units, as the shared viewport always
 * is; `dpr` converts a device-pixel column index back to a CSS-pixel offset before turning it into
 * a sample position. Pure and canvas-free so it's testable without a 2D context (jsdom has none).
 */
export function frameColumnBounds(
  backingWidthPx: number,
  startSample: number,
  samplesPerPixel: number,
  dpr: number,
  hop: number,
): { lo: Float64Array; hi: Float64Array } {
  const lo = new Float64Array(backingWidthPx);
  const hi = new Float64Array(backingWidthPx);
  for (let px = 0; px < backingWidthPx; px++) {
    const s0 = startSample + (px / dpr) * samplesPerPixel;
    const s1 = s0 + samplesPerPixel / dpr;
    lo[px] = s0 / hop;
    hi[px] = s1 / hop;
  }
  return { lo, hi };
}

/**
 * H-13 (WebGL2 renderer): the linear frame-at-device-pixel mapping `frameColumnBounds` uses per
 * column (`frame(px) = frameAtPx0 + framesPerPx * px`), as two scalars instead of a per-column
 * array — the fragment shader recomputes `frame(px)` itself from these two uniforms rather than
 * sampling a per-column lookup, so this is the CPU-side derivation shared with the tests.
 */
export function frameLinearMapping(
  startSample: number,
  samplesPerPixel: number,
  dpr: number,
  hop: number,
): { frameAtPx0: number; framesPerPx: number } {
  return {
    frameAtPx0: startSample / hop,
    framesPerPx: samplesPerPixel / (dpr * hop),
  };
}

/**
 * H-13: which tile indices' quads could be visible across device-pixel columns
 * `[0, backingWidthPx)`, given {@link frameLinearMapping}'s scalars — one tile of margin on each
 * side so a quad's edge is never clipped by a rounding sliver. Ordered ascending (draw order
 * doesn't matter: tiles never overlap after {@link tileDevicePxRange}'s partition).
 */
export function visibleTileIndices(
  frameAtPx0: number,
  framesPerPx: number,
  backingWidthPx: number,
  tileCount: number,
): number[] {
  if (tileCount <= 0 || backingWidthPx <= 0) {
    return [];
  }
  const lastFrame = frameAtPx0 + framesPerPx * backingWidthPx;
  const first = Math.max(0, Math.floor(frameAtPx0 / TILE_FRAMES) - 1);
  const last = Math.min(tileCount - 1, Math.floor(lastFrame / TILE_FRAMES) + 1);
  const out: number[] = [];
  for (let k = first; k <= last; k++) {
    out.push(k);
  }
  return out;
}

/**
 * H-13: the device-pixel column range `[x0, x1)` tile `tileIndex`'s quad should cover, the exact
 * inverse of {@link frameLinearMapping}'s `frame(px)` — adjacent tiles partition `[0,
 * backingWidthPx)` with no gap or overlap because both boundaries are computed from the same
 * formula rounded the same way.
 */
export function tileDevicePxRange(
  tileIndex: number,
  frameAtPx0: number,
  framesPerPx: number,
  backingWidthPx: number,
): { x0: number; x1: number } {
  if (!(framesPerPx > 0)) {
    return { x0: 0, x1: 0 };
  }
  const pxAtFrame = (frame: number) => (frame - frameAtPx0) / framesPerPx;
  const clamp = (px: number) => Math.max(0, Math.min(backingWidthPx, Math.round(px)));
  const x0 = clamp(pxAtFrame(tileIndex * TILE_FRAMES));
  const x1 = clamp(pxAtFrame((tileIndex + 1) * TILE_FRAMES));
  return { x0, x1 };
}

/** Overview hops (`hop > N`) get a `PREVIEW` tile before the refined one (SPEC-007 §4.4). */
export function isOverview(fftSize: number, hop: number): boolean {
  return hop > fftSize;
}

/** Frames of the grid anchored at 0 with centre in `[0, len)`: `ceil(len / hop)`. */
export function totalFrames(lenSamples: number, hop: number): number {
  return Math.ceil(lenSamples / hop);
}

export function tileCount(lenSamples: number, hop: number): number {
  return Math.ceil(totalFrames(lenSamples, hop) / TILE_FRAMES);
}

/**
 * H-54 (SPEC-007 AC-10, Canvas2D fallback): above this many (device column × visible bin) units,
 * `SpectralView.svelte`'s Canvas2D path renders at a reduced time-axis resolution and
 * nearest-neighbour-upscales (see {@link canvasColumnStride}). A CDP profile of the failing case
 * (2126×850 window, 1526×219-device-pixel pane, FFT Auto = 2048 → 1025 bins) found 73.6 % of every
 * frame in `columnDb` (SPEC-007 §4.7's time-axis rule, §4.3) plus the pixel-write loop around it —
 * both O(bins) per column and, critically, **independent of the pane's height**: a pixel row's
 * frequency-axis span already merges every bin it covers, so shrinking `backingHeightPx` instead
 * would not shrink the total bins visited. The budget is set just above the passing 1280×720
 * case's 680 × 1025 ≈ 697 000 units, so it is a no-op there (stride 1, bit-identical output) and
 * only engages for the failing size and other panes at least as demanding.
 */
export const CANVAS2D_COLUMN_BIN_BUDGET = 750_000;

/**
 * H-54: how many adjacent device-pixel columns `SpectralView.svelte`'s Canvas2D path should group
 * into one rendered column (1 = full resolution, every device pixel computed). Pure so the budget
 * and the rounding are unit-tested without a canvas.
 */
export function canvasColumnStride(backingWidthPx: number, bins: number): number {
  if (backingWidthPx <= 0 || bins <= 0) {
    return 1;
  }
  return Math.max(1, Math.ceil((backingWidthPx * bins) / CANVAS2D_COLUMN_BIN_BUDGET));
}

/** Centre of frame `frame` of tile `tileIndex`: `(256·k + i)·hop` (SPEC-007 §4.3). */
export function frameCenterSample(tileIndex: number, frame: number, hop: number): number {
  return (tileIndex * TILE_FRAMES + frame) * hop;
}

/**
 * The tiles a viewport `[startSample, endSample)` needs, in request order (SPEC-007 §2.8): the
 * visible tiles left to right, then one viewport's width on each side, nearest first,
 * alternating right/left. Capped at {@link MAX_TILES_PER_REQUEST}; empty for an empty document.
 */
export function tilesForView(
  startSample: number,
  endSample: number,
  lenSamples: number,
  hop: number,
): number[] {
  const count = tileCount(lenSamples, hop);
  if (count === 0 || endSample < startSample) {
    return [];
  }
  const span = TILE_FRAMES * hop;
  const tileAt = (sample: number) =>
    Math.min(count - 1, Math.max(0, Math.floor(Math.max(0, sample) / span)));
  // A frame is drawn if its centre lies within half a hop of the viewport.
  const first = tileAt(startSample - hop / 2);
  const last = tileAt(endSample + hop / 2);
  const out: number[] = [];
  for (let k = first; k <= last && out.length < MAX_TILES_PER_REQUEST; k++) {
    out.push(k);
  }
  const width = endSample - startSample;
  const leftmost = tileAt(startSample - width);
  const rightmost = tileAt(endSample + width);
  for (let step = 1; out.length < MAX_TILES_PER_REQUEST; step++) {
    const right = last + step;
    const left = first - step;
    const hasRight = right <= rightmost;
    const hasLeft = left >= leftmost;
    if (!hasRight && !hasLeft) {
      break;
    }
    if (hasRight) {
      out.push(right);
    }
    if (hasLeft && out.length < MAX_TILES_PER_REQUEST) {
      out.push(left);
    }
  }
  return out;
}
