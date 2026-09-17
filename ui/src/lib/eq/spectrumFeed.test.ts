import { describe, expect, it } from "vitest";
import { VXSA_FIXTURE_FIELDS, VXSA_FIXTURE_HEX } from "../ipc/vxsa_fixture";
import { encodeVxsa } from "../test/vxsa";
import { createEqSpectrumFeed } from "./spectrumFeed";

function hexToBuffer(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return bytes.buffer as ArrayBuffer;
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
