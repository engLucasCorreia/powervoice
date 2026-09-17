import { describe, expect, it } from "vitest";
import { decodeVxtc } from "./transferCurve";
import {
  VXTC_FALLING_FIXTURE_FIELDS,
  VXTC_FALLING_FIXTURE_HEX,
  VXTC_FIXTURE_FIELDS,
  VXTC_FIXTURE_HEX,
} from "./vxtc_fixture";

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes;
}

function hexToBuffer(hex: string): ArrayBuffer {
  return hexToBytes(hex).buffer as ArrayBuffer;
}

/** AC-23: the golden frames Rust encodes decode field by field, with and without HAS_FALLING. */
describe("VXTC decoder (SPEC-016 §4.12 layout contract)", () => {
  it("decodes the Rust-encoded golden frame without a Falling branch", () => {
    const frame = decodeVxtc(hexToBuffer(VXTC_FIXTURE_HEX));
    expect(frame).not.toBeNull();
    expect(frame!.seq).toBe(VXTC_FIXTURE_FIELDS.seq);
    expect(frame!.rising).toEqual([...VXTC_FIXTURE_FIELDS.rising]);
    expect(frame!.falling).toBeNull();
    expect(frame!.components).toEqual(VXTC_FIXTURE_FIELDS.components.map((row) => [...row]));
    expect(frame!.handles).toEqual(VXTC_FIXTURE_FIELDS.handles.map((h) => ({ ...h })));
    // The input levels are derived from the range, not transmitted.
    expect(frame!.inDbfs).toEqual([
      VXTC_FIXTURE_FIELDS.xMinDb,
      (VXTC_FIXTURE_FIELDS.xMinDb + VXTC_FIXTURE_FIELDS.xMaxDb) / 2,
      VXTC_FIXTURE_FIELDS.xMaxDb,
    ]);
  });

  it("decodes the golden frame with HAS_FALLING, a hysteresis loop and muted levels", () => {
    const frame = decodeVxtc(hexToBuffer(VXTC_FALLING_FIXTURE_HEX));
    expect(frame).not.toBeNull();
    expect(frame!.seq).toBe(VXTC_FALLING_FIXTURE_FIELDS.seq);
    expect(frame!.rising).toEqual([...VXTC_FALLING_FIXTURE_FIELDS.rising]);
    expect(frame!.falling).toEqual([...VXTC_FALLING_FIXTURE_FIELDS.falling]);
    expect(frame!.components).toEqual(
      VXTC_FALLING_FIXTURE_FIELDS.components.map((row) => [...row]),
    );
    expect(frame!.handles).toEqual(VXTC_FALLING_FIXTURE_FIELDS.handles.map((h) => ({ ...h })));
    // −∞ survives as −∞ (never NaN), which is what draws a gap instead of a spike.
    expect(frame!.rising[0]).toBe(Number.NEGATIVE_INFINITY);
    expect(frame!.rising.some(Number.isNaN)).toBe(false);
    expect(frame!.handles[1]!.enabled).toBe(false);
  });

  it("rejects another magic, another version and truncated frames", () => {
    const good = hexToBytes(VXTC_FALLING_FIXTURE_HEX);
    const badMagic = good.slice();
    badMagic[3] = "X".charCodeAt(0);
    expect(decodeVxtc(badMagic.buffer as ArrayBuffer)).toBeNull();
    const badVersion = good.slice();
    badVersion[4] = 2;
    expect(decodeVxtc(badVersion.buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxtc(good.slice(0, 20).buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxtc(good.slice(0, good.length - 2).buffer as ArrayBuffer)).toBeNull();
  });

  it("rejects an impossible point count", () => {
    const good = hexToBytes(VXTC_FIXTURE_HEX);
    const huge = good.slice();
    new DataView(huge.buffer).setUint32(24, 100_000, true);
    expect(decodeVxtc(huge.buffer as ArrayBuffer)).toBeNull();
  });

  it("accepts a larger header_len (fields appended without a version bump)", () => {
    const good = hexToBytes(VXTC_FIXTURE_HEX);
    const longer = new Uint8Array(good.length + 4);
    longer.set(good.subarray(0, 40), 0);
    longer.set(good.subarray(40), 44);
    new DataView(longer.buffer).setUint16(6, 44, true);
    expect(decodeVxtc(longer.buffer as ArrayBuffer)?.rising).toEqual([
      ...VXTC_FIXTURE_FIELDS.rising,
    ]);
  });
});
