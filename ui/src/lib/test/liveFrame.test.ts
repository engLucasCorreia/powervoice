import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import { decodeVxsa } from "../ipc/analyzer";
import { deliverChannelMessage } from "./liveFrame";
import { encodeVxsa } from "./vxsa";

/**
 * H-88: the helper itself. `deliverChannelMessage` is the seam that lets a component test push a
 * live IPC-channel message into a component's own subscription — see the module doc comment in
 * `./liveFrame.ts` for how (and why) it works with no production code changes.
 */

afterEach(() => {
  clearMocks();
});

describe("deliverChannelMessage", () => {
  it("invokes the channel's own onmessage with the delivered payload", () => {
    mockIPC(() => undefined);
    const received: string[] = [];
    const channel = new Channel<string>((m) => received.push(m));
    deliverChannelMessage(channel, "hello");
    expect(received).toEqual(["hello"]);
  });

  it("delivers several messages on the same channel in order", () => {
    mockIPC(() => undefined);
    const received: string[] = [];
    const channel = new Channel<string>((m) => received.push(m));
    deliverChannelMessage(channel, "a");
    deliverChannelMessage(channel, "b");
    deliverChannelMessage(channel, "c");
    expect(received).toEqual(["a", "b", "c"]);
  });

  it("keeps two channels' message ordering independent of each other", () => {
    mockIPC(() => undefined);
    const a: string[] = [];
    const b: string[] = [];
    const chA = new Channel<string>((m) => a.push(m));
    const chB = new Channel<string>((m) => b.push(m));
    deliverChannelMessage(chA, "a1");
    deliverChannelMessage(chB, "b1");
    deliverChannelMessage(chA, "a2");
    expect(a).toEqual(["a1", "a2"]);
    expect(b).toEqual(["b1"]);
  });

  it("throws instead of silently doing nothing when no mockIPC() is active", () => {
    const fakeChannel = { id: 123 } as unknown as Channel<string>;
    expect(() => deliverChannelMessage(fakeChannel, "x")).toThrow();
  });

  it("delivers a real encoded VXSA buffer that decodeVxsa accepts (end-to-end with ./vxsa)", () => {
    mockIPC(() => undefined);
    const frames: ArrayBuffer[] = [];
    const channel = new Channel<ArrayBuffer>((m) => frames.push(m));
    deliverChannelMessage(channel, encodeVxsa({ levelsDb: [-10, -20, -30], reset: true }));
    expect(frames).toHaveLength(1);
    const decoded = decodeVxsa(frames[0]!);
    expect(decoded).not.toBeNull();
    expect(decoded!.levelsDb).toEqual([-10, -20, -30]);
    expect(decoded!.reset).toBe(true);
  });
});
