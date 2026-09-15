import { describe, expect, it } from "vitest";
import {
  formatDocumentTime,
  formatSamplesValue,
  formatSecondsValue,
  parseDocumentTime,
} from "./timeFormat";

describe("formatDocumentTime / parseDocumentTime (SPEC-006 §2.5)", () => {
  const RATES = [44_100, 48_000, 96_000];

  it("samples: formats the exact integer sample count", () => {
    expect(formatSamplesValue(0)).toBe("0");
    expect(formatSamplesValue(172_800_000)).toBe("172800000");
    expect(formatDocumentTime(48_000, 48_000, "samples")).toBe("48000");
  });

  it("samples: round-trips exactly for a range of sample counts", () => {
    for (const samples of [0, 1, 512, 48_000, 172_800_000, 4_000_000_000]) {
      const text = formatDocumentTime(samples, 48_000, "samples");
      expect(parseDocumentTime(text, 48_000, "samples")).toBe(samples);
    }
  });

  it("seconds: round-trips to the exact sample at every tested rate", () => {
    for (const rateHz of RATES) {
      for (const samples of [0, 1, rateHz - 1, rateHz, rateHz * 3661 + 17, 172_800_000]) {
        const text = formatDocumentTime(samples, rateHz, "seconds");
        expect(parseDocumentTime(text, rateHz, "seconds")).toBe(samples);
      }
    }
  });

  it("seconds: formats with 6 decimals", () => {
    expect(formatSecondsValue(48_000, 48_000)).toBe("1.000000");
    expect(formatSecondsValue(24_000, 48_000)).toBe("0.500000");
  });

  it("timecode: matches transport/playhead.ts::formatTime exactly (unchanged default)", () => {
    expect(formatDocumentTime(0, 48_000, "timecode")).toBe("00:00:00.000");
    expect(formatDocumentTime(48_000 * 3661.5, 48_000, "timecode")).toBe("01:01:01.500");
  });

  it("timecode: format -> parse -> format -> parse is a stable fixed point (its own display limit is milliseconds, so raw sample counts aren't bit-exact, but editing settles)", () => {
    for (const rateHz of RATES) {
      for (const samples of [0, 1, rateHz - 1, rateHz, rateHz * 3661 + 17, 172_800_000]) {
        const text1 = formatDocumentTime(samples, rateHz, "timecode");
        const parsed1 = parseDocumentTime(text1, rateHz, "timecode");
        expect(parsed1).not.toBeNull();
        const text2 = formatDocumentTime(parsed1 as number, rateHz, "timecode");
        const parsed2 = parseDocumentTime(text2, rateHz, "timecode");
        expect(parsed2).toBe(parsed1);
        expect(text2).toBe(text1);
      }
    }
  });

  it("timecode: a concrete millisecond-exact value round-trips exactly", () => {
    const samples = Math.round(3661.5 * 48_000); // 01:01:01.500
    const text = formatDocumentTime(samples, 48_000, "timecode");
    expect(text).toBe("01:01:01.500");
    expect(parseDocumentTime(text, 48_000, "timecode")).toBe(samples);
  });

  it("timecode: parses shorter mm:ss and ss forms too", () => {
    expect(parseDocumentTime("1:02.500", 48_000, "timecode")).toBe(
      Math.round(62.5 * 48_000),
    );
    expect(parseDocumentTime("5", 48_000, "timecode")).toBe(5 * 48_000);
  });

  it("rejects garbage and out-of-range input in every format", () => {
    expect(parseDocumentTime("not a time", 48_000, "timecode")).toBeNull();
    expect(parseDocumentTime("00:60:00.000", 48_000, "timecode")).toBeNull();
    expect(parseDocumentTime("abc", 48_000, "samples")).toBeNull();
    expect(parseDocumentTime("-5", 48_000, "samples")).toBeNull();
    expect(parseDocumentTime("1.5", 0, "seconds")).toBeNull();
    expect(parseDocumentTime("", 48_000, "samples")).toBeNull();
  });
});
