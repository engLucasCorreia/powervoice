import { describe, expect, it } from "vitest";
import { ClockSync, PlayheadExtrapolator, extrapolate, formatTime } from "./playhead";

const MS = 1e6;

describe("extrapolate (SPEC-003 §2.2)", () => {
  it("advances at the rate from the anchor, clamped to the document", () => {
    const anchor = { sample: 48_000, timeNs: 1_000 * MS, rate: 48_000 };
    expect(extrapolate(anchor, 1_500 * MS, 1e9)).toBe(72_000);
    expect(extrapolate(anchor, 10_000 * MS, 100_000)).toBe(100_000);
    expect(extrapolate(anchor, 900 * MS, 1e9)).toBe(48_000);
    expect(extrapolate({ ...anchor, rate: 0 }, 5_000 * MS, 1e9)).toBe(48_000);
  });
});

describe("PlayheadExtrapolator", () => {
  it("slews corrections under 20 ms of audio over 100 ms and jumps larger ones", () => {
    const p = new PlayheadExtrapolator();
    p.update({ sample: 0, timeNs: 0, rate: 48_000 }, 0, 1e9);
    expect(p.position(100 * MS, 1e9)).toBe(4_800);
    // 5 ms behind the prediction: slewed, no visible jump.
    p.update({ sample: 4_560, timeNs: 100 * MS, rate: 48_000 }, 100 * MS, 1e9);
    expect(p.position(100 * MS, 1e9)).toBeCloseTo(4_800);
    expect(p.position(150 * MS, 1e9)).toBeCloseTo(4_560 + 2_400 + 120);
    expect(p.position(200 * MS, 1e9)).toBeCloseTo(4_560 + 4_800);
    // A seek: jump.
    p.update({ sample: 96_000, timeNs: 200 * MS, rate: 48_000 }, 200 * MS, 1e9);
    expect(p.position(200 * MS, 1e9)).toBe(96_000);
    // Stopped: jump to the stopped position.
    p.update({ sample: 1_000, timeNs: 250 * MS, rate: 0 }, 250 * MS, 1e9);
    expect(p.position(400 * MS, 1e9)).toBe(1_000);
  });
});

describe("ClockSync (ADR-003 §3)", () => {
  it("keeps the offset of the smallest round trip", async () => {
    let now = 0;
    const sync = new ClockSync(() => now);
    const rtts = [10, 2, 6];
    let i = 0;
    const trueOffsetNs = 5e9;
    await sync.sync(async () => {
      const rtt = rtts[i++] ?? 1;
      // The server answers late in the round trip: the error grows with the RTT.
      const serverNs = (now + rtt * 0.8) * 1e6 + trueOffsetNs;
      now += rtt;
      return serverNs;
    }, 3);
    expect(sync.offsetNs).toBeCloseTo(trueOffsetNs + 0.3 * 2 * 1e6);
    now = 100;
    expect(sync.nowNs()).toBeCloseTo(100 * 1e6 + sync.offsetNs);
  });
});

describe("formatTime", () => {
  it("formats hh:mm:ss.fff", () => {
    expect(formatTime(0, 0)).toBe("00:00:00.000");
    expect(formatTime(48_000 * 3661.5, 48_000)).toBe("01:01:01.500");
    expect(formatTime(44_100, 44_100)).toBe("00:00:01.000");
  });
});
