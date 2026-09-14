import { describe, expect, it } from "vitest";
import { formatRulerTime } from "./timeRuler";

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
});
