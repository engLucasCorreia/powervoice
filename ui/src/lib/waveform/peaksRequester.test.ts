import { describe, expect, it, vi } from "vitest";
import type { PeaksRequestDto } from "../ipc/bindings";
import { PeaksRequester } from "./peaksRequester";
import { VXPK_FLAGS } from "./vxpk";

/** Minimal `VXPK` encoder for tests (see `vxpk.test.ts` for the full layout walkthrough). */
function encodeVxpk(opts: {
  requestId: number;
  audioRev: number;
  spp: number;
  startSample?: number;
  buckets: Array<[number, number]>;
}): ArrayBuffer {
  const buf = new ArrayBuffer(48 + opts.buckets.length * 8);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x50);
  dv.setUint8(3, 0x4b);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  dv.setUint32(8, opts.requestId, true);
  dv.setUint32(12, 0, true);
  dv.setUint32(16, opts.audioRev, true);
  dv.setUint32(24, opts.startSample ?? 0, true);
  dv.setUint32(32, opts.spp, true);
  dv.setUint32(36, opts.buckets.length, true);
  dv.setUint32(40, 48_000, true);
  dv.setUint32(44, 0, true);
  opts.buckets.forEach(([mn, mx], i) => {
    dv.setFloat32(48 + i * 8, mn, true);
    dv.setFloat32(48 + i * 8 + 4, mx, true);
  });
  return buf;
}

describe("PeaksRequester (SPEC-006 §2.3, §4.3, ADR-003 §2)", () => {
  it("applies a matching response", async () => {
    const fetchPeaks = vi.fn(async (req: PeaksRequestDto) =>
      encodeVxpk({ requestId: req.request_id, audioRev: 0, spp: 64, buckets: [[-0.5, 0.5]] }),
    );
    const requester = new PeaksRequester(fetchPeaks);
    await requester.request(0, 1, 100);
    expect(requester.state?.buckets).toEqual([[-0.5, 0.5]]);
    expect(requester.state?.level).toBe(64);
    expect(fetchPeaks).toHaveBeenCalledWith(
      expect.objectContaining({ spp: 64, start_sample: 0, count: 1 }),
    );
  });

  it("drops a response whose audio_rev doesn't match the current document (ADR-003)", async () => {
    const fetchPeaks = vi.fn(async (req: PeaksRequestDto) =>
      encodeVxpk({ requestId: req.request_id, audioRev: 999, spp: 64, buckets: [[-1, 1]] }),
    );
    const requester = new PeaksRequester(fetchPeaks);
    requester.setAudioRev(1); // the document's real current audio_rev
    await requester.request(0, 1, 100);
    expect(requester.state).toBeNull();
  });

  it("drops a stale (superseded) response even when audio_rev matches (SPEC-006 §2.3)", async () => {
    // The response for the later request_id arrives first...
    let responses: ArrayBuffer[] = [];
    const fetchPeaks = vi.fn(async () => responses.shift()!);
    const requester = new PeaksRequester(fetchPeaks);

    responses = [encodeVxpk({ requestId: 2, audioRev: 0, spp: 64, buckets: [[0, 1]] })];
    await requester.request(0, 1, 100); // request_id 1, but the fake resolves request_id 2's bytes
    expect(requester.state?.buckets).toEqual([[0, 1]]);

    // ...then request_id 1's (older, cheaper) reply arrives: it must not overwrite request_id 2's.
    responses = [encodeVxpk({ requestId: 1, audioRev: 0, spp: 64, buckets: [[-9, 9]] })];
    await requester.request(0, 1, 100); // issues request_id 2 this time, but resolves id 1's bytes
    // The newer reply's buckets must stick.
    expect(requester.state?.buckets).toEqual([[0, 1]]);
  });

  it("clearing the cached state on a document change (setAudioRev) forces a fresh look", () => {
    const requester = new PeaksRequester(async () => new ArrayBuffer(0));
    requester.setAudioRev(1);
    expect(requester.state).toBeNull();
  });

  it("ignores an undecodable response instead of throwing", async () => {
    const requester = new PeaksRequester(async () => new ArrayBuffer(4));
    await expect(requester.request(0, 1, 100)).resolves.toBeUndefined();
    expect(requester.state).toBeNull();
  });

  it("swallows a rejected fetch instead of throwing", async () => {
    const requester = new PeaksRequester(async () => {
      throw new Error("network gone");
    });
    await expect(requester.request(0, 1, 100)).resolves.toBeUndefined();
    expect(requester.state).toBeNull();
  });
});
