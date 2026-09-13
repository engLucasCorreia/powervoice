import { describe, expect, it } from "vitest";
import { VXST_FIXTURES } from "../ipc/vxst_fixture";
import { Q_CEIL_DB, Q_FLOOR_DB, VXST_FLAGS, decodeVxst, dequantizeDb } from "./vxst";

function hexToBuffer(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes.buffer;
}

describe("VXST decoder (ADR-003 layout contract, SPEC-007 AC-6)", () => {
  it("decodes the Rust-generated PREVIEW and LAST fixtures field for field", () => {
    expect(VXST_FIXTURES).toHaveLength(2);
    for (const { hex, fields } of VXST_FIXTURES) {
      const frame = decodeVxst(hexToBuffer(hex));
      expect(frame).not.toBeNull();
      const { data, ...header } = frame!;
      expect(header).toEqual({ ...fields });
      expect(data.length).toBe(fields.frames * fields.bins);
      data.forEach((code, i) => expect(code).toBe((i * 37 + 11) % 256));
    }
    const [preview, last] = VXST_FIXTURES;
    expect(preview.fields.flags & VXST_FLAGS.PREVIEW).toBe(VXST_FLAGS.PREVIEW);
    expect(last.fields.flags & VXST_FLAGS.LAST).toBe(VXST_FLAGS.LAST);
  });

  it("views the payload without copying", () => {
    const buf = hexToBuffer(VXST_FIXTURES[0].hex);
    const frame = decodeVxst(buf)!;
    expect(frame.data.buffer).toBe(buf);
    expect(frame.data.byteOffset).toBe(64);
  });

  it("rejects truncated, foreign and unknown-version frames; honours a longer header", () => {
    const buf = hexToBuffer(VXST_FIXTURES[1].hex);
    expect(decodeVxst(buf.slice(0, 63))).toBeNull();
    expect(decodeVxst(buf.slice(0, buf.byteLength - 1))).toBeNull();
    const wrongMagic = new Uint8Array(buf.slice(0));
    wrongMagic[0] = 0x51;
    expect(decodeVxst(wrongMagic.buffer)).toBeNull();
    const v2 = new Uint8Array(buf.slice(0));
    v2[4] = 2;
    expect(decodeVxst(v2.buffer)).toBeNull();

    const src = new Uint8Array(buf);
    const longer = new Uint8Array(src.length + 8);
    longer.set(src.subarray(0, 64), 0);
    longer.set(src.subarray(64), 72);
    new DataView(longer.buffer).setUint16(6, 72, true);
    const frame = decodeVxst(longer.buffer)!;
    expect(frame.tileIndex).toBe(4);
    expect(Array.from(frame.data)).toEqual(Array.from(src.subarray(64)));
  });

  it("dequantizes codes 0 and 255 to the quantization range", () => {
    expect(dequantizeDb(0)).toBe(Q_FLOOR_DB);
    expect(dequantizeDb(255)).toBeCloseTo(Q_CEIL_DB, 10);
    expect(dequantizeDb(1) - dequantizeDb(0)).toBeCloseTo(156 / 255, 10);
  });
});
