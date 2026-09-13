import { describe, expect, it } from "vitest";
import { decodeVxmt } from "./moduleTelemetry";
import { VXMT_FIXTURE_FIELDS, VXMT_FIXTURE_HEX } from "./vxmt_fixture";

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

describe("VXMT decoder (SPEC-016 §4.12 layout contract)", () => {
  it("decodes the Rust-encoded golden frame", () => {
    expect(decodeVxmt(hexToBuffer(VXMT_FIXTURE_HEX))).toEqual(VXMT_FIXTURE_FIELDS);
  });

  it("rejects another magic, another version and truncated frames", () => {
    const good = hexToBytes(VXMT_FIXTURE_HEX);
    const badMagic = good.slice();
    badMagic[3] = "X".charCodeAt(0);
    expect(decodeVxmt(badMagic.buffer as ArrayBuffer)).toBeNull();
    const badVersion = good.slice();
    badVersion[4] = 2;
    expect(decodeVxmt(badVersion.buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxmt(good.slice(0, 20).buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxmt(good.slice(0, good.length - 2).buffer as ArrayBuffer)).toBeNull();
  });

  it("accepts a larger header_len (fields appended without a version bump)", () => {
    const good = hexToBytes(VXMT_FIXTURE_HEX);
    const longer = new Uint8Array(good.length + 4);
    longer.set(good.subarray(0, 32), 0);
    longer.set(good.subarray(32), 36);
    new DataView(longer.buffer).setUint16(6, 36, true);
    expect(decodeVxmt(longer.buffer as ArrayBuffer)).toEqual(VXMT_FIXTURE_FIELDS);
  });
});
