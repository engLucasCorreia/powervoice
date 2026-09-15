import { describe, expect, it } from "vitest";
import { binFrequencies, decodeVxis, decodeVxlt } from "./inspector";
import { VXIS_FIXTURE_FIELDS, VXIS_FIXTURE_HEX } from "./vxis_fixture";
import { VXLT_FIXTURE_HEX } from "./vxlt_fixture";

function fromHex(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes.buffer;
}

describe("VXIS / VXLT decoders (H-42, SPEC-007 §8.9)", () => {
  it("decodes the Rust-encoded VXIS fixture", () => {
    const f = decodeVxis(fromHex(VXIS_FIXTURE_HEX))!;
    expect(f).not.toBeNull();
    const { levelsDb, ...rest } = VXIS_FIXTURE_FIELDS;
    expect({
      seq: f.seq,
      reset: f.reset,
      silent: f.silent,
      sampleRateHz: f.sampleRateHz,
      fftSize: f.fftSize,
      window: f.window,
      response: f.response,
    }).toEqual(rest);
    expect(Array.from(f.levelsDb)).toEqual([...levelsDb]);
  });

  it("decodes the Rust-encoded VXLT fixture, room tone included", () => {
    const c = decodeVxlt(fromHex(VXLT_FIXTURE_HEX))!;
    expect(c.jobId).toBe(5);
    expect(c.index).toBe(1);
    expect(c.sampleRateHz).toBe(44_100);
    expect(c.fftSize).toBe(4);
    expect(c.window).toBe("hann");
    expect(c.levelsDb[0]).toBe(0);
    expect(c.levelsDb[1]).toBeCloseTo(-6.0206, 3);
    expect(c.levelsDb[2]).toBe(-Infinity);
    expect(c.noiseDb).not.toBeNull();
    expect(c.noiseDb![0]).toBeCloseTo(-60, 3);
    expect(c.noiseDb![1]).toBeCloseTo(-80, 3);
  });

  it("rejects truncated or foreign frames", () => {
    const vxis = fromHex(VXIS_FIXTURE_HEX);
    expect(decodeVxis(vxis.slice(0, vxis.byteLength - 2))).toBeNull();
    expect(decodeVxlt(vxis)).toBeNull();
    const vxlt = fromHex(VXLT_FIXTURE_HEX);
    expect(decodeVxlt(vxlt.slice(0, 40))).toBeNull();
    expect(decodeVxis(new ArrayBuffer(8))).toBeNull();
  });

  it("derives bin frequencies", () => {
    expect(Array.from(binFrequencies(3, 48_000, 4))).toEqual([0, 12_000, 24_000]);
  });
});
