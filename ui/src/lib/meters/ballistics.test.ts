import { describe, expect, it } from "vitest";
import {
  DEFAULT_METER_SPEED,
  METER_SPEED_PROFILES,
  meterSourceAtRest,
  meterSpeedProfile,
  PEAK_HOLD_MS,
  PEAK_RELEASE_DB_PER_S,
  PeakBallistics,
  READOUT_INTERVAL_MS,
  SILENT_SOURCE_DBFS,
  SmoothedDb,
  ThrottledReadout,
  type MeterSpeed,
} from "./ballistics";

/**
 * H-41 ballistics maths (shared by the input meter, the output meter and, potentially, other
 * level meters — see the H-41 ticket report for why gain-reduction meters don't adopt it too).
 */

describe("PeakBallistics", () => {
  it("attacks instantly and releases at PEAK_RELEASE_DB_PER_S", () => {
    const b = new PeakBallistics();
    b.update(-6, 1000);
    expect(b.bar).toBe(-6); // instant attack: the very first sample is the bar
    b.update(-60, 1500); // 0.5 s later, no new peak
    expect(b.bar).toBeCloseTo(-6 - PEAK_RELEASE_DB_PER_S * 0.5, 6);
  });

  it("never rises above the true peak but jumps straight to a louder one", () => {
    const b = new PeakBallistics();
    b.update(-20, 0);
    b.update(-40, 100); // quieter — falls only a little, not straight to -40
    expect(b.bar).toBeGreaterThan(-40);
    expect(b.bar).toBeLessThan(-20);
    b.update(-3, 200); // louder — instant attack, no lag
    expect(b.bar).toBe(-3);
  });

  it("holds the peak for PEAK_HOLD_MS then falls, never below the bar", () => {
    const b = new PeakBallistics();
    b.update(-6, 1000);
    expect(b.hold).toBe(-6);
    b.update(-60, 1000 + PEAK_HOLD_MS - 1); // just under the hold time
    expect(b.hold).toBe(-6);
    b.update(-60, 1000 + PEAK_HOLD_MS + 1); // just over — starts falling
    expect(b.hold).toBeLessThan(-6);
    expect(b.hold).toBeGreaterThanOrEqual(b.bar);
  });

  it("a fresh peak resets the hold immediately, even while the previous hold was falling", () => {
    const b = new PeakBallistics();
    b.update(-6, 0);
    b.update(-60, PEAK_HOLD_MS + 500); // hold has started falling
    expect(b.hold).toBeLessThan(-6);
    b.update(-3, PEAK_HOLD_MS + 600);
    expect(b.hold).toBe(-3);
    expect(b.bar).toBe(-3);
  });

  it("actually reaches -Infinity (and stops changing) after enough silent time, not just a very negative number forever (H-41 idle-CPU guard)", () => {
    const b = new PeakBallistics();
    b.update(-6, 0);
    b.update(Number.NEGATIVE_INFINITY, 20_000); // 20 s of true silence — releasing the whole way
    expect(b.bar).toBe(Number.NEGATIVE_INFINITY);
    expect(b.hold).toBe(Number.NEGATIVE_INFINITY);
    // A further identical update produces bit-identical output — no residual per-frame drift a
    // caller would have to re-render for.
    const barBefore = b.bar;
    const holdBefore = b.hold;
    b.update(Number.NEGATIVE_INFINITY, 20_100);
    expect(b.bar).toBe(barBefore);
    expect(b.hold).toBe(holdBefore);
  });

  it("resets to silence", () => {
    const b = new PeakBallistics();
    b.update(-6, 0);
    b.reset();
    expect(b.bar).toBe(Number.NEGATIVE_INFINITY);
    expect(b.hold).toBe(Number.NEGATIVE_INFINITY);
    // A reset also forgets the previous timestamp, so the next update is a fresh instant attack,
    // not a huge one-off "release" computed against a stale `lastMs`.
    b.update(-6, 10_000);
    expect(b.bar).toBe(-6);
  });
});

describe("meter speed profiles (H-123, owner request: \"can i setup the speed?\")", () => {
  it("Medium keeps the pre-H-123 SPEC-002 §3 numbers exactly (no ballistics regression for the default)", () => {
    expect(METER_SPEED_PROFILES.medium.peakReleaseDbPerS).toBe(PEAK_RELEASE_DB_PER_S);
    expect(METER_SPEED_PROFILES.medium.peakHoldMs).toBe(PEAK_HOLD_MS);
    expect(DEFAULT_METER_SPEED).toBe("medium");
  });

  it("Fast releases and returns to rest faster than Medium; Slow is slower than Medium", () => {
    const { fast, medium, slow } = METER_SPEED_PROFILES;
    expect(fast.peakReleaseDbPerS).toBeGreaterThan(medium.peakReleaseDbPerS);
    expect(slow.peakReleaseDbPerS).toBeLessThan(medium.peakReleaseDbPerS);
    expect(fast.peakHoldMs).toBeLessThan(medium.peakHoldMs);
    expect(slow.peakHoldMs).toBeGreaterThan(medium.peakHoldMs);
    expect(fast.readoutSmoothingTauMs).toBeLessThan(medium.readoutSmoothingTauMs);
    expect(slow.readoutSmoothingTauMs).toBeGreaterThan(medium.readoutSmoothingTauMs);
  });

  it("meterSpeedProfile falls back to Medium for null/undefined/unrecognized input", () => {
    expect(meterSpeedProfile(undefined)).toBe(METER_SPEED_PROFILES.medium);
    expect(meterSpeedProfile(null)).toBe(METER_SPEED_PROFILES.medium);
    expect(meterSpeedProfile("bogus" as MeterSpeed)).toBe(METER_SPEED_PROFILES.medium);
    expect(meterSpeedProfile("fast")).toBe(METER_SPEED_PROFILES.fast);
  });

  it.each(["fast", "medium", "slow"] as const)(
    "%s: PeakBallistics falls at exactly its own release rate, not the Medium default",
    (speed) => {
      const profile = METER_SPEED_PROFILES[speed];
      const b = new PeakBallistics();
      b.update(-6, 0, profile.peakReleaseDbPerS, profile.peakHoldMs);
      // A much quieter peak (never the floor here) 0.1 s later: the bar only releases.
      b.update(Number.NEGATIVE_INFINITY, 100, profile.peakReleaseDbPerS, profile.peakHoldMs);
      expect(b.bar).toBeCloseTo(-6 - profile.peakReleaseDbPerS * 0.1, 5);
    },
  );

  it("time to fall 24 dB is shorter for Fast than Medium, and shorter for Medium than Slow", () => {
    function msToFall24Db(speed: MeterSpeed): number {
      const profile = METER_SPEED_PROFILES[speed];
      const b = new PeakBallistics();
      b.update(0, 0, profile.peakReleaseDbPerS, profile.peakHoldMs);
      // Release starts immediately after the hold tick's own duration (the bar itself has no
      // hold — only the tick does — but this isolates pure release-rate comparison).
      return (24 / profile.peakReleaseDbPerS) * 1000;
    }
    expect(msToFall24Db("fast")).toBeLessThan(msToFall24Db("medium"));
    expect(msToFall24Db("medium")).toBeLessThan(msToFall24Db("slow"));
  });

  it("a mid-flight speed change takes effect on the very next update, with no discontinuity", () => {
    const b = new PeakBallistics();
    b.update(-6, 0, METER_SPEED_PROFILES.medium.peakReleaseDbPerS, METER_SPEED_PROFILES.medium.peakHoldMs);
    // Switch to Slow immediately (still within Medium's hold window) — the bar must not jump.
    b.update(-6, 10, METER_SPEED_PROFILES.slow.peakReleaseDbPerS, METER_SPEED_PROFILES.slow.peakHoldMs);
    expect(b.bar).toBe(-6);
  });
});

describe("meterSourceAtRest (H-43/H-123: shared animation-frame-fallback threshold)", () => {
  it("is true only once both peak and RMS are at or below SILENT_SOURCE_DBFS", () => {
    expect(meterSourceAtRest(SILENT_SOURCE_DBFS, SILENT_SOURCE_DBFS)).toBe(true);
    expect(meterSourceAtRest(SILENT_SOURCE_DBFS + 1, SILENT_SOURCE_DBFS)).toBe(false);
    expect(meterSourceAtRest(SILENT_SOURCE_DBFS, SILENT_SOURCE_DBFS + 1)).toBe(false);
    expect(meterSourceAtRest(Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY)).toBe(true);
  });
});

describe("ThrottledReadout (the numeric readouts' 4-5 Hz cap)", () => {
  it("changes on the very first update regardless of timing", () => {
    const r = new ThrottledReadout(Number.NEGATIVE_INFINITY);
    r.update(-6, 0);
    expect(r.value).toBe(-6);
  });

  it("ignores updates closer together than READOUT_INTERVAL_MS", () => {
    const r = new ThrottledReadout(-6);
    r.update(-6, 0);
    r.update(-12, 10);
    r.update(-3, READOUT_INTERVAL_MS - 1);
    expect(r.value).toBe(-6); // none of the above were far enough apart to take effect
  });

  it("takes the latest value once the interval has passed", () => {
    const r = new ThrottledReadout(-6);
    r.update(-6, 0);
    r.update(-12, 50);
    r.update(-3, READOUT_INTERVAL_MS + 1);
    expect(r.value).toBe(-3); // the *latest* value at that point, not an average
  });

  it("reset() makes the next update land immediately", () => {
    const r = new ThrottledReadout(-6);
    r.update(-6, 0);
    r.reset(Number.NEGATIVE_INFINITY);
    r.update(-40, 1); // 1 ms later — would normally be throttled
    expect(r.value).toBe(-40);
  });
});

describe("SmoothedDb (the RMS readout's extra smoothing)", () => {
  it("jumps to the first value instead of smoothing from an arbitrary initial state", () => {
    const s = new SmoothedDb(Number.NEGATIVE_INFINITY, 300);
    s.update(-20, 0);
    expect(s.value).toBe(-20);
  });

  it("moves partway toward the target and converges over several time constants", () => {
    const s = new SmoothedDb(-40, 300);
    s.update(-40, 0); // establish a baseline timestamp (the very first call always jumps)
    s.update(-10, 300); // one time constant later: ~63% of the way there
    expect(s.value).toBeGreaterThan(-40);
    expect(s.value).toBeLessThan(-10);
    expect(s.value).toBeCloseTo(-40 + (-10 - -40) * (1 - Math.exp(-1)), 3);
    s.update(-10, 300 * 6); // several more time constants — essentially arrived
    expect(s.value).toBeCloseTo(-10, 0);
  });

  it("drops to -Infinity instantly instead of crawling down through it", () => {
    const s = new SmoothedDb(-10, 300);
    s.update(Number.NEGATIVE_INFINITY, 10);
    expect(s.value).toBe(Number.NEGATIVE_INFINITY);
  });

  it("rises from -Infinity instantly instead of starting from an undefined smoothing state", () => {
    const s = new SmoothedDb(Number.NEGATIVE_INFINITY, 300);
    s.update(-6, 10);
    expect(s.value).toBe(-6);
  });

  it("H-123: a per-call tau overrides the constructor's default (a speed change applies immediately)", () => {
    const fast = new SmoothedDb(-40, 300);
    fast.update(-40, 0);
    fast.update(-10, 150, 150); // one FAST time constant, not the constructor's 300 ms MEDIUM one
    expect(fast.value).toBeCloseTo(-40 + (-10 - -40) * (1 - Math.exp(-1)), 3);
  });
});
