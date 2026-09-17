import { describe, expect, it } from "vitest";
import { VXSA_FIXTURE_FIELDS, VXSA_FIXTURE_HEX } from "../ipc/vxsa_fixture";
import { createEqSpectrumFeed } from "./spectrumFeed";

function hexToBuffer(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes.buffer as ArrayBuffer;
}

const VXSA_HEADER_LEN = 48;

/** A minimal, valid `VXSA` v1 buffer (SPEC-007 §4.9 layout), for exercising the feed's
 * decode/dedup path without depending on the golden fixture's own flags (it's a `reset` frame). */
function encodeVxsa(opts: { reset?: boolean; silent?: boolean; levelsDb: number[] }): ArrayBuffer {
  const bandCount = opts.levelsDb.length;
  const buf = new ArrayBuffer(VXSA_HEADER_LEN + 4 * bandCount);
  const dv = new DataView(buf);
  dv.setUint8(0, "V".charCodeAt(0));
  dv.setUint8(1, "X".charCodeAt(0));
  dv.setUint8(2, "S".charCodeAt(0));
  dv.setUint8(3, "A".charCodeAt(0));
  dv.setUint16(4, 1, true); // version
  dv.setUint16(6, VXSA_HEADER_LEN, true);
  dv.setUint32(8, 1, true); // seq
  const flags = (opts.reset ? 1 << 0 : 0) | (opts.silent ? 1 << 2 : 0);
  dv.setUint32(12, flags, true);
  dv.setUint32(16, 0, true); // frameTimeNs low
  dv.setUint32(20, 0, true); // frameTimeNs high
  dv.setUint32(24, 48_000, true); // sampleRateHz
  dv.setUint32(28, 2_048, true); // fftSize
  dv.setFloat32(32, 20, true); // f0Hz
  dv.setUint32(36, 24, true); // bandsPerOctave
  dv.setUint32(40, bandCount, true);
  dv.setUint32(44, 1, true); // response
  for (let k = 0; k < bandCount; k++) {
    dv.setFloat32(VXSA_HEADER_LEN + 4 * k, opts.levelsDb[k]!, true);
  }
  return buf;
}

describe("createEqSpectrumFeed (H-84, AC-21)", () => {
  it("decodes a real VXSA buffer and calls onFrame once", () => {
    const frames: number[] = [];
    const feed = createEqSpectrumFeed((f) => frames.push(f.bandCount));
    feed.handleMessage(hexToBuffer(VXSA_FIXTURE_HEX));
    expect(frames).toEqual([VXSA_FIXTURE_FIELDS.bandCount]);
  });

  it("ignores a garbage/undecodable message", () => {
    const frames: unknown[] = [];
    const feed = createEqSpectrumFeed((f) => frames.push(f));
    feed.handleMessage(new ArrayBuffer(2));
    feed.handleMessage("not a buffer");
    expect(frames).toEqual([]);
  });

  it("does not call onFrame again for a repeated, unchanged frame (H-43 idle dedup)", () => {
    let calls = 0;
    const feed = createEqSpectrumFeed(() => calls++);
    const levels = [-40, -50, -60];
    feed.handleMessage(encodeVxsa({ levelsDb: levels }));
    feed.handleMessage(encodeVxsa({ levelsDb: levels })); // identical, a fresh buffer instance
    feed.handleMessage(encodeVxsa({ levelsDb: levels }));
    expect(calls).toBe(1);

    feed.handleMessage(encodeVxsa({ levelsDb: [-41, -50, -60] })); // one level changed
    expect(calls).toBe(2);
  });

  it("always forwards a reset frame, even with unchanged levels", () => {
    let calls = 0;
    const feed = createEqSpectrumFeed(() => calls++);
    const levels = [-40, -50];
    feed.handleMessage(encodeVxsa({ levelsDb: levels }));
    feed.handleMessage(encodeVxsa({ levelsDb: levels, reset: true }));
    expect(calls).toBe(2);
  });
});
