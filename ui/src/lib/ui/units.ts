/**
 * Number ↔ text for every value the UI shows with a unit (H-25 design system: "units always
 * shown, numbers in tabular figures"). Pure; used by NumberField, Slider and Readout.
 *
 * Typography rules: negative values use the true minus sign U+2212 (same width as `+` in
 * tabular figures, so columns of dB values align); value and unit are joined by a no-break space
 * so "−23.0 LUFS" never wraps between number and unit; `%` attaches directly.
 */

export const MINUS = "−";
const NBSP = " ";

export interface FormatOptions {
  /** Show `+` on positive values (gains, offsets). Zero never gets a sign. */
  signed?: boolean;
}

export function formatNumber(value: number, decimals: number, options: FormatOptions = {}): string {
  if (Number.isNaN(value)) {
    return "—";
  }
  if (value === Number.NEGATIVE_INFINITY) {
    return `${MINUS}∞`;
  }
  if (value === Number.POSITIVE_INFINITY) {
    return "+∞";
  }
  const fixed = Math.abs(value).toFixed(decimals);
  // Rounds to zero → no sign at all (never "−0.0").
  if (Number(fixed) === 0) {
    return fixed;
  }
  if (value < 0) {
    return `${MINUS}${fixed}`;
  }
  return options.signed ? `+${fixed}` : fixed;
}

export function formatWithUnit(
  value: number,
  unit: string,
  decimals: number,
  options: FormatOptions = {},
): string {
  const text = formatNumber(value, decimals, options);
  if (unit === "") {
    return text;
  }
  return unit === "%" ? `${text}%` : `${text}${NBSP}${unit}`;
}

const UNIT_SUFFIX = /\s*(dbfs|dbtp|db|lufs|lu|ms|s|khz|hz|smp|samples|%)$/i;

/**
 * Parses user input into a number: accepts an ASCII `-`, the U+2212 minus `formatNumber` writes, `+`, a decimal point or comma, surrounding
 * spaces and a trailing unit. `kHz` is scaled to Hz when `baseUnit` is `"Hz"`. Returns `null` for
 * anything that isn't exactly one number.
 */
export function parseNumber(input: string, baseUnit = ""): number | null {
  let text = input.trim();
  let scale = 1;
  const unit = UNIT_SUFFIX.exec(text);
  if (unit) {
    if (unit[1]!.toLowerCase() === "khz" && baseUnit.toLowerCase() === "hz") {
      scale = 1000;
    }
    text = text.slice(0, unit.index).trim();
  }
  // U+2212 minus (what `formatNumber` writes), en/figure dashes and the full-width hyphen all
  // read as a sign, so a value copied from a readout parses back.
  text = text.replace(/^[\u2212\u2012\u2013\uFE63\uFF0D]/, "-").replace(",", ".");
  if (!/^[+-]?(\d+\.?\d*|\.\d+)$/.test(text)) {
    return null;
  }
  const value = Number(text) * scale;
  return Number.isFinite(value) ? value : null;
}
