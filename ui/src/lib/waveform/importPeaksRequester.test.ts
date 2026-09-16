import { describe, expect, it, vi } from "vitest";
import type { PeaksRequestDto } from "../ipc/bindings";
import { ImportPeaksRequester } from "./importPeaksRequester";

/** Minimal `VXPK` encoder for tests (see `vxpk.test.ts` for the full layout walkthrough). A
 * `NaN` bucket encodes a `PARTIAL` (H-71) column. */
function encodeVxpk(opts: {
  requestId: number;
  spp: number;
  startSample?: number;
  buckets: Array<[number, number]>;
  partial?: boolean;
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
  dv.setUint32(12, opts.partial ? 0b10 : 0, true);
  dv.setUint32(16, 0, true); // audio_rev: always 0 (not a document revision)
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

describe("ImportPeaksRequester (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 7)", () => {
  it("does nothing until a job id is set", async () => {
    const fetchPeaks = vi.fn(async () => encodeVxpk({ requestId: 1, spp: 64, buckets: [] }));
    const requester = new ImportPeaksRequester(fetchPeaks);
    await requester.request(0, 1, 100);
    expect(fetchPeaks).not.toHaveBeenCalled();
    expect(requester.state).toBeNull();
  });

  it("applies a matching response for the current job", async () => {
    const fetchPeaks = vi.fn(async (jobId: number, req: PeaksRequestDto) =>
      encodeVxpk({ requestId: req.request_id, spp: 64, buckets: [[-0.5, 0.5]] }),
    );
    const requester = new ImportPeaksRequester(fetchPeaks);
    requester.setJobId(7);
    await requester.request(0, 1, 100);
    expect(requester.state?.buckets).toEqual([[-0.5, 0.5]]);
    expect(requester.state?.level).toBe(64);
    expect(requester.state?.partial).toBe(false);
    expect(fetchPeaks).toHaveBeenCalledWith(
      7,
      expect.objectContaining({ spp: 64, start_sample: 0, count: 1, audio_rev: 0 }),
    );
  });

  it("carries PARTIAL buckets (some not yet computed, ADR-003 §2) straight through", async () => {
    const fetchPeaks = vi.fn(async (_jobId: number, req: PeaksRequestDto) =>
      encodeVxpk({
        requestId: req.request_id,
        spp: 64,
        buckets: [
          [-0.5, 0.5],
          [Number.NaN, Number.NaN],
        ],
        partial: true,
      }),
    );
    const requester = new ImportPeaksRequester(fetchPeaks);
    requester.setJobId(1);
    await requester.request(0, 2, 100);
    expect(requester.state?.partial).toBe(true);
    const buckets = requester.state?.buckets ?? [];
    expect(buckets[0]).toEqual([-0.5, 0.5]);
    expect(buckets[1]?.every((v) => Number.isNaN(v))).toBe(true);
  });

  it("drops a stale (superseded) response — partial frames must apply in order", async () => {
    // The response for the later request_id arrives first...
    let responses: ArrayBuffer[] = [];
    const fetchPeaks = vi.fn(async () => responses.shift()!);
    const requester = new ImportPeaksRequester(fetchPeaks);
    requester.setJobId(1);

    responses = [encodeVxpk({ requestId: 2, spp: 64, buckets: [[0, 1]] })];
    await requester.request(0, 1, 100); // request_id 1 issued, but resolves id 2's bytes
    expect(requester.state?.buckets).toEqual([[0, 1]]);

    // ...then request_id 1's (older, cheaper) reply arrives: it must not overwrite id 2's.
    responses = [encodeVxpk({ requestId: 1, spp: 64, buckets: [[-9, 9]] })];
    await requester.request(0, 1, 100); // issues request_id 2 this time, but resolves id 1's bytes
    // The newer reply's buckets must stick.
    expect(requester.state?.buckets).toEqual([[0, 1]]);
  });

  it("clears the cached state the instant the running job id changes", async () => {
    const fetchPeaks = vi.fn(async (_jobId: number, req: PeaksRequestDto) =>
      encodeVxpk({ requestId: req.request_id, spp: 64, buckets: [[-1, 1]] }),
    );
    const requester = new ImportPeaksRequester(fetchPeaks);
    requester.setJobId(1);
    await requester.request(0, 1, 100);
    expect(requester.state).not.toBeNull();

    requester.setJobId(2); // a different (or a fresh) import job started
    expect(requester.state).toBeNull();
  });

  it("drops a response that resolves after the job it was requested for already changed", async () => {
    let resolveFetch: (buf: ArrayBuffer) => void = () => {};
    const fetchPeaks = vi.fn(
      (): Promise<ArrayBuffer> =>
        new Promise<ArrayBuffer>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    const requester = new ImportPeaksRequester(fetchPeaks);
    requester.setJobId(1);
    const pending = requester.request(0, 1, 100);
    requester.setJobId(2); // the job changed while the request was in flight
    resolveFetch(encodeVxpk({ requestId: 1, spp: 64, buckets: [[-1, 1]] }));
    await pending;
    expect(requester.state).toBeNull();
  });

  it("ignores an undecodable response instead of throwing", async () => {
    const requester = new ImportPeaksRequester(async () => new ArrayBuffer(4));
    requester.setJobId(1);
    await expect(requester.request(0, 1, 100)).resolves.toBeUndefined();
    expect(requester.state).toBeNull();
  });

  it("swallows a rejected fetch instead of throwing", async () => {
    const requester = new ImportPeaksRequester(async () => {
      throw new Error("network gone");
    });
    requester.setJobId(1);
    await expect(requester.request(0, 1, 100)).resolves.toBeUndefined();
    expect(requester.state).toBeNull();
  });
});
