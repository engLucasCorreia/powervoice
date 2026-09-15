import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { TransportStateDto } from "../ipc/bindings";
import { clearActionHandlers, dispatchAction } from "../shortcuts";
import { transportStateDto } from "../test/fixtures";
import { resetSelectionForTest, setSelectionFromResult } from "./selection.svelte";
import {
  extrapolatedPositionAt,
  initTransport,
  onTelemetry,
  resetTransportForTest,
  syncSelection,
  toggleLoop,
  transportState,
} from "./transport.svelte";

/** H-37 (SPEC-003 §2.1): the Loop toggle, the selection sync and the looped extrapolation. */

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetSelectionForTest();
});

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}

function u64(dv: DataView, offset: number, value: number): void {
  dv.setUint32(offset, value >>> 0, true);
  dv.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

function vxtm(playheadSample: number, playheadTimeNs: number, rate: number): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const dv = new DataView(buf);
  [0x56, 0x58, 0x54, 0x4d].forEach((b, i) => dv.setUint8(i, b));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 72, true);
  u64(dv, 16, playheadSample);
  u64(dv, 24, playheadTimeNs);
  dv.setFloat64(32, rate, true);
  return buf;
}

describe("the Loop toggle", () => {
  it("sends the opposite of the engine's loop state (button, Ctrl/⌘+L action)", async () => {
    let loopEnabled = false;
    const sent: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return transportStateDto({ can_play: true });
      if (cmd === "transport_set_loop") {
        const enabled = (args as { enabled: boolean }).enabled;
        sent.push(enabled);
        loopEnabled = enabled;
        return transportStateDto({ can_play: true, loop_enabled: loopEnabled });
      }
      return null;
    });
    const stop = await initTransport();
    await toggleLoop();
    expect(transportState().state.loop_enabled).toBe(true);
    dispatchAction("transport.toggle_loop");
    await settle();
    expect(sent).toEqual([true, false]);
    expect(transportState().state.loop_enabled).toBe(false);
    stop();
  });
});

describe("the selection sync", () => {
  it("is latest-wins: a burst while one call is in flight sends only the newest range", async () => {
    const sent: unknown[] = [];
    const pending: Array<() => void> = [];
    mockIPC((cmd, args) => {
      if (cmd === "transport_set_selection") {
        sent.push((args as { selection: unknown }).selection);
        return new Promise<TransportStateDto>((resolve) => pending.push(() => resolve(transportStateDto())));
      }
      return null;
    });
    const done = syncSelection({ startSample: 10, endSample: 20 });
    void syncSelection({ startSample: 10, endSample: 30 });
    void syncSelection({ startSample: 10.4, endSample: 40.2 });
    await settle();
    expect(sent).toEqual([[10, 20]]);
    pending.shift()?.();
    await settle();
    expect(sent).toEqual([[10, 20], [10, 40]]);
    pending.shift()?.();
    await done;
    await settle();
    // Same range again, or an empty one after a clear: only real changes go out.
    await syncSelection({ startSample: 10, endSample: 40 });
    expect(sent).toHaveLength(2);
    const cleared = syncSelection({ startSample: 5, endSample: 5 });
    await settle();
    pending.shift()?.();
    await cleared;
    expect(sent).toEqual([[10, 20], [10, 40], null]);
  });

  it("follows the selection store once the transport is initialised", async () => {
    const sent: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return transportStateDto({ can_play: true });
      if (cmd === "transport_set_selection") {
        sent.push((args as { selection: unknown }).selection);
        return transportStateDto({ can_play: true });
      }
      return null;
    });
    const stop = await initTransport();
    await settle();
    expect(sent).toEqual([]); // no selection yet: nothing to send
    setSelectionFromResult([1_000, 5_000]);
    await settle();
    expect(sent).toEqual([[1_000, 5_000]]);
    stop();
  });
});

describe("the looped extrapolation (SPEC-003 §2.2)", () => {
  it("wraps the displayed position inside the engine's loop_range", async () => {
    mockIPC((cmd) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get")
        return transportStateDto({
          playing: true,
          can_play: true,
          doc_len_samples: 480_000,
          loop_enabled: true,
          loop_range: [48_000, 96_000],
        });
      return null;
    });
    const stop = await initTransport();
    const anchorMs = 1_000_000;
    onTelemetry(vxtm(95_000, anchorMs * 1e6, 48_000));
    // 100 ms later: 95_000 + 4_800 = 99_800 → 48_000 + 3_800.
    expect(extrapolatedPositionAt(anchorMs + 100)).toBeCloseTo(51_800, 0);
    stop();
  });
});
