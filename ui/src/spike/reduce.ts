/**
 * Reduces the waveform's precomputed (min, max) bucket pyramid level down to one (min, max) pair
 * per display column — the same "pick a level, reduce buckets to pixels" step ADR-003 describes
 * for the real waveform view — used by both the WebGL2 and Canvas2D renderers so the CPU
 * decimation cost (which is real and shared, not a WebGL-vs-Canvas2D difference) is identical
 * between them and only the draw/upload path differs.
 *
 * `minMax` is `count * 2` floats (interleaved min, max). Only one spp level exists in the spike
 * (unlike production, which picks the coarsest level ≤ the pixel spp — see ADR-003 "UI choice"),
 * so at the widest zoom-out this reduces far more than 4 buckets/column; that's a known
 * simplification, noted in ADR-009.
 */
export function reduceToColumns(
  minMax: Float32Array,
  count: number,
  startBucket: number,
  visibleBuckets: number,
  columns: number,
  out: Float32Array,
): void {
  const clampedVisible = Math.max(1, Math.min(visibleBuckets, count - startBucket));
  const stride = clampedVisible / columns;
  for (let col = 0; col < columns; col++) {
    const from = startBucket + Math.floor(col * stride);
    const to = Math.max(from + 1, startBucket + Math.floor((col + 1) * stride));
    const end = Math.min(to, count);
    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (let b = from; b < end; b++) {
      const bMin = minMax[b * 2] ?? 0;
      const bMax = minMax[b * 2 + 1] ?? 0;
      if (bMin < min) min = bMin;
      if (bMax > max) max = bMax;
    }
    if (min === Number.POSITIVE_INFINITY) {
      min = 0;
      max = 0;
    }
    out[col * 2] = min;
    out[col * 2 + 1] = max;
  }
}

/** Triangle wave 0 -> 1 -> 0 across a 0..1 run: "whole file -> ~1 sample/px -> back" (ticket). */
export function zoomTriangle(progress: number): number {
  return progress < 0.5 ? progress * 2 : (1 - progress) * 2;
}
