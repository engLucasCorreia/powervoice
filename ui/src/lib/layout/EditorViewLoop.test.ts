import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearActionHandlers } from "../shortcuts";
import { resetSelectionForTest } from "../state/selection.svelte";
import { resetSpectralForTest } from "../state/spectral.svelte";
import { initTransport, resetTransportForTest, toggleLoop } from "../state/transport.svelte";
import { resetWaveformViewForTest, waveformViewApi } from "../state/waveformView.svelte";
import { docDto, transportStateDto } from "../test/fixtures";
import EditorView from "./EditorView.svelte";

/** H-37 (SPEC-006 §2.12 amendment): the loop region's bar on the shared time ruler. */

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetSelectionForTest();
  resetSpectralForTest();
  resetWaveformViewForTest();
  resetDocumentStateForTest();
  if (widthDescriptor) Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  if (heightDescriptor) Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
});

function headerOnlyVxpk(): ArrayBuffer {
  const buf = new ArrayBuffer(48);
  const dv = new DataView(buf);
  [0x56, 0x58, 0x50, 0x4b].forEach((b, i) => dv.setUint8(i, b));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  return buf;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 3; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}

describe("the ruler's loop bar", () => {
  it("spans the engine's loop_range while looping, and disappears when loop is off", async () => {
    const doc = docDto();
    let loopEnabled = true;
    const state = () =>
      transportStateDto({
        doc_len_samples: doc.len_samples,
        doc_rate_hz: doc.sample_rate_hz,
        can_play: true,
        loop_enabled: loopEnabled,
        loop_range: loopEnabled ? [60_000, 120_000] : null,
      });
    mockIPC((cmd, args) => {
      switch (cmd) {
        case "document_open":
          return doc;
        case "peaks_get":
          return headerOnlyVxpk();
        case "clock_now_ns":
          return 0;
        case "transport_get":
        case "transport_seek":
        case "transport_set_selection":
          return state();
        case "transport_set_loop":
          loopEnabled = (args as { enabled: boolean }).enabled;
          return state();
        default:
          return null;
      }
    });
    stubSize(800, 400);
    const stop = await initTransport();
    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();

    const bar = () => target.querySelector<HTMLElement>('[data-testid="editor-ruler-loop"]');
    const { startSample, samplesPerPixel } = waveformViewApi();
    expect(samplesPerPixel).toBeGreaterThan(0);
    const left = (60_000 - startSample) / samplesPerPixel + 48; // past the 48 px gutter
    const width = 60_000 / samplesPerPixel;
    expect(bar()).not.toBeNull();
    expect(Number.parseFloat(bar()!.style.left)).toBeCloseTo(left, 3);
    expect(Number.parseFloat(bar()!.style.width)).toBeCloseTo(width, 3);

    await toggleLoop();
    await settle();
    expect(bar()).toBeNull();

    unmount(app);
    target.remove();
    stop();
  });
});
