import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, RecordStateDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { initDocument, openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { initRecord, resetRecordForTest } from "../state/record.svelte";
import WaveformView from "./WaveformView.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetDocumentStateForTest();
  resetRecordForTest();
});

function headerOnlyVxpk(): ArrayBuffer {
  // Header-only VXPK (no buckets) is enough for these smoke tests.
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

describe("WaveformView (S1-03)", () => {
  it("shows the empty state with no document open", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="waveform-empty"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-canvas"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("renders the canvas, ruler and scrollbar once a document is open", async () => {
    const fixture: DocumentDto = {
      name: "take.wav",
      path: "/home/user/take.wav",
      sample_rate_hz: 48_000,
      len_samples: 480_000,
      dirty: false,
      audio_rev: 1,
    };
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        // Header-only VXPK (no buckets) is enough for this smoke test.
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
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-ruler"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-scrollbar"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-empty"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  // Regression: the canvas container only renders once a document is open, so the size observer
  // must attach then — not at mount (no document yet), which left the viewport 0 and the view blank.
  it("measures the viewport and requests peaks when a document opens after mount", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const peakRequests: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        return {
          name: "take.wav",
          path: "/home/user/take.wav",
          sample_rate_hz: 48_000,
          len_samples: 480_000,
          dirty: false,
          audio_rev: 1,
        } satisfies DocumentDto;
      }
      if (cmd === "peaks_get") {
        peakRequests.push(args);
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
      throw new Error(`unmocked command: ${cmd}`);
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    expect(peakRequests).toHaveLength(0);

    try {
      await openDocument("/home/user/take.wav");
      flushSync();
      await Promise.resolve();
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      expect(peakRequests.length).toBeGreaterThan(0);
    } finally {
      unmount(app);
      target.remove();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
    }
  });

  // H-07: the take lives only in the take files and is committed to the document at Stop, so the
  // document is empty (len_samples 0) while recording. The view must still show the canvas and
  // draw the growing take from record_peaks_get, polling instead of the normal peaks_get path.
  it("polls record_peaks_get and renders the canvas while recording, without a committed document", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const heightDescriptor = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight",
    );
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    const livePeakRequests: unknown[] = [];
    const recordingDoc: DocumentDto = {
      name: null,
      path: null,
      sample_rate_hz: 48_000,
      len_samples: 0, // S1-04: the take isn't committed until Stop
      dirty: false,
      audio_rev: 0,
    };
    const recordingState: RecordStateDto = {
      input_device: "Mic",
      input_channel: 1,
      input_status: "healthy",
      armed: true,
      input_open: true,
      input_rate_hz: 48_000,
      recording: true,
      finishing: false,
      monitor: "off",
      monitoring: true,
    };
    mockIPC(
      (cmd, args) => {
        if (cmd === "record_get") {
          return recordingState;
        }
        if (cmd === "record_peaks_get") {
          livePeakRequests.push(args);
          return headerOnlyVxpk();
        }
        if (cmd === "peaks_get") {
          throw new Error("peaks_get must not be called while recording (S1-04: empty document)");
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    const stopRecord = initRecord();
    await emit("document_changed", recordingDoc);
    flushSync();
    await Promise.resolve();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await Promise.resolve();

    try {
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      expect(target.querySelector('[data-testid="waveform-empty"]')).toBeNull();
      expect(livePeakRequests.length).toBeGreaterThan(0);
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });
});
