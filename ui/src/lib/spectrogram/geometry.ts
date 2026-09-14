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
