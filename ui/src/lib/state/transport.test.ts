import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TransportStateDto } from "../ipc/bindings";
import { VXTM_FLAGS } from "../ipc/telemetry";
import { clearActionHandlers } from "../shortcuts";
import { transportStateDto } from "../test/fixtures";
import {
  clearOutputClip,
  extrapolatedPositionAt,
  initTransport,
  onTelemetry,
  resetTransportForTest,
  transportState,
} from "./transport.svelte";
import { frameScheduler } from "../render/frameScheduler";

/**
 * S2-03: `extrapolatedPositionAt` is SPEC-009 §4.3's "position at the key press" — the same
 * `PlayheadExtrapolator`/`ClockSync` math `transport/playhead.test.ts` already covers, wired to a
 * `KeyboardEvent.timeStamp`. These tests exercise the wiring (ms → ns, the clock offset, the
 * no-anchor fallback), not the extrapolation arithmetic itself.
 */

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  frameScheduler.resetForTest();
});

function u64(dv: DataView, offset: number, value: number): void {
  dv.setUint32(offset, value >>> 0, true);
  dv.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

/** A minimal 72-byte `VXTM` v1 frame (`ipc/telemetry.ts`'s `decodeVxtm` contract). `flags` and the
 * meter fields default to 0/`-Infinity` for tests that only care about the playhead. */
function buildVxtmFrame(fields: {
  playheadSample: number;
  playheadTimeNs: number;
  rate: number;
  flags?: number;
  outPeakDbfs?: number;
  outRmsDbfs?: number;
}): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56); // 'V'
  dv.setUint8(1, 0x58); // 'X'
  dv.setUint8(2, 0x54); // 'T'
  dv.setUint8(3, 0x4d); // 'M'
  dv.setUint16(4, 1, true); // version
  dv.setUint16(6, 72, true); // header_len
  dv.setUint32(12, fields.flags ?? 0, true); // flags
  u64(dv, 16, fields.playheadSample);
  u64(dv, 24, fields.playheadTimeNs);
  dv.setFloat64(32, fields.rate, true);
  dv.setFloat32(40, fields.outPeakDbfs ?? Number.NEGATIVE_INFINITY, true); // out_peak_dbfs
  dv.setFloat32(44, fields.outRmsDbfs ?? Number.NEGATIVE_INFINITY, true); // out_rms_dbfs
  return buf;
}

const TRANSPORT_GET = transportStateDto({ playing: true, doc_len_samples: 10_000_000, can_play: true });

describe("extrapolatedPositionAt (S2-03, SPEC-009 §4.3)", () => {
  it("falls back to the last known playhead sample before any telemetry anchor arrives", () => {
    expect(extrapolatedPositionAt(performance.now())).toBe(0);
  });

  it("extrapolates from the last telemetry anchor at the given event timestamp", async () => {
    mockIPC((cmd) => {
      // A server clock that agrees with the UI's own `performance.now()` keeps `ClockSync`'s
      // offset near zero, so the test's timestamps don't need to account for it.
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return TRANSPORT_GET;
      return null;
    });
    const stop = await initTransport();

    const anchorMs = 1_000_000;
    onTelemetry(
      buildVxtmFrame({
        playheadSample: 48_000,
        playheadTimeNs: anchorMs * 1e6,
        rate: 48_000,
      }),
    );

    // 10 ms later at 48 kHz: +480 samples. The clock offset is 0 (no sync ran), so the event
    // timestamp maps directly onto the anchor's app-clock ns.
    expect(extrapolatedPositionAt(anchorMs + 10)).toBeCloseTo(48_480, 0);
    // Exactly at the anchor: no extrapolation yet.
    expect(extrapolatedPositionAt(anchorMs)).toBeCloseTo(48_000, 0);

    stop();
  });
});

// H-28 item 2: `initTransport` subscribes to telemetry before awaiting `transportGet` (matching
// the real engine, which can start streaming telemetry before the initial state round trip
// lands). A telemetry frame arriving in that gap must not corrupt the initial state application.
describe("a telemetry frame arriving before transport_get resolves (H-28 item 2)", () => {
  it("is ignored, so the initial transport_get snapshot's own playhead_samples still applies", async () => {
    let resolveGet: (value: TransportStateDto) => void = () => {};
    const getPromise = new Promise<TransportStateDto>((resolve) => {
      resolveGet = resolve;
    });
    mockIPC((cmd) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return getPromise;
      return null;
    });

    const initPromise = initTransport();
    // A telemetry frame beats transport_get's round trip (a race the real engine can hit too).
    onTelemetry(
      buildVxtmFrame({ playheadSample: 999, playheadTimeNs: performance.now() * 1e6, rate: 48_000 }),
    );

    resolveGet(transportStateDto({ playhead_samples: 555_555, doc_len_samples: 480_000, can_play: true }));
    const stop = await initPromise;

    // Without the fix, the pre-ready frame would already have set an extrapolator anchor, so
    // `applyState`'s `!extrapolator.hasAnchor` guard would skip applying the snapshot's own
    // `playhead_samples`, leaving the readout stuck at 0.
    expect(transportState().playheadSamples).toBe(555_555);

    stop();
  });
});

// H-41: the output meter applies the same ballistics the input meter does (instant attack, 20
// dB/s release, a 1.5 s hold) on top of the engine's now-properly-windowed out_peak_dbfs/
// out_rms_dbfs, throttles the numeric readouts to ~4-5 Hz, and latches OUT_CLIP client-side.
describe("the output meter (H-41)", () => {
  async function setUp(): Promise<() => void> {
    mockIPC((cmd) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return TRANSPORT_GET;
      return null;
    });
    return initTransport();
  }

  it("applies instant-attack / smooth-release ballistics to the bar, and holds the peak", async () => {
    const stop = await setUp();
    const now = vi.spyOn(performance, "now");
    try {
      now.mockReturnValue(0);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -6, outRmsDbfs: -9 }));
      expect(transportState().meter.peakDbfs).toBe(-6); // instant attack
      expect(transportState().meter.holdDbfs).toBe(-6);
      expect(transportState().meter.rmsDbfs).toBe(-9); // the bar tracks the engine's own RMS directly

      now.mockReturnValue(500); // 0.5 s later, a much quieter frame — the bar only releases
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -60, outRmsDbfs: -60 }));
      expect(transportState().meter.peakDbfs).toBeCloseTo(-16, 5); // -6 - 20 dB/s * 0.5 s
      expect(transportState().meter.holdDbfs).toBe(-6); // still held (< 1.5 s since the peak)
    } finally {
      now.mockRestore();
      stop();
    }
  });

  it("latches the clip indicator on OUT_CLIP until clearOutputClip()", async () => {
    const stop = await setUp();
    try {
      onTelemetry(
        buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, flags: VXTM_FLAGS.OUT_CLIP, outPeakDbfs: 0 }),
      );
      expect(transportState().meter.clip).toBe(true);

      // A later frame with no clip flag must not clear it — it latches until clicked.
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -40 }));
      expect(transportState().meter.clip).toBe(true);

      clearOutputClip();
      expect(transportState().meter.clip).toBe(false);
    } finally {
      stop();
    }
  });

  it("throttles the numeric readouts to ~4-5 Hz instead of every telemetry frame", async () => {
    const stop = await setUp();
    const now = vi.spyOn(performance, "now");
    try {
      now.mockReturnValue(0);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -6, outRmsDbfs: -9 }));
      expect(transportState().meter.peakReadoutDbfs).toBe(-6);

      now.mockReturnValue(50); // well under the throttle interval
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -3, outRmsDbfs: -3 }));
      // The bar itself moves every frame (lively motion)...
      expect(transportState().meter.peakDbfs).toBe(-3);
      // ...but the readout hasn't been redrawn yet.
      expect(transportState().meter.peakReadoutDbfs).toBe(-6);

      now.mockReturnValue(400); // past the throttle interval
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -3, outRmsDbfs: -3 }));
      expect(transportState().meter.peakReadoutDbfs).toBe(-3);
    } finally {
      now.mockRestore();
      stop();
    }
  });

  it("stops writing new meter state once it has settled back at silence (idle-CPU guard, H-41 owner note)", async () => {
    const stop = await setUp();
    const now = vi.spyOn(performance, "now");
    try {
      now.mockReturnValue(0);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000, outPeakDbfs: -6, outRmsDbfs: -9 }));

      // Enough silent, real time for the bar and hold to fully release to -Infinity.
      now.mockReturnValue(20_000);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000 }));
      // ... and past the readout throttle interval, so the readouts have caught up too.
      now.mockReturnValue(20_300);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000 }));
      const settled = transportState().meter;
      expect(settled.peakDbfs).toBe(Number.NEGATIVE_INFINITY);
      expect(settled.holdDbfs).toBe(Number.NEGATIVE_INFINITY);

      // A further identical (silent) telemetry frame must not produce a new meter object — a
      // component reading it reactively should not re-render for a bar that isn't moving.
      now.mockReturnValue(20_600);
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: 0, rate: 48_000 }));
      expect(transportState().meter).toBe(settled);
    } finally {
      now.mockRestore();
      stop();
    }
  });
});

describe("animation frames only while something moves (H-43)", () => {
  const FAKE = [
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "performance",
  ] as const;

  async function setUpWith(playing: boolean): Promise<() => void> {
    mockIPC((cmd) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") {
        return transportStateDto({ playing, doc_len_samples: 10_000_000, doc_rate_hz: 48_000, can_play: true });
      }
      return null;
    });
    return initTransport();
  }

  /** Animation frames the shared scheduler ran during `ms` of fake time. */
  function framesDuring(ms: number): number {
    const before = frameScheduler.stats.frames;
    vi.advanceTimersByTime(ms);
    return frameScheduler.stats.frames - before;
  }

  beforeEach(() => {
    vi.useFakeTimers({ toFake: [...FAKE] });
    // A frame an earlier (real-timer) test requested must not look "already scheduled" here.
    frameScheduler.resetForTest();
  });

  afterEach(() => {
    frameScheduler.resetForTest();
    vi.useRealTimers();
  });

  it("an idle second schedules no frame; after Stop the meter falls to the floor on its own, then frames stop", async () => {
    const stop = await setUpWith(false);
    try {
      expect(framesDuring(1000)).toBe(0);

      // The engine's frames around a Stop: the last loud block, then its final silent frame —
      // after which an idle engine sends nothing more (crates/engine/tests/idle_telemetry.rs).
      onTelemetry(buildVxtmFrame({ playheadSample: 480, playheadTimeNs: 0, rate: 0, outPeakDbfs: -6, outRmsDbfs: -9 }));
      vi.advanceTimersByTime(17);
      onTelemetry(buildVxtmFrame({ playheadSample: 480, playheadTimeNs: 0, rate: 0 }));
      expect(transportState().meter.peakDbfs).toBeGreaterThan(-60); // still falling
      expect(transportState().playheadSamples).toBe(480);

      // No telemetry: the bar and the (held, then released) hold tick keep falling on animation
      // frames, reach the floor and snap to silence.
      expect(framesDuring(6000)).toBeGreaterThan(100);
      const settled = transportState().meter;
      expect(settled.peakDbfs).toBe(Number.NEGATIVE_INFINITY);
      expect(settled.holdDbfs).toBe(Number.NEGATIVE_INFINITY);
      expect(settled.rmsDbfs).toBe(Number.NEGATIVE_INFINITY);
      expect(settled.peakReadoutDbfs).toBe(Number.NEGATIVE_INFINITY);
      expect(settled.rmsReadoutDbfs).toBe(Number.NEGATIVE_INFINITY);

      // Then an idle second costs nothing: no frame, no reactive meter write.
      expect(framesDuring(1000)).toBe(0);
      expect(transportState().meter).toBe(settled);
    } finally {
      stop();
    }
  });

  it.each([
    ["playback", VXTM_FLAGS.PLAYING],
    ["recording", VXTM_FLAGS.RECORDING],
  ])("%s animates the playhead every frame, and frames stop after the transport stops", async (_, flag) => {
    const stop = await setUpWith(true);
    try {
      let sample = 0;
      const before = frameScheduler.stats.frames;
      // One second of engine telemetry at 60 Hz while moving.
      for (let i = 0; i < 60; i++) {
        onTelemetry(
          buildVxtmFrame({
            playheadSample: sample,
            playheadTimeNs: performance.now() * 1e6,
            rate: 48_000,
            flags: flag,
            outPeakDbfs: -20,
            outRmsDbfs: -24,
          }),
        );
        vi.advanceTimersByTime(1000 / 60);
        sample += 800;
      }
      expect(frameScheduler.stats.frames - before).toBeGreaterThanOrEqual(55);
      expect(transportState().playheadSamples).toBeGreaterThan(40_000);

      // Stop: the engine's final frame (rate 0), then silence.
      onTelemetry(buildVxtmFrame({ playheadSample: sample, playheadTimeNs: performance.now() * 1e6, rate: 0 }));
      vi.advanceTimersByTime(6000);
      expect(transportState().playheadSamples).toBe(sample);
      expect(framesDuring(1000)).toBe(0);
    } finally {
      stop();
    }
  });

  it("a moving playhead whose telemetry stalls stops animating instead of running forever", async () => {
    const stop = await setUpWith(true);
    try {
      onTelemetry(buildVxtmFrame({ playheadSample: 0, playheadTimeNs: performance.now() * 1e6, rate: 48_000, flags: VXTM_FLAGS.PLAYING }));
      expect(framesDuring(500)).toBeGreaterThan(20);
      vi.advanceTimersByTime(2000);
      expect(framesDuring(1000)).toBe(0);
    } finally {
      stop();
    }
  });
});
