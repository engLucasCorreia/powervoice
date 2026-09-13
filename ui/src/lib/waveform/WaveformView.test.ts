import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, RecordStateDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { initDocument, openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { initRecord, resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, selectionState } from "../state/selection.svelte";
import WaveformView from "./WaveformView.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetDocumentStateForTest();
  resetRecordForTest();
  resetSelectionForTest();
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

// S2-01, SPEC-006 §2.9: click-drag creates a selection at exact document samples; a plain click
// (no movement) clears it instead. The viewport is zoomed to fit exactly (8 000 samples over
// 800 px = 10 samples/px, `startSample = 0`), so `sample = round(clientX * 10)` and jsdom's
// zeroed `getBoundingClientRect` makes `clientX` itself the in-canvas pixel.
describe("WaveformView selection (S2-01)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => px,
    });
  }

  afterEach(() => {
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  async function openFixture(lenSamples: number): Promise<void> {
    const fixture: DocumentDto = {
      name: "take.wav",
      path: "/home/user/take.wav",
      sample_rate_hz: 48_000,
      len_samples: lenSamples,
      dirty: false,
      audio_rev: 1,
    };
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
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
  }

  it("a click-drag creates a selection at exact document samples", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 10, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 50, bubbles: true }),
    );
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 100, endSample: 500 });

    unmount(app);
    target.remove();
  });

  it("a plain click (no drag) clears the selection", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    // First, an actual drag to have something to clear.
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).not.toBeNull();

    // Then a plain click (mousedown/up at the same point, no movement) clears it.
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 20, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 20, bubbles: true }));
    flushSync();
    expect(selectionState().current).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Ctrl+A selects the entire document", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    // Ctrl+A -> "waveform.select_all" is exercised in `keymap.test.ts`; here we only check
    // `WaveformView`'s registered handler, so no `attachKeymap()` listener is needed.
    const { dispatchAction } = await import("../keymap");
    dispatchAction("waveform.select_all");
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 0, endSample: 8_000 });

    unmount(app);
    target.remove();
  });

  it("Esc clears the selection", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).not.toBeNull();

    const { dispatchAction } = await import("../keymap");
    dispatchAction("waveform.deselect");
    flushSync();
    expect(selectionState().current).toBeNull();

    unmount(app);
    target.remove();
  });
});
