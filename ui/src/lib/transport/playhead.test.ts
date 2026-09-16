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

/**
 * H-59: SPEC-003 AC-6's composed bound. The pieces above test the extrapolation formula and
 * `ClockSync`'s min-RTT arithmetic in isolation; the AC is about the *combination* — "the
 * displayed position is within ±10 ms of the true heard position at any instant between two
 * telemetry frames", a bound dominated by clock-offset error, not by the telemetry period. So
 * this drives both together against a simulated engine clock with a real offset and jittery
 * round trips, exactly as SPEC-003 §6's test plan describes.
 */
describe("playhead accuracy under clock-sync jitter (SPEC-003 AC-6)", () => {
  const RATE = 48_000;
  const LEN = 60 * RATE;
  /** The UI's `performance.now()` and the engine's app clock differ by this much. */
  const TRUE_OFFSET_NS = 12_345.678 * MS;

  /** A seeded LCG, so a flaky run is reproducible. */
  function rng(seed: number): () => number {
    let s = seed >>> 0;
    return () => {
      s = (s * 1_664_525 + 1_013_904_223) >>> 0;
      return s / 2 ** 32;
    };
  }

  it("stays within 10 ms of the true position at every instant between telemetry frames", async () => {
    const random = rng(7);
    let uiNowMs = 1000;
    const serverNsAtUiMs = (ms: number): number => ms * 1e6 + TRUE_OFFSET_NS;

    const clock = new ClockSync(() => uiNowMs);
    // Five samples with round trips of 0.5-8 ms, the server answering at a random point inside
    // each one (so the midpoint estimate is wrong by up to half the RTT).
    await clock.sync(async () => {
      const rtt = 0.5 + random() * 7.5;
      const answerAt = uiNowMs + rtt * random();
      uiNowMs += rtt;
      return serverNsAtUiMs(answerAt);
    }, 5);

    const player = new PlayheadExtrapolator();
    // 60 Hz telemetry: an anchor every ~16.67 ms, each stamped one output period in the past and
    // delivered with its own jitter, which is what the extrapolator has to absorb.
    const framePeriodMs = 1000 / 60;
    let worstErrorSamples = 0;
    for (let frame = 0; frame < 600; frame++) {
      const arrivalMs = 1000 + frame * framePeriodMs + random() * 3;
      uiNowMs = arrivalMs;
      // The engine stamped this anchor 5 ms of output latency ago.
      const anchorUiMs = arrivalMs - 5;
      const anchorSample = Math.round(((anchorUiMs - 1000) / 1000) * RATE);
      player.update(
        { sample: anchorSample, timeNs: serverNsAtUiMs(anchorUiMs), rate: RATE },
        clock.nowNs(),
        LEN,
      );

      // Sample the display at arbitrary instants until the next frame is due.
      for (let k = 0; k < 8; k++) {
        uiNowMs = arrivalMs + (k / 8) * framePeriodMs;
        const truePosition = ((uiNowMs - 5 - 1000) / 1000) * RATE;
        const shown = player.position(clock.nowNs(), LEN);
        worstErrorSamples = Math.max(worstErrorSamples, Math.abs(shown - truePosition));
      }
    }

    const worstErrorMs = (worstErrorSamples / RATE) * 1000;
    // ~5.3 ms on this seed: real headroom under the bound, but not a vacuous assertion — most of
    // it is the 100 ms slew converging after each anchor.
    expect(worstErrorMs).toBeLessThanOrEqual(10);
    expect(worstErrorMs).toBeGreaterThan(1);
  });
});
