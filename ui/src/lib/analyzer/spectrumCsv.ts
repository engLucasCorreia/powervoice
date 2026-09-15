/**
 * The Spectrum Inspector's CSV export (H-42, SPEC-007 §8.3): the displayed curve (after
 * smoothing) inside the visible frequency range, one row per point. Machine-readable on purpose:
 * `.` decimals, ASCII `-`, fixed column names, an empty cell for silence (−∞), whatever the UI
 * language — spreadsheets and plotting scripts read it as is.
 */

export interface CsvCurve {
  freqsHz: ArrayLike<number>;
  levelsDb: ArrayLike<number>;
}

export const CSV_HEADER = "frequency_hz,level_db";

export function spectrumCsv(curve: CsvCurve, fLoHz = 0, fHiHz = Infinity): string {
  const rows = [CSV_HEADER];
  const n = Math.min(curve.freqsHz.length, curve.levelsDb.length);
  for (let i = 0; i < n; i++) {
    const f = curve.freqsHz[i] ?? 0;
    if (f < fLoHz || f > fHiHz) {
      continue;
    }
    const db = curve.levelsDb[i] ?? -Infinity;
    rows.push(`${f.toFixed(2)},${Number.isFinite(db) ? db.toFixed(2) : ""}`);
  }
  return `${rows.join("\n")}\n`;
}
