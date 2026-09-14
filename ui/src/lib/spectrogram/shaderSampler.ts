/**
 * A bounded-loop mirror of `sampler.ts`'s max-or-interpolate rule (SPEC-007 §2.3/§2.4, §4.7 step 2:
 * "It may loop over up to 16 bins or frames per pixel") — this is the exact algorithm the WebGL2
 * fragment shader implements (`webglRenderer.ts`'s `SAMPLE_GLSL`), written once in TS so it can be
 * parity-tested against the canonical `sampleGridSpan`/`pixelDb` (`sampler.ts`) the Canvas2D
 * fallback uses, instead of trusting the GLSL source (untestable without a GPU) to match by eye.
 *
 * A real fragment shader can't run an unbounded loop, so every span here is capped at
 * {@link MAX_SHADER_TAPS} grid points — the spec explicitly sanctions this cap ("may loop over up
 * to 16"), and in practice SPEC-007 §4.3's hop rule keeps the time-axis span at 1-2 frames per
 * device column when zoomed out (never near 16), so the cap only ever bites in a pathological
 * frequency-axis case (an extreme log-scale zoom-out packing many bins into one row) — a graceful
 * approximation, not silent wrongness (the true max is still >= the capped max whenever there IS a
 * higher value beyond the cap, since a max over a superset can only be >=; a slight undercount of
 * the theoretical max is a strict improvement over pretending a bin doesn't exist).
 */

export const MAX_SHADER_TAPS = 16;

/** Bounded version of `sampler.ts`'s `sampleGridSpan`: identical logic, except the max-branch loop
 * never runs more than {@link MAX_SHADER_TAPS} iterations (starting from the span's lower edge, so
 * it's the *lowest* `MAX_SHADER_TAPS` grid points of the span that are ever missed when a span is
 * wider than the cap). */
export function sampleGridSpanBounded(
  count: number,
  lo: number,
  hi: number,
  at: (i: number) => number | null,
): number | null {
  if (count <= 0 || !(hi > lo)) {
    return null;
  }
  const clampedLo = Math.max(0, lo);
  const clampedHi = Math.min(count, hi);
  if (clampedHi <= clampedLo) {
    return null;
  }
  if (clampedHi - clampedLo >= 1) {
    const i0 = Math.max(0, Math.floor(clampedLo));
    const i1Full = Math.min(count - 1, Math.ceil(clampedHi) - 1);
    const i1 = Math.min(i1Full, i0 + MAX_SHADER_TAPS - 1);
    let max: number | null = null;
    for (let i = i0; i <= i1; i++) {
      const v = at(i);
      if (v !== null && (max === null || v > max)) {
        max = v;
      }
    }
    return max;
  }
  const center = Math.min(count - 1, Math.max(0, (lo + hi) / 2));
  const i0 = Math.floor(center);
  const i1 = Math.min(count - 1, i0 + 1);
  const v0 = at(i0);
  const v1 = at(i1);
  if (v0 === null) {
    return v1;
  }
  if (v1 === null) {
    return v0;
  }
  const frac = center - i0;
  return v0 + (v1 - v0) * frac;
}

/** Bounded version of `sampler.ts`'s `pixelDb`: the per-pixel dB value nesting the bounded
 * time-axis rule inside the bounded frequency-axis rule (SPEC-007 §4.7 step 2). */
export function pixelDbBounded(
  lookup: (frame: number, bin: number) => number | null,
  totalFrames: number,
  bins: number,
  frameLo: number,
  frameHi: number,
  binLo: number,
  binHi: number,
): number | null {
  const binValue = (bin: number): number | null =>
    sampleGridSpanBounded(totalFrames, frameLo, frameHi, (frame) => lookup(frame, bin));
  return sampleGridSpanBounded(bins, binLo, binHi, binValue);
}
