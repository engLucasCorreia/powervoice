import { describe, expect, it } from "vitest";
import { VXTM_FLAGS, decodeVxtm, toArrayBuffer } from "./telemetry";
import { VXTM_FIXTURE_FIELDS, VXTM_FIXTURE_HEX } from "./vxtm_fixture";

function hexToBuffer(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes.buffer;
}

describe("VXTM decoder (ADR-003 layout contract)", () => {
  it("decodes the Rust-generated fixture field for field", () => {
    const frame = decodeVxtm(hexToBuffer(VXTM_FIXTURE_HEX));
    expect(frame).toEqual({ ...VXTM_FIXTURE_FIELDS });
    expect((frame?.flags ?? 0) & VXTM_FLAGS.PLAYING).toBe(VXTM_FLAGS.PLAYING);
    expect((frame?.flags ?? 0) & VXTM_FLAGS.XRUN).toBe(VXTM_FLAGS.XRUN);
  });

  it("accepts typed-array payloads and rejects other frames", () => {
    const buf = hexToBuffer(VXTM_FIXTURE_HEX);
    const fromView = toArrayBuffer(new Uint8Array(buf));
    expect(fromView && decodeVxtm(fromView)).toEqual(decodeVxtm(buf));
    expect(decodeVxtm(buf.slice(0, 40))).toBeNull();
    const wrongMagic = new Uint8Array(buf.slice(0));
    wrongMagic[0] = 0x51;
    expect(decodeVxtm(wrongMagic.buffer)).toBeNull();
  });
});
