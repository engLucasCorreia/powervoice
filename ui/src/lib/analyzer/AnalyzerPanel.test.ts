import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetAnalyzerForTest } from "./analyzer.svelte";
import { resetOutputDeviceStatusForTest } from "./outputDeviceStatus.svelte";
import AnalyzerPanel from "./AnalyzerPanel.svelte";

/**
 * H-24 item 5 (SPEC-007 §2.9): the analyzer used to draw grid lines with no frequency/dB labels
 * at all. These tests cover the persistent DOM-based axis labels and the ticket's regression for
 * item 4 ("every canvas sits in a container with a definite size ... never sized from its own
 * content"): the plot area's measured size must not change just because the canvas redraws.
 */

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

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

afterEach(() => {
  clearMocks();
  resetAnalyzerForTest();
  resetOutputDeviceStatusForTest();
  unstubSize();
});

describe("AnalyzerPanel axes (H-24 item 5)", () => {
  it("renders a dB gutter with its unit shown once, and a frequency axis strip", async () => {
    mockIPC(() => null);
    stubSize(400, 120);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const dbAxis = target.querySelector('[data-testid="analyzer-db-axis"]')!;
    expect(dbAxis).not.toBeNull();
    expect(dbAxis.textContent).toContain("dBFS");
    // The unit appears exactly once even though there are several dB ticks.
    expect(dbAxis.textContent?.match(/dBFS/g)?.length).toBe(1);

    const freqAxis = target.querySelector('[data-testid="analyzer-freq-axis"]')!;
    expect(freqAxis).not.toBeNull();
    expect(freqAxis.querySelectorAll(".tick").length).toBeGreaterThan(0);

    unmount(app);
    target.remove();
  });

  it("the canvas backing store tracks the container's size and never grows on its own across redraws (item 4 regression)", async () => {
    // The bug this guards against: sizing the canvas from its own rendered content instead of
    // its container, which grows the container next frame (measured here), which grows the
    // canvas again, etc. `stubSize` pins the container's `clientWidth`/`clientHeight` (jsdom has
    // no real layout engine, MEMORY.md) — the canvas's backing store (`canvasEl.width/height`,
    // a real, readable attribute even in jsdom) must stay locked to that pinned size no matter
    // how many animation frames redraw it.
    mockIPC(() => null);
    stubSize(400, 150);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const canvas = target.querySelector('[data-testid="analyzer-panel"] canvas') as HTMLCanvasElement;
    const expectedW = canvas.width;
    const expectedH = canvas.height;
    expect(expectedW).toBeGreaterThan(0);
    expect(expectedH).toBeGreaterThan(0);

    // Peak-hold ballistics redraw every animation frame (a real timer under jsdom, MEMORY.md) —
    // several frames must never change the backing store size.
    await new Promise((resolve) => setTimeout(resolve, 80));
    flushSync();
    expect(canvas.width).toBe(expectedW);
    expect(canvas.height).toBe(expectedH);

    unmount(app);
    target.remove();
  });
});

// --- H-42: modes, toggles, diagnostics panel (SPEC-007 §8) --------------------------------------

import { diagnosticsState, resetDiagnosticsForTest } from "./diagnostics.svelte";

describe("AnalyzerPanel diagnostics (H-42)", () => {
  afterEach(() => {
    resetDiagnosticsForTest();
    document.body.innerHTML = "";
  });

  function recordCalls(): Array<[string, Record<string, unknown>]> {
    const calls: Array<[string, Record<string, unknown>]> = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
      return cmd === "analyzer_voice_subscribe" ? 77 : null;
    });
    return calls;
  }

  function segment(target: HTMLElement, testid: string, label: string): HTMLElement {
    return [...target.querySelectorAll<HTMLElement>(`[data-testid="${testid}"] [role="radio"], [data-testid="${testid}"] button`)].find(
      (el) => el.textContent?.trim() === label,
    )!;
  }

  it("keeps the live look by default: peaks on, diagnostics hidden, no extra bars", async () => {
    recordCalls();
    stubSize(600, 150);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    expect(target.querySelector('[data-testid="analyzer-peaks-toggle"]')!.getAttribute("aria-pressed")).toBe("true");
    expect(target.querySelector('[data-testid="analyzer-diagnostics-toggle"]')!.getAttribute("aria-pressed")).toBe("false");
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).toBeNull();
    expect(target.querySelector('[data-testid="analyzer-average-bar"]')).toBeNull();
    expect(target.querySelector('[data-testid="analyzer-compare-bar"]')).toBeNull();
    unmount(app);
  });

  it("the Diagnostics toggle shows the panel and holds the live voice stream only while shown", async () => {
    const calls = recordCalls();
    stubSize(900, 180);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-diagnostics-toggle"]')!.click();
    await settle();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).not.toBeNull();
    expect(calls.filter(([c]) => c === "analyzer_voice_subscribe").length).toBe(1);
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-diagnostics-close"]')!.click();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).toBeNull();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id)).toContain(77);
    unmount(app);
  });

  it("Average and Compare modes show their bars; Analyze needs something to analyze", async () => {
    recordCalls();
    stubSize(900, 180);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    segment(target, "analyzer-mode", "Average").click();
    await settle();
    expect(diagnosticsState().mode).toBe("average");
    expect(target.querySelector('[data-testid="analyzer-average-bar"]')).not.toBeNull();
    expect(target.querySelector<HTMLButtonElement>('[data-testid="analyzer-average-analyze"]')!.disabled).toBe(true);
    expect(target.querySelector('[data-testid="analyzer-plot"]')!.textContent).toContain("Open a file");
    segment(target, "analyzer-mode", "Compare").click();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-compare-bar"]')).not.toBeNull();
    // No live frame yet: nothing to freeze.
    expect(target.querySelector<HTMLButtonElement>('[data-testid="analyzer-freeze-a"]')!.disabled).toBe(true);
    unmount(app);
  });
});
