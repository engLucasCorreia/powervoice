import { describe, expect, it } from "vitest";
import { decodeVxpk, VXPK_FLAGS } from "./vxpk";

/**
 * Builds a `VXPK` frame byte-for-byte per ADR-003 §2's layout (mirrors
 * `crates/project/src/vxpk.rs`'s `encode_vxpk`, exercised independently here so the TS decoder
 * and the Rust encoder are checked against the same written-down layout, not against each other).
 */
function buildVxpk(opts: {
  requestId: number;
  flags: number;
  audioRev: number;
  startSample: number;
  spp: number;
  sampleRateHz: number;
  payload: number[]; // f32 values, in wire order
}): ArrayBuffer {
  const buf = new ArrayBuffer(48 + opts.payload.length * 4);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56); // 'V'
  dv.setUint8(1, 0x58); // 'X'
  dv.setUint8(2, 0x50); // 'P'
  dv.setUint8(3, 0x4b); // 'K'
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  dv.setUint32(8, opts.requestId, true);
  dv.setUint32(12, opts.flags, true);
  dv.setUint32(16, opts.audioRev >>> 0, true);
  dv.setUint32(20, 0, true);
  dv.setUint32(24, opts.startSample >>> 0, true);
  dv.setUint32(28, 0, true);
  dv.setUint32(32, opts.spp, true);
  const raw = (opts.flags & VXPK_FLAGS.RAW) !== 0;
  const count = raw ? opts.payload.length : opts.payload.length / 2;
  dv.setUint32(36, count, true);
  dv.setUint32(40, opts.sampleRateHz, true);
  dv.setUint32(44, 0, true);
  opts.payload.forEach((v, i) => dv.setFloat32(48 + i * 4, v, true));
  return buf;
}

describe("decodeVxpk (ADR-003 §2)", () => {
  it("decodes a bucket (min, max) frame", () => {
    const buf = buildVxpk({
      requestId: 7,
      flags: 0,
      audioRev: 42,
      startSample: 1000,
      spp: 64,
      sampleRateHz: 48_000,
      payload: [-0.5, 0.5, -0.25, 0.75],
    });
    const frame = decodeVxpk(buf)!;
    expect(frame.requestId).toBe(7);
    expect(frame.raw).toBe(false);
    expect(frame.partial).toBe(false);
    expect(frame.audioRev).toBe(42);
    expect(frame.startSample).toBe(1000);
    expect(frame.samplesPerBucket).toBe(64);
    expect(frame.sampleRateHz).toBe(48_000);
    expect(frame.count).toBe(2);
    expect(frame.buckets).toEqual([
      [-0.5, 0.5],
      [-0.25, 0.75],
    ]);
  });

  it("decodes a RAW frame as degenerate (x, x) pairs", () => {
    const buf = buildVxpk({
      requestId: 1,
      flags: VXPK_FLAGS.RAW,
      audioRev: 0,
      startSample: 0,
      spp: 1,
      sampleRateHz: 48_000,
      payload: [0.25, -0.75],
    });
    const frame = decodeVxpk(buf)!;
    expect(frame.raw).toBe(true);
    expect(frame.count).toBe(2);
    expect(frame.buckets).toEqual([
      [0.25, 0.25],
      [-0.75, -0.75],
    ]);
  });

  it("decodes the PARTIAL flag with an empty payload", () => {
    const buf = buildVxpk({
      requestId: 0,
      flags: VXPK_FLAGS.PARTIAL,
      audioRev: 0,
      startSample: 0,
      spp: 64,
      sampleRateHz: 48_000,
      payload: [],
    });
    const frame = decodeVxpk(buf)!;
    expect(frame.partial).toBe(true);
    expect(frame.buckets).toEqual([]);
  });

  it("rejects a buffer that is too short or has the wrong magic/version", () => {
    expect(decodeVxpk(new ArrayBuffer(10))).toBeNull();
    const bad = buildVxpk({
      requestId: 0,
      flags: 0,
      audioRev: 0,
      startSample: 0,
      spp: 64,
      sampleRateHz: 48_000,
      payload: [],
    });
    new DataView(bad).setUint16(4, 2, true); // wrong version
    expect(decodeVxpk(bad)).toBeNull();
  });
});
