import { describe, expect, it } from "vitest";
import { bandCenterHz, decodeVxsa } from "./analyzer";
import { VXSA_FIXTURE_FIELDS, VXSA_FIXTURE_HEX } from "./vxsa_fixture";

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

describe("VXSA decoder (SPEC-007 §4.9 layout contract)", () => {
  it("decodes the Rust-encoded golden frame", () => {
    expect(decodeVxsa(hexToBuffer(VXSA_FIXTURE_HEX))).toEqual(VXSA_FIXTURE_FIELDS);
  });

  it("rejects another magic, another version and truncated frames", () => {
    const good = hexToBytes(VXSA_FIXTURE_HEX);
    const badMagic = good.slice();
    badMagic[3] = "X".charCodeAt(0);
    expect(decodeVxsa(badMagic.buffer as ArrayBuffer)).toBeNull();
    const badVersion = good.slice();
    badVersion[4] = 2;
    expect(decodeVxsa(badVersion.buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxsa(good.slice(0, 20).buffer as ArrayBuffer)).toBeNull();
    expect(decodeVxsa(good.slice(0, good.length - 2).buffer as ArrayBuffer)).toBeNull();
  });

  it("accepts a larger header_len (fields appended without a version bump)", () => {
    const good = hexToBytes(VXSA_FIXTURE_HEX);
    const longer = new Uint8Array(good.length + 8);
    longer.set(good.subarray(0, 48), 0);
    longer.set(good.subarray(48), 56);
    new DataView(longer.buffer).setUint16(6, 56, true);
    expect(decodeVxsa(longer.buffer as ArrayBuffer)).toEqual(VXSA_FIXTURE_FIELDS);
  });

  it("carries -Infinity band levels, never NaN", () => {
    const frame = decodeVxsa(hexToBuffer(VXSA_FIXTURE_HEX));
    expect(frame?.levelsDb.some((v) => v === -Infinity)).toBe(true);
    expect(frame?.levelsDb.every((v) => !Number.isNaN(v))).toBe(true);
  });
});

describe("bandCenterHz (SPEC-007 §4.8.3)", () => {
  it("matches f_k = 20 * 2^(k/24)", () => {
    expect(bandCenterHz(0)).toBeCloseTo(20, 9);
    expect(bandCenterHz(24)).toBeCloseTo(40, 9);
    expect(bandCenterHz(48)).toBeCloseTo(80, 9);
  });

  it("is monotonically increasing", () => {
    let prev = 0;
    for (let k = 0; k < 246; k++) {
      const f = bandCenterHz(k);
      expect(f).toBeGreaterThan(prev);
      prev = f;
    }
  });
});
