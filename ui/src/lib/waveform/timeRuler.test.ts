import { describe, expect, it } from "vitest";
import { niceTickStepSeconds, timeTicks } from "./coords";
import { formatRulerSeconds, formatRulerTime } from "./timeRuler";

describe("formatRulerTime (H-24 item 7: compact, zoom-adaptive ruler labels)", () => {
  it("shows m:ss with no decimals at a 1-second-or-coarser step", () => {
    expect(formatRulerTime(5, 1, false)).toBe("0:05");
    expect(formatRulerTime(65, 5, false)).toBe("1:05");
    expect(formatRulerTime(0, 10, false)).toBe("0:00");
  });

  it("shows milliseconds (3 decimals) at any step below 1 second", () => {
    expect(formatRulerTime(5.25, 0.25, false)).toBe("0:05.250");
    expect(formatRulerTime(5.25, 0.1, false)).toBe("0:05.250");
    expect(formatRulerTime(5.001, 0.001, false)).toBe("0:05.001");
  });

  it("shows the hour group once the document is >= 1 hour, even at 0", () => {
    expect(formatRulerTime(3723, 1, true)).toBe("1:02:03");
    expect(formatRulerTime(0, 1, true)).toBe("0:00:00");
  });

  it("shows the hour group once elapsed time itself crosses 1 hour, even if includeHours is false", () => {
    expect(formatRulerTime(3723, 1, false)).toBe("1:02:03");
  });

  it("never shows a negative time", () => {
    expect(formatRulerTime(-5, 1, false)).toBe("0:00");
  });

  it("rounds at the label's own precision without corrupting the carry", () => {
    // 59.9996 rounded to 3 decimals is 60.000 -> rolls over to the next minute cleanly.
    expect(formatRulerTime(59.9996, 0.001, false)).toBe("1:00.000");
  });

  // H-60 (SPEC-006 §2.6 AC-4/§4.2, extreme zoom): below a 1 ms step, a fixed 3-decimal label
  // can't tell consecutive ticks apart — this reproduces the exact scenario at the documented
  // `samplesPerPixel` zoom ceiling (0.1) on a 48 kHz document with a 70 px minimum label gap
  // (`EditorView.svelte`'s own constant), which used to print "0:00.001" for three ticks in a
  // row.
  it("never collides at sub-millisecond tick steps (extreme zoom)", () => {
    const step = niceTickStepSeconds((70 * 0.1) / 48_000);
    expect(step).toBeLessThan(0.001); // confirms this test actually exercises the extreme case
    const seconds = [0, step, 2 * step, 3 * step, 4 * step, 5 * step];
    const labels = seconds.map((s) => formatRulerTime(s, step, false));
    expect(new Set(labels).size).toBe(labels.length);
  });

  it("shows more than 3 decimals only once the step needs them, and never more than 6", () => {
    expect(formatRulerTime(0.0002, 0.0002, false)).toBe("0:00.0002");
    expect(formatRulerTime(0.00002, 0.00002, false)).toBe("0:00.00002");
    // Far below any realistic step (SPEC-005 doc_rate_range_hz tops out at 384 kHz): still capped.
    expect(formatRulerTime(0.0000001, 0.0000001, false)).toBe("0:00.000000");
  });
});

describe("timeTicks integration (H-60): every extreme-zoom tick label is unique", () => {
  it("48 kHz at the samplesPerPixel floor (0.1) produces distinct labels for every visible tick", () => {
    const sampleRateHz = 48_000;
    const samplesPerPixel = 0.1;
    const viewportPx = 800;
    const step = niceTickStepSeconds((70 * samplesPerPixel) / sampleRateHz);
    const ticks = timeTicks(0, samplesPerPixel, viewportPx, sampleRateHz, 70);
    const labels = ticks.map((t) => formatRulerTime(t.seconds, step, false));
    expect(new Set(labels).size).toBe(labels.length);
  });

  it("384 kHz (the fastest accepted document rate) at the samplesPerPixel floor also stays distinct", () => {
    const sampleRateHz = 384_000;
    const samplesPerPixel = 0.1;
    const viewportPx = 800;
    const step = niceTickStepSeconds((70 * samplesPerPixel) / sampleRateHz);
    const ticks = timeTicks(0, samplesPerPixel, viewportPx, sampleRateHz, 70);
    const labels = ticks.map((t) => formatRulerSeconds(t.seconds, step));
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("formatRulerSeconds (T-206, SPEC-006 §2.5 'seconds' format)", () => {
  it("plain decimal seconds, no h:m:s grouping", () => {
    expect(formatRulerSeconds(65, 5)).toBe("65");
    expect(formatRulerSeconds(3723, 1)).toBe("3723");
  });

  it("0 decimals at a 1-second-or-coarser step, milliseconds below that", () => {
    expect(formatRulerSeconds(5, 1)).toBe("5");
    expect(formatRulerSeconds(5.25, 0.25)).toBe("5.250");
    expect(formatRulerSeconds(5.001, 0.001)).toBe("5.001");
  });

  it("never shows a negative time", () => {
    expect(formatRulerSeconds(-5, 1)).toBe("0");
  });
});
