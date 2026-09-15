import { describe, expect, it } from "vitest";
import { formatCents, formatNote, noteForFreq, noteName } from "./notes";

describe("note naming (H-42)", () => {
  it("names reference pitches exactly", () => {
    expect(noteForFreq(440)).toEqual({ name: "A", octave: 4, cents: 0, midi: 69 });
    expect(noteName(noteForFreq(261.6256)!)).toBe("C4");
    expect(noteName(noteForFreq(220)!)).toBe("A3");
    expect(noteName(noteForFreq(27.5)!)).toBe("A0");
    expect(noteName(noteForFreq(4186.01)!)).toBe("C8");
    expect(noteName(noteForFreq(277.18)!)).toBe("C#4");
  });

  it("measures cents to the nearest note", () => {
    // +12 cents above A3.
    const n = noteForFreq(220 * 2 ** (12 / 1200))!;
    expect(noteName(n)).toBe("A3");
    expect(n.cents).toBe(12);
    // 30 cents below B3 is still B3.
    const b = noteForFreq(246.9417 * 2 ** (-30 / 1200))!;
    expect(noteName(b)).toBe("B3");
    expect(b.cents).toBe(-30);
    // 60 cents above A3 is 40 cents below A#3.
    const up = noteForFreq(220 * 2 ** (60 / 1200))!;
    expect(noteName(up)).toBe("A#3");
    expect(up.cents).toBe(-40);
    // Across the B→C octave boundary.
    const c = noteForFreq(261.6256 * 2 ** (-20 / 1200))!;
    expect(noteName(c)).toBe("C4");
    expect(c.cents).toBe(-20);
  });

  it("keeps cents within ±50", () => {
    for (let f = 30; f < 16_000; f *= 1.013) {
      const n = noteForFreq(f)!;
      expect(Math.abs(n.cents)).toBeLessThanOrEqual(50);
      const back = 440 * 2 ** ((n.midi - 69) / 12 + n.cents / 1200);
      expect(Math.abs(1200 * Math.log2(back / f))).toBeLessThan(0.51);
    }
  });

  it("rejects non-frequencies", () => {
    expect(noteForFreq(0)).toBeNull();
    expect(noteForFreq(-5)).toBeNull();
    expect(noteForFreq(Number.NaN)).toBeNull();
    expect(formatNote(0)).toBe("");
  });

  it("formats the label text", () => {
    expect(formatCents(12)).toBe("+12");
    expect(formatCents(-7)).toBe("−7");
    expect(formatCents(0)).toBe("±0");
    expect(formatNote(220 * 2 ** (12 / 1200))).toBe("A3 +12¢");
  });
});
