import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RecordStartedDto } from "../ipc/bindings";
import { clearActionHandlers } from "../shortcuts";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { recordState, resetRecordForTest, toggleRecord } from "../state/record.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import { docDto, recordStateDto } from "../test/fixtures";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import WaveformView from "./WaveformView.svelte";

/**
 * H-21 (SPEC-022 §2.11): during a record operation on a document with audio the waveform keeps
 * the normal document view (`peaks_get`, the user's zoom) and additionally polls the live take
 * (`record_peaks_get`) to draw it at the record point — it must not switch to H-07's new-recording
 * view (which zooms to fit the take from sample 0 and hides the document).
 */

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
  resetSelectionForTest();
});

function headerOnlyVxpk(): ArrayBuffer {
  const buf = new ArrayBuffer(48);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x50);
  dv.setUint8(3, 0x4b);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  return buf;
}

const DOC = docDto();

const RECORDING = recordStateDto({ armed: true, input_open: true, recording: true });

describe("WaveformView during a record operation (H-21)", () => {
  it("keeps the document view and polls the live take", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
    const peakRequests: Array<{ start_sample: number }> = [];
    const livePolls: unknown[] = [];
    const started: RecordStartedDto = {
      take_id: 3,
      op: "insert",
      at_samples: 96_000,
      end_samples: null,
      preroll_samples: 0,
      postroll_samples: 0,
      aligned: false,
      state: RECORDING,
    };
    mockIPC((cmd, args) => {
      if (cmd === "document_open") return DOC;
      if (cmd === "peaks_get") {
        peakRequests.push((args as { request: { start_sample: number } }).request);
        return headerOnlyVxpk();
      }
      if (cmd === "record_start_at") return started;
      if (cmd === "record_peaks_get") {
        livePolls.push(args);
        return headerOnlyVxpk();
      }
      return null;
    });
    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target, props: { startSample: 0, samplesPerPixel: 600 } });
    flushSync();
    await Promise.resolve();
    try {
      const before = peakRequests.length;
      expect(before).toBeGreaterThan(0);
      await toggleRecord();
      flushSync();
      await Promise.resolve();
      expect(recordState().op?.op).toBe("insert");
      expect(livePolls.length).toBeGreaterThan(0);
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      // Still the document view from sample 0 (no zoom-to-fit of the take).
      expect(peakRequests.every((r) => r.start_sample === 0)).toBe(true);
    } finally {
      unmount(app);
      target.remove();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
    }
  });
});
