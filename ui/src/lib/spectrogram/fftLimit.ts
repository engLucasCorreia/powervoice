/**
 * FFT-size texture-limit check (SPEC-007 §2.6, AC-14): "sizes whose bin count `N/2 + 1` exceeds
 * the GPU's `MAX_TEXTURE_SIZE` are disabled". `isFftSizeDisabled` is pure and testable with a
 * stubbed limit; {@link detectMaxTextureSize} does the real WebGL2 probe (Canvas2D has no such
 * limit — ADR-009 §4/§2 — so it reports `Infinity` when WebGL2 is unavailable).
 */

/** Probes the real `MAX_TEXTURE_SIZE` via a throwaway WebGL2 context, or `Infinity` if WebGL2
 * isn't available (the Canvas2D fallback has no texture-size limit). */
export function detectMaxTextureSize(): number {
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2");
    if (!gl) {
      return Infinity;
    }
    const size = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    return typeof size === "number" && size > 0 ? size : Infinity;
  } catch {
    return Infinity;
  }
}

/** SPEC-007 §2.6, AC-14: an FFT size is disabled once its bin count `N/2 + 1` exceeds
 * `maxTextureSize`. */
export function isFftSizeDisabled(fftSize: number, maxTextureSize: number): boolean {
  return fftSize / 2 + 1 > maxTextureSize;
}
