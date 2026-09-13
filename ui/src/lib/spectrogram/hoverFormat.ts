/**
 * Hover-readout level formatting (SPEC-007 §2.7, AC-3, AC-13): code 0 (digital silence / below
 * the tile's quantization floor) shows "≤ −150 dB", code 255 shows "≥ +6 dB", otherwise the
 * dequantized level with one decimal, e.g. "−20.3 dB". Uses the proper minus sign (U+2212) per
 * the spec's own examples.
 */

import { Q_CEIL_DB, Q_FLOOR_DB, dequantizeDb } from "./vxst";

const MINUS = "−";

function signedInt(db: number): string {
  const n = Math.round(db);
  if (n === 0) {
    return "0";
  }
  return n > 0 ? `+${n}` : `${MINUS}${Math.abs(n)}`;
}

function signedFixed1(db: number): string {
  const n = Math.round(db * 10) / 10;
  const abs = Math.abs(n).toFixed(1);
  if (n === 0) {
    return "0.0";
  }
  return n > 0 ? `+${abs}` : `${MINUS}${abs}`;
}

/**
 * `code` is the raw `VXST` quantization code (0-255) of the nearest bin in the nearest frame
 * ({@link import("./sampler").nearestCode}), or `null` when no tile has arrived there yet — the
 * caller substitutes its own "no data" i18n string in that case (SPEC-007 §2.7: "It shows '—'
 * where no tile has arrived yet").
 */
export function formatLevelDb(code: number | null): string | null {
  if (code === null) {
    return null;
  }
  if (code <= 0) {
    return `≤ ${signedInt(Q_FLOOR_DB)} dB`;
  }
  if (code >= 255) {
    return `≥ ${signedInt(Q_CEIL_DB)} dB`;
  }
  return `${signedFixed1(dequantizeDb(code))} dB`;
}
