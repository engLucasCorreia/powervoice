import { describe, expect, it } from "vitest";
import {
  INSERT_SILENCE_MIN_SAMPLES,
  insertSilenceInRange,
  insertSilenceMaxSamples,
  parseInsertSilenceDuration,
} from "./insertSilenceDuration";

const RATE = 48_000;

describe("parseInsertSilenceDuration (H-56, SPEC-008 §2.5)", () => {
  it("parses plain seconds", () => {
    expect(parseInsertSilenceDuration("1", RATE)).toBe(48_000);
    expect(parseInsertSilenceDuration("1.5", RATE)).toBe(72_000);
    expect(parseInsertSilenceDuration("0.250", RATE)).toBe(12_000);
  });

  it("parses plain seconds at 44.1 kHz", () => {
    expect(parseInsertSilenceDuration("1.000", 44_100)).toBe(44_100);
    expect(parseInsertSilenceDuration("0.5", 44_100)).toBe(22_050);
  });

  it("parses timecode [[hh:]mm:]ss[.fff]", () => {
    expect(parseInsertSilenceDuration("0:01.500", RATE)).toBe(72_000);
    expect(parseInsertSilenceDuration("00:00:02", RATE)).toBe(96_000);
  });

  it("parses an integer sample count with an `smp` suffix, unrounded", () => {
    expect(parseInsertSilenceDuration("480 smp", RATE)).toBe(480);
    expect(parseInsertSilenceDuration("480smp", RATE)).toBe(480);
    expect(parseInsertSilenceDuration("48000 SMP", RATE)).toBe(48_000);
  });

  it("rejects unparseable text", () => {
    expect(parseInsertSilenceDuration("abc", RATE)).toBeNull();
    expect(parseInsertSilenceDuration("", RATE)).toBeNull();
    expect(parseInsertSilenceDuration("   ", RATE)).toBeNull();
  });
});

describe("insertSilenceInRange (SPEC-008 §2.5: 1 sample .. 3 600 s)", () => {
  it("accepts the bounds and rejects outside them", () => {
    expect(insertSilenceInRange(INSERT_SILENCE_MIN_SAMPLES, RATE)).toBe(true);
    expect(insertSilenceInRange(0, RATE)).toBe(false);
    expect(insertSilenceInRange(insertSilenceMaxSamples(RATE), RATE)).toBe(true);
    expect(insertSilenceInRange(insertSilenceMaxSamples(RATE) + 1, RATE)).toBe(false);
  });

  it("the dialog rejects '0', '3600.001' and 'abc'; accepts '3600'", () => {
    const zero = parseInsertSilenceDuration("0", RATE);
    expect(zero === null || !insertSilenceInRange(zero, RATE)).toBe(true);

    const tooLong = parseInsertSilenceDuration("3600.001", RATE);
    expect(tooLong === null || !insertSilenceInRange(tooLong, RATE)).toBe(true);

    expect(parseInsertSilenceDuration("abc", RATE)).toBeNull();

    const exactlyOneHour = parseInsertSilenceDuration("3600", RATE);
    expect(exactlyOneHour).not.toBeNull();
    expect(insertSilenceInRange(exactlyOneHour!, RATE)).toBe(true);
  });
});
