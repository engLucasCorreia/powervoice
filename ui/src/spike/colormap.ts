/** A simple magnitude -> color ramp, shared in spirit (not bit-for-bit) between the WebGL2
 * fragment shader and the Canvas2D lookup table, so both renderers do "real" colormap work. */
export function colormapRgb(t: number): [number, number, number] {
  const r = t * t;
  const g = Math.pow(t, 1.5) * 0.8;
  const b = Math.sin(t * Math.PI * 0.5) * (1 - t * 0.3);
  return [r, g, b];
}

/** 256-entry RGBA lookup table for the Canvas2D path (cheap per-pixel colormap via array index
 * instead of recomputing the ramp per pixel every frame). */
export function buildColormapLut(): Uint8ClampedArray {
  const lut = new Uint8ClampedArray(256 * 4);
  for (let i = 0; i < 256; i++) {
    const [r, g, b] = colormapRgb(i / 255);
    lut[i * 4 + 0] = r * 255;
    lut[i * 4 + 1] = g * 255;
    lut[i * 4 + 2] = b * 255;
    lut[i * 4 + 3] = 255;
  }
  return lut;
}

export const COLORMAP_GLSL = `
vec3 colormap(float t) {
  float r = t * t;
  float g = pow(t, 1.5) * 0.8;
  float b = sin(t * 1.5707963) * (1.0 - t * 0.3);
  return vec3(r, g, b);
}
`;
