import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import type { DocumentDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { resetSelectionForTest, selectionState } from "../state/selection.svelte";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import { docDto, transportStateDto } from "../test/fixtures";
import { resetWaveformViewForTest, waveformViewApi } from "../state/waveformView.svelte";
import EditorView from "./EditorView.svelte";

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

function unstubSize(): void {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
}

const FIXTURE: DocumentDto = docDto();

/** Header-only `VXPK` (no buckets) — enough for these layout/viewport smoke tests. */
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

function setupIpc(overrides: Partial<DocumentDto> = {}): void {
  const doc = { ...FIXTURE, ...overrides };
  mockIPC((cmd, args) => {
    if (cmd === "document_open") {
      return doc;
    }
    if (cmd === "peaks_get") {
      return headerOnlyVxpk();
    }
    if (cmd === "transport_seek") {
      // H-12: a restored `waveform_view.cursor_samples` is applied via `seek()` — a real
      // `transport_seek` always answers with a `TransportStateDto`, never `null`.
      const at = (args as { positionSamples: number }).positionSamples;
      return transportStateDto({
        playhead_samples: at,
        play_start_samples: at,
        doc_len_samples: doc.len_samples,
        doc_rate_hz: doc.sample_rate_hz,
        can_play: doc.len_samples > 0,
      });
    }
    return null;
  });
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

beforeEach(() => {
  mockIPC(() => null);
});

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetSpectralForTest();
  resetWaveformViewForTest();
  resetDocumentStateForTest();
  resetSelectionForTest();
  unstubSize();
});

describe("EditorView split layout (T-207, SPEC-007 §2.1)", () => {
  it("shows only the waveform pane by default (spectral pane hidden)", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="editor-waveform-pane"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="editor-divider"]')).toBeNull();
    expect(target.querySelector('[data-testid="editor-spectral-pane"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("shows the divider and spectral pane once toggled visible", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    flushSync();

    spectralState().toggle();
    flushSync();

    expect(target.querySelector('[data-testid="editor-divider"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="editor-spectral-pane"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("defaults the split to 50/50", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    const app = mount(EditorView, { target });
    flushSync();

    const wave = target.querySelector<HTMLElement>('[data-testid="editor-waveform-pane"]')!;
    const spec = target.querySelector<HTMLElement>('[data-testid="editor-spectral-pane"]')!;
    expect(wave.style.flex).toBe("50 1 0%");
    expect(spec.style.flex).toBe("50 1 0%");

    unmount(app);
    target.remove();
  });

  it("dragging the divider changes the split ratio, clamped to [0, 100]", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    const app = mount(EditorView, { target });
    flushSync();

    const container = target.querySelector<HTMLElement>('[data-testid="editor"]')!;
    // jsdom's getBoundingClientRect is all zeros by default; stub a 200px-tall editor.
    container.getBoundingClientRect = () =>
      ({ top: 0, height: 200, left: 0, width: 100, right: 100, bottom: 200, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;

    const divider = target.querySelector<HTMLElement>('[data-testid="editor-divider"]')!;
    divider.setPointerCapture = () => {};
    divider.releasePointerCapture = () => {};
    divider.dispatchEvent(new PointerEvent("pointerdown", { clientY: 100, bubbles: true }));
    divider.dispatchEvent(new PointerEvent("pointermove", { clientY: 150, bubbles: true }));
    flushSync();

    expect(spectralState().splitRatio).toBeCloseTo(75, 6);

    // Dragging past the bottom clamps to 100 (waveform-only collapses the spectral strip).
    divider.dispatchEvent(new PointerEvent("pointermove", { clientY: 1000, bubbles: true }));
    flushSync();
    expect(spectralState().splitRatio).toBe(100);

    divider.dispatchEvent(new PointerEvent("pointerup", { clientY: 1000, bubbles: true }));

    unmount(app);
    target.remove();
  });

  it("double-clicking the divider resets the split to 50%", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    spectralState().setSplitRatio(20);
    const app = mount(EditorView, { target });
    flushSync();

    const divider = target.querySelector<HTMLElement>('[data-testid="editor-divider"]')!;
    divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    flushSync();

    expect(spectralState().splitRatio).toBe(50);

    unmount(app);
    target.remove();
  });
});

describe("EditorView shared ruler/scrollbar (H-12, SPEC-007 §2.1's ruler → waveform → divider → spectral → scrollbar)", () => {
  it("shows no ruler/scrollbar with no document open, both once one opens", async () => {
    mockIPC(() => null);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="editor-ruler"]')).toBeNull();
    expect(target.querySelector('[data-testid="editor-scrollbar"]')).toBeNull();

    stubSize(800, 400);
    setupIpc();
    await openDocument("/home/user/take.wav");
    await settle();

    expect(target.querySelector('[data-testid="editor-ruler"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="editor-scrollbar"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("the shared scrollbar drives the one viewport store both panes bind to", async () => {
    stubSize(800, 400);
    setupIpc();
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();
    // Zoomed to fit: samplesPerPixel = 480_000 / 800 = 600, so max scroll is 0 (the whole file
    // already fits the viewport) — pick a document long enough that some room to scroll exists by
    // reading it back from the store rather than assuming a value.
    const wv = waveformViewApi();
    const before = wv.startSample;

    const scrollbar = target.querySelector<HTMLInputElement>('[data-testid="editor-scrollbar"]')!;
    // Force some scrollable room by zooming in first (halves samplesPerPixel).
    wv.samplesPerPixel = wv.samplesPerPixel / 4;
    flushSync();
    scrollbar.value = "1000";
    scrollbar.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(waveformViewApi().startSample).toBe(1_000);
    expect(waveformViewApi().startSample).not.toBe(before);

    unmount(app);
    target.remove();
  });

  it("restores a saved in-range waveform_view instead of zooming to fit (SPEC-018 §2.6.5)", async () => {
    stubSize(800, 400);
    setupIpc({
      waveform_view: {
        start_sample: 12_000,
        samples_per_pixel: 100, // well within [0.1, zoom-full = 480000/800 = 600]
        selection: { start_sample: 1_000, end_sample: 2_000 },
        cursor_samples: 1_500,
      },
    });

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();

    expect(waveformViewApi().startSample).toBe(12_000);
    expect(waveformViewApi().samplesPerPixel).toBe(100);
    expect(selectionState().current).toEqual({ startSample: 1_000, endSample: 2_000 });

    const scrollbar = target.querySelector<HTMLInputElement>('[data-testid="editor-scrollbar"]')!;
    expect(scrollbar.value).toBe("12000");

    unmount(app);
    target.remove();
  });

  it("an out-of-range samples_per_pixel falls back to zoom-full (SPEC-018 §2.6.5)", async () => {
    stubSize(800, 400);
    setupIpc({
      waveform_view: {
        start_sample: 12_000,
        samples_per_pixel: 1e9, // far beyond zoom-full for this document/viewport
        selection: null,
        cursor_samples: 0,
      },
    });

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();

    // Zoom-full: samplesPerPixel = len_samples / viewportPx = 480_000 / 800 = 600, start 0.
    expect(waveformViewApi().samplesPerPixel).toBe(600);
    expect(waveformViewApi().startSample).toBe(0);

    unmount(app);
    target.remove();
  });

  it("ruler ticks use compact adaptive labels and are offset past the amplitude/frequency gutter (H-24 item 7)", async () => {
    stubSize(800, 400);
    setupIpc();
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();

    const ruler = target.querySelector('[data-testid="editor-ruler"]')!;
    expect(ruler.querySelector('[data-testid="editor-ruler-gutter"]')).not.toBeNull();

    const tickEls = Array.from(ruler.querySelectorAll<HTMLElement>(".tick"));
    expect(tickEls.length).toBeGreaterThan(0);
    // Compact "m:ss" labels (no "00:00:00.000" transport-style padding) at this zoom.
    for (const el of tickEls) {
      expect(el.textContent).toMatch(/^\d+:\d{2}$/);
    }
    // Every tick sits at or past the 48px gutter width (SPEC-006 §2.1: the ruler spans the
    // canvas, not the amplitude gutter) — the leftmost tick (sample 0) sits exactly at it.
    const leftPx = tickEls.map((el) => Number(el.style.left.replace("px", "")));
    expect(Math.min(...leftPx)).toBe(48);

    unmount(app);
    target.remove();
  });

  it("stacks ruler -> waveform -> divider -> spectral -> scrollbar (SPEC-007 §2.1)", async () => {
    stubSize(800, 400);
    setupIpc();
    await openDocument("/home/user/take.wav");
    spectralState().setVisible(true);

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    await settle();

    const editor = target.querySelector('[data-testid="editor"]')!;
    const testids = Array.from(editor.children).map((el) => el.getAttribute("data-testid"));
    expect(testids).toEqual([
      "editor-ruler",
      "editor-waveform-pane",
      "editor-divider",
      "editor-spectral-pane",
      "editor-scrollbar",
    ]);

    unmount(app);
    target.remove();
  });
});
