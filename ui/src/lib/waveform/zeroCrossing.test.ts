import { describe, expect, it, vi } from "vitest";
import type { PeaksRequestDto } from "../ipc/bindings";
import { VXPK_FLAGS } from "./vxpk";
import { findZeroCrossing, snapSampleToZeroCrossing, ZERO_CROSSING_WINDOW_SAMPLES } from "./zeroCrossing";

/** Builds a raw `VXPK` frame (ADR-003 §2 layout), mirroring `vxpk.test.ts`'s own helper. */
function buildRawVxpk(opts: {
  audioRev: number;
  startSample: number;
  sampleRateHz: number;
  samples: number[];
}): ArrayBuffer {
  const buf = new ArrayBuffer(48 + opts.samples.length * 4);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x50);
  dv.setUint8(3, 0x4b);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  dv.setUint32(8, 1, true);
  dv.setUint32(12, VXPK_FLAGS.RAW, true);
  dv.setUint32(16, opts.audioRev >>> 0, true);
  dv.setUint32(20, 0, true);
  dv.setUint32(24, opts.startSample >>> 0, true);
  dv.setUint32(28, 0, true);
  dv.setUint32(32, 1, true);
  dv.setUint32(36, opts.samples.length, true);
  dv.setUint32(40, opts.sampleRateHz, true);
  dv.setUint32(44, 0, true);
  opts.samples.forEach((v, i) => dv.setFloat32(48 + i * 4, v, true));
  return buf;
}

describe("findZeroCrossing (SPEC-006 §4.4, pure scan)", () => {
  it("a monotonic ramp: finds the single, unique sign change", () => {
    // Strictly decreasing: the only sign change is between index 4 (0.1) and index 5 (-0.1).
    const samples = [0.9, 0.7, 0.5, 0.3, 0.1, -0.1, -0.3, -0.5, -0.7, -0.9];
    expect(findZeroCrossing(samples, 0, 4)).toBe(4);
    // Centered a few samples away on either side, still finds the same unique crossing.
    expect(findZeroCrossing(samples, 0, 1)).toBe(4);
    expect(findZeroCrossing(samples, 0, 8)).toBe(4);
  });

  it("constant DC (no sign change anywhere): returns null within the window", () => {
    const samples = new Array(1200).fill(0.5);
    expect(findZeroCrossing(samples, 0, 600, ZERO_CROSSING_WINDOW_SAMPLES)).toBeNull();
  });

  it("negative constant DC: also no crossing", () => {
    const samples = new Array(1200).fill(-0.25);
    expect(findZeroCrossing(samples, 0, 600, ZERO_CROSSING_WINDOW_SAMPLES)).toBeNull();
  });

  it("digital silence: the center sample is itself a crossing (distance 0)", () => {
    const samples = new Array(1200).fill(0);
    expect(findZeroCrossing(samples, 0, 600, ZERO_CROSSING_WINDOW_SAMPLES)).toBe(600);
  });

  it("a lone zero sample surrounded by positive values counts as its own crossing", () => {
    const samples = [0.5, 0.5, 0, 0.5, 0.5];
    expect(findZeroCrossing(samples, 0, 2)).toBe(2);
  });

  it("noise: finds the nearest of several crossings", () => {
    // Sign changes at (absolute) index 2 (0.3 -> -0.2) and index 5 (-0.4 -> 0.6).
    const samples = [0.9, 0.5, 0.3, -0.2, -0.4, -0.4, 0.6, 0.1];
    expect(findZeroCrossing(samples, 100, 102)).toBe(102); // center already sits on it
    expect(findZeroCrossing(samples, 100, 101)).toBe(102); // nearer to the first crossing
    expect(findZeroCrossing(samples, 100, 105)).toBe(105); // nearer to the second crossing
  });

  it("respects windowStart: returned index is absolute, not window-relative", () => {
    const samples = [1, 1, 1, -1, 1];
    expect(findZeroCrossing(samples, 5_000, 5_000)).toBe(5_002);
  });

  it("ties: the earlier (lower) index wins when both sides are equidistant", () => {
    // center=5; index 3 (before, distance 2) and index 7 (after, distance 2) are both crossings.
    const samples = [1, 1, 1, 0, 1, 1, 1, 0, 1, 1];
    expect(findZeroCrossing(samples, 0, 5)).toBe(3);
  });

  it("stops at the ±maxDistance boundary", () => {
    const samples = new Array(20).fill(0.5);
    samples[15] = -0.5; // crossing between index 14 and 15, distance 5 from center=10 (via +d)... actually check both sides
    expect(findZeroCrossing(samples, 0, 10, 3)).toBeNull();
    expect(findZeroCrossing(samples, 0, 10, 5)).toBe(14);
  });
});

describe("snapSampleToZeroCrossing (SPEC-006 §4.4, async wrapper over peaks_get RAW)", () => {
  it("snaps to the nearest crossing in the fetched window", async () => {
    const samples = new Array(20).fill(0.5);
    samples[10] = 0.1;
    samples[11] = -0.1;
    const fetch = vi.fn(async (_req: PeaksRequestDto) =>
      buildRawVxpk({ audioRev: 1, startSample: 0, sampleRateHz: 48_000, samples }),
    );
    const result = await snapSampleToZeroCrossing(fetch, 1, 1000, 9, 512);
    expect(result).toBe(10);
    expect(fetch).toHaveBeenCalledOnce();
    const req = fetch.mock.calls[0]![0];
    expect(req.spp).toBe(1);
  });

  it("returns the pointer sample unchanged when there's no crossing (e.g. constant DC)", async () => {
    const samples = new Array(1200).fill(0.5);
    const fetch = vi.fn(async () =>
      buildRawVxpk({ audioRev: 1, startSample: 0, sampleRateHz: 48_000, samples }),
    );
    const result = await snapSampleToZeroCrossing(fetch, 1, 100_000, 600, 512);
    expect(result).toBe(600);
  });

  it("returns the pointer sample unchanged when the response is stale (audio_rev mismatch)", async () => {
    const samples = [1, 1, -1, 1, 1];
    const fetch = vi.fn(async () =>
      buildRawVxpk({ audioRev: 999, startSample: 0, sampleRateHz: 48_000, samples }),
    );
    const result = await snapSampleToZeroCrossing(fetch, 1, 1000, 1, 512);
    expect(result).toBe(1);
  });

  it("returns the pointer sample unchanged when the fetch fails", async () => {
    const fetch = vi.fn(async () => {
      throw new Error("ipc down");
    });
    const result = await snapSampleToZeroCrossing(fetch, 1, 1000, 42, 512);
    expect(result).toBe(42);
  });

  it("clamps the request window at the document edges (no negative start_sample)", async () => {
    const samples = [1, -1, 1];
    const fetch = vi.fn(async (_req: PeaksRequestDto) =>
      buildRawVxpk({ audioRev: 1, startSample: 0, sampleRateHz: 48_000, samples }),
    );
    await snapSampleToZeroCrossing(fetch, 1, 5, 1, 512);
    const req = fetch.mock.calls[0]![0];
    expect(req.start_sample).toBe(0);
  });

  it("an empty document is a no-op", async () => {
    const fetch = vi.fn();
    const result = await snapSampleToZeroCrossing(fetch, 1, 0, 5, 512);
    expect(result).toBe(5);
    expect(fetch).not.toHaveBeenCalled();
  });
});
