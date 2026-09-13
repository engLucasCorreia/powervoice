import { describe, expect, it, vi } from "vitest";
import type { SpectroRequestDto } from "../ipc/bindings";
import { SpectroRequester, type SpectroViewport } from "./spectroRequester";
import { VXST_FLAGS } from "./vxst";

/** Minimal `VXST` encoder for tests (the layout contract itself is `vxst.test.ts`). */
function encodeVxst(opts: {
  requestId: number;
  audioRev: number;
  fftSize?: number;
  hop: number;
  tileIndex: number;
  frames?: number;
  flags?: number;
  fill?: number;
}): ArrayBuffer {
  const fftSize = opts.fftSize ?? 256;
  const bins = fftSize / 2 + 1;
  const frames = opts.frames ?? 2;
  const buf = new ArrayBuffer(64 + frames * bins);
  const dv = new DataView(buf);
  [0x56, 0x58, 0x53, 0x54].forEach((b, i) => dv.setUint8(i, b));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 64, true);
  dv.setUint32(8, opts.requestId, true);
  dv.setUint32(12, opts.flags ?? 0, true);
  dv.setUint32(16, opts.audioRev, true);
  dv.setUint32(24, opts.tileIndex * 256 * opts.hop, true);
  dv.setUint32(32, opts.hop, true);
  dv.setUint32(36, fftSize, true);
  dv.setUint32(40, frames, true);
  dv.setUint32(44, bins, true);
  dv.setFloat32(48, -150, true);
  dv.setFloat32(52, 6, true);
  dv.setUint32(56, opts.tileIndex, true);
  dv.setUint32(60, 0, true);
  new Uint8Array(buf, 64).fill(opts.fill ?? 1);
  return buf;
}

/** A viewport showing tile 0 only of a 1-tile document at hop 16 (N = 256). */
const ONE_TILE: SpectroViewport = {
  startSample: 0,
  endSample: 1000,
  samplesPerDevicePixel: 1,
  lenSamples: 256 * 16,
  fftSize: 256,
};

function setup(maxBytes?: number) {
  const sent: SpectroRequestDto[] = [];
  const send = vi.fn(async (req: SpectroRequestDto) => {
    sent.push(req);
  });
  const requester = new SpectroRequester(send, { maxBytes });
  return { requester, sent, send };
}

describe("SpectroRequester (SPEC-007 §2.8, §4.6, ADR-003)", () => {
  it("requests the missing tiles with the zoom's hop and applies the reply", async () => {
    const { requester, sent } = setup();
    const id = await requester.request(ONE_TILE);
    expect(id).toBe(1);
    expect(sent[0]).toEqual({
      request_id: 1,
      audio_rev: 0,
      fft_size: 256,
      hop: 16,
      window: 0,
      tiles: [0],
    });
    const tile = requester.handleMessage(
      encodeVxst({ requestId: 1, audioRev: 0, hop: 16, tileIndex: 0, flags: VXST_FLAGS.LAST }),
    );
    expect(tile?.frames).toBe(2);
    expect(requester.tile(256, 16, 0)?.data.length).toBe(2 * 129);
    // Everything held: no IPC on the next redraw.
    expect(await requester.request(ONE_TILE)).toBeNull();
    expect(sent).toHaveLength(1);
  });

  it("drops tiles with a stale audio_rev and clears held tiles on an audio edit (AC-8)", async () => {
    const { requester } = setup();
    requester.setAudioRev(5);
    expect(
      requester.handleMessage(encodeVxst({ requestId: 1, audioRev: 4, hop: 16, tileIndex: 0 })),
    ).toBeNull();
    expect(requester.tile(256, 16, 0)).toBeUndefined();
    requester.handleMessage(encodeVxst({ requestId: 1, audioRev: 5, hop: 16, tileIndex: 0 }));
    expect(requester.tile(256, 16, 0)).toBeDefined();
    requester.setAudioRev(6);
    expect(requester.tile(256, 16, 0)).toBeUndefined();
    expect(requester.bytes).toBe(0);
    // A marker-only edit keeps audio_rev, so nothing is dropped or refetched.
    requester.handleMessage(encodeVxst({ requestId: 2, audioRev: 6, hop: 16, tileIndex: 0 }));
    requester.setAudioRev(6);
    expect(requester.tile(256, 16, 0)).toBeDefined();
  });

  it("lets a refined tile replace its preview, never the other way round", () => {
    const { requester } = setup();
    const preview = encodeVxst({
      requestId: 1,
      audioRev: 0,
      hop: 512,
      tileIndex: 3,
      flags: VXST_FLAGS.PREVIEW,
      fill: 7,
    });
    const refined = encodeVxst({ requestId: 1, audioRev: 0, hop: 512, tileIndex: 3, fill: 9 });
    expect(requester.handleMessage(preview)?.preview).toBe(true);
    expect(requester.handleMessage(refined)?.preview).toBe(false);
    expect(requester.handleMessage(preview)).toBeNull();
    expect(requester.tile(256, 512, 3)?.data[0]).toBe(9);
  });

  it("doesn't re-request tiles the outstanding request already covers", async () => {
    const { requester, sent } = setup();
    await requester.request(ONE_TILE);
    // A redraw while the tile is in flight: no new request (that would cancel the work).
    expect(await requester.request({ ...ONE_TILE, endSample: 1001 })).toBeNull();
    // A preview arriving doesn't complete it either.
    requester.handleMessage(
      encodeVxst({ requestId: 1, audioRev: 0, hop: 16, tileIndex: 0, flags: VXST_FLAGS.PREVIEW }),
    );
    expect(await requester.request(ONE_TILE)).toBeNull();
    // A different hop (zoom) is a new request.
    expect(await requester.request({ ...ONE_TILE, samplesPerDevicePixel: 64 })).toBe(2);
    expect(sent.map((r) => r.hop)).toEqual([16, 64]);
  });

  it("retries after a failed IPC call", async () => {
    const send = vi.fn().mockRejectedValueOnce(new Error("ipc")).mockResolvedValue(undefined);
    const requester = new SpectroRequester(send);
    expect(await requester.request(ONE_TILE)).toBeNull();
    expect(await requester.request(ONE_TILE)).toBe(2);
  });

  it("evicts least-recently-used tiles over its byte cap", () => {
    const tileBytes = 2 * 129;
    const { requester } = setup(2 * tileBytes);
    for (const k of [0, 1]) {
      requester.handleMessage(encodeVxst({ requestId: 1, audioRev: 0, hop: 16, tileIndex: k }));
    }
    requester.tile(256, 16, 0); // touch 0 → 1 is now the LRU
    requester.handleMessage(encodeVxst({ requestId: 1, audioRev: 0, hop: 16, tileIndex: 2 }));
    expect(requester.tile(256, 16, 1)).toBeUndefined();
    expect(requester.tile(256, 16, 0)).toBeDefined();
    expect(requester.tile(256, 16, 2)).toBeDefined();
    expect(requester.bytes).toBe(2 * tileBytes);
  });
});
