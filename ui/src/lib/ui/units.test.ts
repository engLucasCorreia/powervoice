import { describe, expect, it } from "vitest";
import { formatNumber, formatWithUnit, parseNumber } from "./units";

describe("formatNumber", () => {
  it("uses a true minus sign (U+2212) for negatives and fixed decimals", () => {
    expect(formatNumber(-3, 1)).toBe("−3.0");
    expect(formatNumber(12.345, 2)).toBe("12.35");
    expect(formatNumber(0, 0)).toBe("0");
  });

  it("never shows negative zero", () => {
    expect(formatNumber(-0.04, 1)).toBe("0.0");
    expect(formatNumber(-0, 2)).toBe("0.00");
  });

  it("shows −∞ / +∞ for infinities and an em dash for NaN", () => {
    expect(formatNumber(Number.NEGATIVE_INFINITY, 1)).toBe("−∞");
    expect(formatNumber(Number.POSITIVE_INFINITY, 1)).toBe("+∞");
    expect(formatNumber(Number.NaN, 1)).toBe("—");
  });

  it("can force a plus sign for gains", () => {
    expect(formatNumber(3, 1, { signed: true })).toBe("+3.0");
    expect(formatNumber(0, 1, { signed: true })).toBe("0.0");
  });
});

describe("formatWithUnit", () => {
  it("joins value and unit with a no-break space", () => {
    expect(formatWithUnit(-23, "LUFS", 1)).toBe("−23.0 LUFS");
    expect(formatWithUnit(48000, "Hz", 0)).toBe("48000 Hz");
  });

  it("omits the separator for % and an empty unit", () => {
    expect(formatWithUnit(50, "%", 0)).toBe("50%");
    expect(formatWithUnit(2, "", 1)).toBe("2.0");
  });
});

describe("parseNumber", () => {
  it("accepts ASCII and Unicode minus, plus and leading/trailing spaces", () => {
    expect(parseNumber(" -3.5 ")).toBe(-3.5);
    expect(parseNumber("−18")).toBe(-18);
    expect(parseNumber("+6")).toBe(6);
  });

  it("accepts a decimal comma", () => {
    expect(parseNumber("-0,5")).toBe(-0.5);
  });

  it("strips a trailing unit (dB, dBFS, LUFS, ms, Hz, kHz, %)", () => {
    expect(parseNumber("-1 dB")).toBe(-1);
    expect(parseNumber("-23.0 LUFS")).toBe(-23);
    expect(parseNumber("150ms")).toBe(150);
    expect(parseNumber("2.5 kHz", "Hz")).toBe(2500);
    expect(parseNumber("50 %")).toBe(50);
  });

  it("rejects empty and garbage input", () => {
    expect(parseNumber("")).toBeNull();
    expect(parseNumber("abc")).toBeNull();
    expect(parseNumber("1.2.3")).toBeNull();
    expect(parseNumber("--3")).toBeNull();
  });
});
