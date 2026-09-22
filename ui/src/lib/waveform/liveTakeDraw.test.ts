import { describe, expect, it } from "vitest";
import { extendLiveBucketsToHead } from "./liveTakeDraw";

// H-114: `record_peaks_get` polls at `LIVE_POLL_MS`, so `liveBuckets` can be stale by up to a
// whole poll interval by the time a frame draws it, while the record head moves every telemetry
// tick — `extendLiveBucketsToHead` closes that visible gap by holding the last known amplitude
// out to the head. Pure function, so these are exact rather than pixel/DOM assertions (jsdom has
// no real canvas to read back from).
describe("extendLiveBucketsToHead", () => {
  it("returns the buckets unchanged when the head hasn't moved past the real data", () => {
    const buckets: Array<[number, number]> = [
      [-0.5, 0.5],
      [-0.25, 0.25],
    ];
    // Real data covers samples [0, 512) at spb 256; a head at or before that is no gap.
    expect(extendLiveBucketsToHead(buckets, 0, 256, 512)).toBe(buckets);
    expect(extendLiveBucketsToHead(buckets, 0, 256, 300)).toBe(buckets);
  });

  it("returns the buckets unchanged when there is nothing to draw yet", () => {
    expect(extendLiveBucketsToHead([], 0, 256, 10_000)).toEqual([]);
  });

  it("holds the last bucket's amplitude out to the head, one synthetic bucket per spb of gap", () => {
    const buckets: Array<[number, number]> = [
      [-0.5, 0.5],
      [-0.1, 0.9],
    ];
    // Real data ends at 512; the head is 300 samples further (300 / 256 spb -> 2 extra buckets).
    const extended = extendLiveBucketsToHead(buckets, 0, 256, 512 + 300);
    expect(extended).toHaveLength(4);
    expect(extended[0]).toEqual([-0.5, 0.5]);
    expect(extended[1]).toEqual([-0.1, 0.9]);
    expect(extended[2]).toEqual([-0.1, 0.9]); // held
    expect(extended[3]).toEqual([-0.1, 0.9]); // held
  });

  it("accounts for bucketsStartSample when the request paged past bucket 0", () => {
    const buckets: Array<[number, number]> = [[-0.2, 0.2]];
    // Data covers samples [1_000, 1_100) at spb 100; a head 50 samples past that end is one gap
    // bucket (ceil(50 / 100) = 1).
    const extended = extendLiveBucketsToHead(buckets, 1_000, 100, 1_150);
    expect(extended).toEqual([
      [-0.2, 0.2],
      [-0.2, 0.2],
    ]);
  });

  it("never mutates the input array (it may be the live `$state` snapshot)", () => {
    const buckets: Array<[number, number]> = [[-1, 1]];
    const before = [...buckets];
    extendLiveBucketsToHead(buckets, 0, 256, 100_000);
    expect(buckets).toEqual(before);
  });
});
