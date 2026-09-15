import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { MarkerDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { clearNotices } from "../state/notices.svelte";
import { applyRecordStateForTest, resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, selectAllOf } from "../state/selection.svelte";
import { initTransport, onTelemetry, resetTransportForTest } from "../state/transport.svelte";
import { transportStateDto } from "../test/fixtures";
import { addMarker, isTakeMarker, markersState, resetMarkersForTest } from "./markers.svelte";

/**
 * H-21 (SPEC-022 §2.9, AC-10): M during a take or record operation adds a point at the heard
 * position under the key press (`at + k` in the record window) — extrapolated from the telemetry
 * anchor like SPEC-009 §4.3, but not clamped to the old document length (an Insert take runs past
 * it) — and the marker is remembered as a take marker (already in committed coordinates).
 */

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetMarkersForTest();
  resetRecordForTest();
  resetSelectionForTest();
  resetTransportForTest();
});

function u64(dv: DataView, offset: number, value: number): void {
  dv.setUint32(offset, value >>> 0, true);
  dv.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

/** A minimal 72-byte `VXTM` v1 frame carrying only the playhead anchor. */
function buildVxtmFrame(playheadSample: number, playheadTimeNs: number, rate: number): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x54);
  dv.setUint8(3, 0x4d);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 72, true);
  u64(dv, 16, playheadSample);
  u64(dv, 24, playheadTimeNs);
  dv.setFloat64(32, rate, true);
  return buf;
}

const STOPPED = transportStateDto({ doc_len_samples: 100_000, can_play: true });

describe("markers during a take or record operation (H-21, SPEC-022 AC-10)", () => {
  it("M adds a point at the heard position under the key press, past the old end, ignoring the selection", async () => {
    const calls: Array<{ posSamples: number; lenSamples: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "clock_now_ns") return performance.now() * 1e6;
      if (cmd === "transport_get") return STOPPED;
      if (cmd === "marker_add") {
        calls.push(args as { posSamples: number; lenSamples: number });
        return { id: 7, pos_samples: 120_480, len_samples: 0, name: "Marker 01" } satisfies MarkerDto;
      }
      return null;
    });
    const stop = await initTransport();
    applyRecordStateForTest({ recording: true });
    selectAllOf(50_000);
    const anchorMs = 1_000_000;
    // The operation's telemetry: heard position 120 000 (past the 100 000-sample document, an
    // Insert take growing) moving at the document rate.
    onTelemetry(buildVxtmFrame(120_000, anchorMs * 1e6, 48_000));

    await addMarker({ repeat: false, timeStamp: anchorMs + 10 } as KeyboardEvent);

    expect(calls).toHaveLength(1);
    expect(calls[0]!.lenSamples).toBe(0);
    // 10 ms after the anchor at 48 kHz: 120 480 (± the clock-sync offset, well under 1 ms).
    expect(Math.abs(calls[0]!.posSamples - 120_480)).toBeLessThanOrEqual(48);
    expect(isTakeMarker(7)).toBe(true);
    expect(markersState().list.map((m) => m.id)).toEqual([7]);
    stop();
  });

  it("outside a take, markers are not take markers", async () => {
    mockIPC((cmd) => {
      if (cmd === "marker_add") {
        return { id: 3, pos_samples: 0, len_samples: 0, name: "Marker 01" } satisfies MarkerDto;
      }
      return null;
    });
    await addMarker();
    expect(isTakeMarker(3)).toBe(false);
  });
});
