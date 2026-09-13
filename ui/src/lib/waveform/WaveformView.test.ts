import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import WaveformView from "./WaveformView.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetDocumentStateForTest();
});

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
});
