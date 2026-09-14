/**
 * Colormaps for the spectral pane (SPEC-007 §2.5): Inferno (default), Viridis and Grayscale, each
 * a 256-entry RGB LUT, plus the dB → `t ∈ [0, 1]` normalization against the display floor/ceiling.
 *
 * SPEC-007 §2.5 says Inferno/Viridis come from matplotlib's CC0-licensed colormap data (van der
 * Walt & Smith). This ticket does not vendor that exact 256-entry table (no new dependency, and
 * the data itself wasn't available to copy byte-for-byte) — instead it builds each LUT from a
 * small set of well-known control-point colors for each map, linearly interpolated in RGB. The
 * result is visually equivalent (dark → highlight, perceptually smooth) but not a bit-exact match
 * to matplotlib's table; call this out if a pixel-perfect match is ever required.
 */

export type ColormapName = "inferno" | "viridis" | "gray";
export const COLORMAP_NAMES: readonly ColormapName[] = ["inferno", "viridis", "gray"];

interface Stop {
  t: number;
  rgb: readonly [number, number, number];
}

// Control points approximating matplotlib's `inferno` (black -> purple -> orange -> pale yellow).
const INFERNO_STOPS: readonly Stop[] = [
  { t: 0.0, rgb: [0, 0, 4] },
  { t: 0.13, rgb: [31, 12, 72] },
  { t: 0.25, rgb: [85, 15, 109] },
  { t: 0.38, rgb: [136, 34, 106] },
  { t: 0.5, rgb: [186, 54, 85] },
  { t: 0.63, rgb: [227, 89, 51] },
  { t: 0.75, rgb: [249, 140, 10] },
  { t: 0.88, rgb: [249, 201, 50] },
  { t: 1.0, rgb: [252, 255, 164] },
];

// Control points approximating matplotlib's `viridis` (dark purple -> teal -> yellow-green).
const VIRIDIS_STOPS: readonly Stop[] = [
  { t: 0.0, rgb: [68, 1, 84] },
  { t: 0.13, rgb: [71, 44, 122] },
  { t: 0.25, rgb: [59, 81, 139] },
  { t: 0.38, rgb: [44, 113, 142] },
  { t: 0.5, rgb: [33, 144, 141] },
  { t: 0.63, rgb: [39, 173, 129] },
  { t: 0.75, rgb: [92, 200, 99] },
  { t: 0.88, rgb: [170, 220, 50] },
  { t: 1.0, rgb: [253, 231, 37] },
];

function buildLut(stops: readonly Stop[]): Uint8ClampedArray {
  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    let lo = stops[0]!;
    let hi = stops[stops.length - 1]!;
    for (let s = 0; s < stops.length - 1; s++) {
      const a = stops[s]!;
      const b = stops[s + 1]!;
      if (t >= a.t && t <= b.t) {
        lo = a;
        hi = b;
        break;
      }
    }
    const span = hi.t - lo.t || 1;
    const f = (t - lo.t) / span;
    lut[i * 3] = lo.rgb[0] + (hi.rgb[0] - lo.rgb[0]) * f;
    lut[i * 3 + 1] = lo.rgb[1] + (hi.rgb[1] - lo.rgb[1]) * f;
    lut[i * 3 + 2] = lo.rgb[2] + (hi.rgb[2] - lo.rgb[2]) * f;
  }
  return lut;
}

function buildGrayLut(): Uint8ClampedArray {
  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i++) {
    lut[i * 3] = i;
    lut[i * 3 + 1] = i;
    lut[i * 3 + 2] = i;
  }
  return lut;
}

const LUTS: Record<ColormapName, Uint8ClampedArray> = {
  inferno: buildLut(INFERNO_STOPS),
  viridis: buildLut(VIRIDIS_STOPS),
  gray: buildGrayLut(),
};

/** The full 256×3 LUT for `name` (SPEC-007 §2.5: "a 256-entry RGB LUT"). */
export function colormapLut(name: ColormapName): Uint8ClampedArray {
  return LUTS[name];
}

function clamp01(x: number): number {
  return Number.isFinite(x) ? Math.max(0, Math.min(1, x)) : 0;
}

/** `t ∈ [0, 1]` -> `[r, g, b]` (0-255), quantized to the 256-entry LUT (SPEC-007 §2.5). */
export function colorForT(name: ColormapName, t: number): [number, number, number] {
  const lut = LUTS[name];
  const idx = Math.round(clamp01(t) * 255);
  return [lut[idx * 3]!, lut[idx * 3 + 1]!, lut[idx * 3 + 2]!];
}

/**
 * Normalizes a dequantized dB level against the display floor/ceiling (SPEC-007 §2.5): `≤ floor`
 * maps to 0 (the colormap's first color), `≥ ceiling` to 1 (its last color).
 */
export function normalizeDb(db: number, floorDb: number, ceilDb: number): number {
  if (!(ceilDb > floorDb)) {
    return 0;
  }
  return clamp01((db - floorDb) / (ceilDb - floorDb));
}

/**
 * A CSS `linear-gradient()` sampling `name`'s LUT at `stops` even steps (H-24 item 6: "a small
 * colour-bar legend with its dB range"). Low `t` (floor) first, so the caller can lay the bar out
 * floor-to-ceiling in either direction with `to right`/`to top`.
 */
export function cssGradientFor(name: ColormapName, stops = 8): string {
  const n = Math.max(2, stops);
  const parts: string[] = [];
  for (let i = 0; i < n; i++) {
    const t = i / (n - 1);
    const [r, g, b] = colorForT(name, t);
    const pct = (t * 100).toFixed(1);
    parts.push(`rgb(${r}, ${g}, ${b}) ${pct}%`);
  }
  return `linear-gradient(to right, ${parts.join(", ")})`;
}
