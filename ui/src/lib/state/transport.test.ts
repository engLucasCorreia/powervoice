import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { TransportStateDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import {
  extrapolatedPositionAt,
  initTransport,
  onTelemetry,
  resetTransportForTest,
} from "./transport.svelte";

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
});

function u64(dv: DataView, offset: number, value: number): void {
  dv.setUint32(offset, value >>> 0, true);
  dv.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

/** A minimal 72-byte `VXTM` v1 frame (`ipc/telemetry.ts`'s `decodeVxtm` contract), for tests that
 * only care about `playheadSample`/`playheadTimeNs`/`rate`. */
function buildVxtmFrame(fields: {
  playheadSample: number;
  playheadTimeNs: number;
  rate: number;
}): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56); // 'V'
  dv.setUint8(1, 0x58); // 'X'
  dv.setUint8(2, 0x54); // 'T'
  dv.setUint8(3, 0x4d); // 'M'
  dv.setUint16(4, 1, true); // version
  dv.setUint16(6, 72, true); // header_len
  u64(dv, 16, fields.playheadSample);
  u64(dv, 24, fields.playheadTimeNs);
  dv.setFloat64(32, fields.rate, true);
  return buf;
}

const TRANSPORT_GET: TransportStateDto = {
  playing: true,
  playhead_samples: 0,
  play_start_samples: 0,
  doc_len_samples: 10_000_000,
  doc_rate_hz: 48_000,
  can_play: true,
};

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
