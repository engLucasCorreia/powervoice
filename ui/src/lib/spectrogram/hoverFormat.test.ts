import { describe, expect, it } from "vitest";
import { formatLevelDb } from "./hoverFormat";
import { dequantizeDb } from "./vxst";

describe("formatLevelDb (SPEC-007 §2.7, AC-3, AC-13)", () => {
  it("code 0 shows the floor text, exactly", () => {
    expect(formatLevelDb(0)).toBe("≤ −150 dB");
  });

  it("code 255 shows the ceiling text, exactly", () => {
    expect(formatLevelDb(255)).toBe("≥ +6 dB");
  });

  it("null (no tile yet) returns null so the caller can show its own placeholder", () => {
    expect(formatLevelDb(null)).toBeNull();
  });

  it("a normal code shows one decimal, matching the dequantized value (AC-13: exact to 0.1 dB)", () => {
    // Code 43 -> dequantizeDb(43) is some negative dB in the middle of the range.
    const db = dequantizeDb(43);
    const expected = `${Math.round(db * 10) / 10}`;
    expect(formatLevelDb(43)).toBe(`${expected.replace("-", "−")} dB`);
  });

  it("uses the proper minus sign U+2212, not a hyphen", () => {
    const text = formatLevelDb(43)!;
    expect(text).toContain("−");
    expect(text).not.toContain("-");
  });

  it("a positive level gets an explicit + sign", () => {
    // Near code 255 but not quite there dequantizes to a positive dB.
    expect(formatLevelDb(254)).toMatch(/^\+/);
  });
});
