import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { initRecord, onInputTelemetry, resetRecordForTest } from "../state/record.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import { docDto, recordStateDto } from "../test/fixtures";
import WaveformView from "./WaveformView.svelte";

// H-122: the owner's real app (WebKitGTK, real Rust backend) never drew the live take — only the
// record head. The broken hop was the UI's `record_peaks_get` poll effect: `poll()` read
// `rec.elapsedSamples` (and `liveSpb`) *synchronously* inside the `$effect` before its first
// `await`, so Svelte tracked them as the effect's dependencies. Every 60 Hz telemetry frame moves
// `elapsedSamples`, so the effect re-ran on every frame: its cleanup set `disposed = true` for the
// request in flight and cleared the interval (which therefore never fired). A response only
// landed if it arrived before the *next* telemetry frame (≤ 16.7 ms, ~8 ms on average) — the
// preview harness and the vitest mocks answer within a microtask, so it always landed there; the
// real IPC round trip (WebKitGTK custom-protocol invoke → `spawn_blocking` → the engine's control
// thread → a VXPK body) usually doesn't, so `liveBuckets` stayed empty and nothing was drawn.
//
// These tests give `record_peaks_get` a realistic round trip (longer than a telemetry frame) and
// check what is actually drawn: a fake 2D context counts the take's `fillRect` columns.

const RATE_HZ = 48_000;
const TELEMETRY_MS = 1000 / 60;
/** Slower than one telemetry frame — the real app's IPC round trip under WebKitGTK. */
const ROUND_TRIP_MS = 30;

function frame(flags: number, playheadSample: number): TelemetryFrame {
  return {
    seq: 0,
    flags,
    playheadSample,
    playheadTimeNs: 0,
    rate: 0,
    outPeakDbfs: Number.NEGATIVE_INFINITY,
    outRmsDbfs: Number.NEGATIVE_INFINITY,
    inPeakDbfs: Number.NEGATIVE_INFINITY,
    inRmsDbfs: Number.NEGATIVE_INFINITY,
    audioRev: 0,
    droppedRtEvents: 0,
  };
}

/** A `PARTIAL` `VXPK` frame of `count` (-0.5, 0.5) buckets from bucket 0. */
function liveVxpk(spb: number, count: number): ArrayBuffer {
  const headerLen = 48;
  const buf = new ArrayBuffer(headerLen + count * 8);
  const dv = new DataView(buf);
  [0x56, 0x58, 0x50, 0x4b].forEach((b, i) => dv.setUint8(i, b));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, headerLen, true);
  dv.setUint32(12, 1 << 1, true);
  dv.setUint32(32, spb, true);
  dv.setUint32(36, count, true);
  dv.setUint32(40, RATE_HZ, true);
  for (let i = 0; i < count; i++) {
    dv.setFloat32(headerLen + i * 8, -0.5, true);
    dv.setFloat32(headerLen + i * 8 + 4, 0.5, true);
  }
  return buf;
}

/** Counts `fillRect` calls whose height is less than the full canvas (the take's columns, not the
 * background). Every other method is a no-op. */
function fakeCtx(heightPx: number): { ctx: CanvasRenderingContext2D; columns: () => number; reset: () => void } {
  let columns = 0;
  const target: Record<string, unknown> = {
    fillRect: (_x: number, _y: number, _w: number, h: number) => {
      if (h > 0 && h < heightPx) {
        columns += 1;
      }
    },
    measureText: () => ({ width: 10 }),
  };
  const ctx = new Proxy(target, {
    get(obj, prop) {
      if (prop in obj) {
        return obj[prop as string];
      }
      const noop = (): void => {};
      obj[prop as string] = noop;
      return noop;
    },
    set(obj, prop, value) {
      obj[prop as string] = value;
      return true;
    },
  }) as unknown as CanvasRenderingContext2D;
  return {
    ctx,
    columns: () => columns,
    reset: () => {
      columns = 0;
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  clearMocks();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
});

describe("H-122: the live take is drawn when record_peaks_get is slower than a telemetry frame", () => {
  it("draws the growing take and polls at LIVE_POLL_MS, not once per telemetry frame", async () => {
    const heightPx = 200;
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => heightPx });
    const fake = fakeCtx(heightPx);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(((id: string) =>
      id === "2d" ? fake.ctx : null) as unknown as HTMLCanvasElement["getContext"]);

    vi.useFakeTimers();
    let elapsed = 0;
    let requests = 0;
    let responses = 0;
    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0, sample_rate_hz: RATE_HZ });
    const recordingState = recordStateDto({ armed: true, input_open: true, recording: true, monitoring: true });
    mockIPC(
      (cmd) => {
        if (cmd === "record_get") {
          return recordingState;
        }
        if (cmd === "record_peaks_get") {
          requests += 1;
          // The backend snapshots the take when the request reaches it, then the body travels
          // back: the UI sees it one round trip later.
          const count = Math.floor(elapsed / 256);
          return new Promise<ArrayBuffer>((resolve) =>
            setTimeout(() => {
              responses += 1;
              resolve(liveVxpk(256, count));
            }, ROUND_TRIP_MS),
          );
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    const stopRecord = initRecord();
    await emit("document_changed", recordingDoc);
    await vi.advanceTimersByTimeAsync(0);
    flushSync();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await vi.advanceTimersByTimeAsync(0);

    try {
      // One second of a take at the engine's 60 Hz telemetry rate.
      const ticks = 60;
      for (let i = 1; i <= ticks; i++) {
        elapsed = Math.round((i * TELEMETRY_MS * RATE_HZ) / 1000);
        onInputTelemetry(frame(VXTM_FLAGS.RECORDING, elapsed));
        flushSync();
        await vi.advanceTimersByTimeAsync(TELEMETRY_MS);
      }
      // The poll runs on its own clock (LIVE_POLL_MS = 50 ms → ~20 per second), not once per
      // telemetry frame (60 per second, every one of them superseded before it landed).
      expect(requests).toBeLessThanOrEqual(25);
      expect(requests).toBeGreaterThanOrEqual(15);
      expect(responses).toBeGreaterThan(0);

      // What is drawn: the take's columns, not just the record head.
      fake.reset();
      onInputTelemetry(frame(VXTM_FLAGS.RECORDING, elapsed + 800));
      flushSync();
      await vi.advanceTimersByTimeAsync(TELEMETRY_MS);
      expect(fake.columns()).toBeGreaterThan(100);
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
    }
  });
});
