import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { rectsOverlap } from "../ui/axisLabels";
import SpectrumPlot from "./SpectrumPlot.svelte";
import type { PlotCurve } from "./plotGeometry";

/**
 * H-42 (SPEC-007 §8.1–§8.2): the shared spectrum plot's DOM overlays — peak labels (placed
 * without collisions, toggleable), keyboard zoom/pan and the hover readout with the note.
 * jsdom has no 2D canvas, so drawing itself is a no-op here; everything tested is DOM.
 */

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

afterEach(() => {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
  document.body.innerHTML = "";
});

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function toneCurve(tones: Array<[number, number]>): PlotCurve {
  const fs = 48_000;
  const fft = 16_384;
  const bins = fft / 2 + 1;
  const freqsHz = Float64Array.from({ length: bins }, (_, k) => (k * fs) / fft);
  const levelsDb = new Float32Array(bins).fill(-100);
  for (const [f, level] of tones) {
    const c = (f * fft) / fs;
    for (let k = Math.floor(c) - 3; k <= Math.ceil(c) + 3; k++) {
      levelsDb[k] = Math.max(levelsDb[k] ?? -100, level - 6 * (k - c) ** 2);
    }
  }
  return { freqsHz, levelsDb, resolution: "bins" };
}

function mountPlot(props: Record<string, unknown>) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(SpectrumPlot, {
    target,
    props: { maxHz: 24_000, floorDb: -120, ceilDb: 0, testid: "t", curve: null, ...props },
  });
  return { target, app };
}

describe("SpectrumPlot overlays (H-42)", () => {
  it("labels the strongest peaks with frequency, note and level, never overlapping", async () => {
    stubSize(700, 220);
    const curve = toneCurve([
      [220, -20],
      [440, -24],
      [466, -26],
      [880, -30],
      [3000, -40],
      [6300, -45],
    ]);
    const { target, app } = mountPlot({ curve, peakLabels: true });
    await wait(150);
    flushSync();
    const labels = [...target.querySelectorAll<HTMLElement>('[data-testid="t-peak-label"]')];
    expect(labels.length).toBeGreaterThanOrEqual(3);
    expect(labels[0]!.textContent).toContain("220 Hz");
    expect(labels[0]!.textContent).toContain("A3");
    expect(labels[0]!.textContent).toMatch(/−20\.0 dB/);
    const rects = labels.map((el) => ({
      x: Number.parseFloat(el.style.left),
      y: Number.parseFloat(el.style.top),
      width: Number.parseFloat(el.style.width),
      height: 28,
    }));
    for (const r of rects) {
      expect(r.x).toBeGreaterThanOrEqual(0);
      expect(r.y).toBeGreaterThanOrEqual(0);
      expect(r.x + r.width).toBeLessThanOrEqual(700);
    }
    for (let i = 0; i < rects.length; i++) {
      for (let j = i + 1; j < rects.length; j++) {
        expect(rectsOverlap(rects[i]!, rects[j]!)).toBe(false);
      }
    }
    unmount(app);
  });

  it("shows no labels with the toggle off", async () => {
    stubSize(700, 220);
    const { target, app } = mountPlot({ curve: toneCurve([[220, -20]]), peakLabels: false });
    await wait(80);
    flushSync();
    expect(target.querySelectorAll('[data-testid="t-peak-label"]').length).toBe(0);
    unmount(app);
  });

  it("is keyboard operable: + zooms in, arrows pan, 0 resets", async () => {
    stubSize(700, 220);
    const { target, app } = mountPlot({ curve: toneCurve([[1000, -20]]) });
    await wait(20);
    flushSync();
    const axisText = () => target.querySelector('[data-testid="t-freq-axis"]')!.textContent;
    const wrap = target.querySelector<HTMLElement>('[data-testid="t-canvas-wrap"]')!;
    expect(wrap.tabIndex).toBe(0);
    expect(wrap.getAttribute("aria-label")).toContain("Spectrum graph");
    const full = axisText();
    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "+", bubbles: true }));
    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "+", bubbles: true }));
    flushSync();
    const zoomed = axisText();
    expect(zoomed).not.toBe(full);
    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    flushSync();
    expect(axisText()).not.toBe(zoomed);
    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "0", bubbles: true }));
    flushSync();
    expect(axisText()).toBe(full);
    unmount(app);
  });

  it("the hover crosshair readout names the note at the pointer", async () => {
    stubSize(700, 220);
    const { target, app } = mountPlot({ curve: toneCurve([[440, -20]]), peakLabels: true });
    await wait(20);
    const canvas = target.querySelector("canvas")!;
    // x for 440 Hz on the 20 Hz … 24 kHz log axis.
    const x = (Math.log2(440 / 20) / Math.log2(24_000 / 20)) * 700;
    canvas.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: 40, bubbles: true }));
    flushSync();
    const hover = target.querySelector('[data-testid="t-hover"]')!;
    expect(hover.textContent).toMatch(/A4/);
    expect(hover.textContent).toMatch(/Hz/);
    unmount(app);
  });

  it("shows a B − A difference in the readout when both snapshots are overlaid", async () => {
    stubSize(700, 220);
    const a = toneCurve([[1000, -30]]);
    const b = toneCurve([[1000, -24]]);
    const { target, app } = mountPlot({
      curve: toneCurve([[1000, -20]]),
      overlays: [
        { key: "a", curve: a, tone: "a" },
        { key: "b", curve: b, tone: "b" },
      ],
      diffAB: true,
    });
    await wait(20);
    const x = (Math.log2(1000 / 20) / Math.log2(24_000 / 20)) * 700;
    target.querySelector("canvas")!.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: 40, bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="t-hover"]')!.textContent).toMatch(/B − A \+\d/);
    unmount(app);
  });
});
