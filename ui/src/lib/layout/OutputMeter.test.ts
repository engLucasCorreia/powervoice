import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { VXTM_FLAGS } from "../ipc/telemetry";
import { clearActionHandlers } from "../shortcuts";
import { initTransport, onTelemetry, resetTransportForTest } from "../state/transport.svelte";
import { transportStateDto } from "../test/fixtures";
import OutputMeter from "./OutputMeter.svelte";

/**
 * H-41: the output meter is a fixed-width column whose width never depends on the meter's
 * values (item 1's acceptance test), and its bars/readouts/clip lamp react correctly to
 * telemetry (ballistics, throttled readouts, clip latch — see `state/transport.test.ts` for the
 * ballistics maths itself; these tests are the component-level wiring).
 */

/** jsdom has no `ResizeObserver` (see EqGraph.redraw.test.ts's identical stand-in) — the
 * component's own guarded `$effect` skips wiring a real one, so `trackHeightPx` (and therefore
 * every scale tick) stays 0 unless a test installs this and triggers it manually. */
class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  readonly #callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.#callback = callback;
    FakeResizeObserver.instances.push(this);
  }

  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}

  trigger(height: number): void {
    this.#callback([{ contentRect: { height } } as ResizeObserverEntry], this as unknown as ResizeObserver);
  }
}

let mounted: ReturnType<typeof mount> | null = null;
let originalResizeObserver: unknown;

afterEach(() => {
  if (mounted) {
    unmount(mounted);
    mounted = null;
  }
  document.body.innerHTML = "";
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  if (originalResizeObserver !== undefined) {
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = originalResizeObserver;
    originalResizeObserver = undefined;
  }
  FakeResizeObserver.instances.length = 0;
});

function render(): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  mounted = mount(OutputMeter, { target });
  flushSync();
  return target;
}

/** Every inline `style="..."` in the rendered tree, concatenated — so a test can assert none of
 * them ever mentions `width` (only height/top/bottom/custom properties should). */
function inlineStyles(root: HTMLElement): string {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("style") ?? "")
    .join("|");
}

function u64(dv: DataView, offset: number, value: number): void {
  dv.setUint32(offset, value >>> 0, true);
  dv.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

function buildVxtmFrame(fields: { flags?: number; outPeakDbfs?: number; outRmsDbfs?: number }): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x54);
  dv.setUint8(3, 0x4d);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 72, true);
  dv.setUint32(12, fields.flags ?? 0, true);
  u64(dv, 16, 0);
  u64(dv, 24, 0);
  dv.setFloat64(32, 48_000, true);
  dv.setFloat32(40, fields.outPeakDbfs ?? Number.NEGATIVE_INFINITY, true);
  dv.setFloat32(44, fields.outRmsDbfs ?? Number.NEGATIVE_INFINITY, true);
  return buf;
}

async function setUp(): Promise<() => void> {
  mockIPC((cmd) => {
    if (cmd === "clock_now_ns") return performance.now() * 1e6;
    if (cmd === "transport_get") return transportStateDto({ playing: true, can_play: true });
    return null;
  });
  return initTransport();
}

describe("OutputMeter layout (H-41 item 1: fixed width regardless of the values)", () => {
  it("never sets an inline width anywhere, at the quietest, loudest and mid-scale values", async () => {
    const stop = await setUp();
    const root = render();
    for (const [peak, rms] of [
      [Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY],
      [0, -3],
      [-30, -33],
      [-59.999, -60],
    ]) {
      onTelemetry(buildVxtmFrame({ outPeakDbfs: peak, outRmsDbfs: rms }));
      flushSync();
      expect(inlineStyles(root)).not.toMatch(/(?<![a-zA-Z-])width\s*:/);
    }
    stop();
  });

  it("does react to values through height/bottom (so it's still a live meter, not a static image)", async () => {
    const stop = await setUp();
    const root = render();

    onTelemetry(buildVxtmFrame({ outPeakDbfs: -6, outRmsDbfs: -9 }));
    flushSync();
    const peakFillA = root.querySelector<HTMLElement>('[data-testid="output-meter-peak-fill"]')?.style.height;
    const rmsFillA = root.querySelector<HTMLElement>('[data-testid="output-meter-rms-fill"]')?.style.height;

    onTelemetry(buildVxtmFrame({ outPeakDbfs: -40, outRmsDbfs: -45 }));
    flushSync();
    const peakFillB = root.querySelector<HTMLElement>('[data-testid="output-meter-peak-fill"]')?.style.height;
    const rmsFillB = root.querySelector<HTMLElement>('[data-testid="output-meter-rms-fill"]')?.style.height;

    expect(peakFillA).not.toBe(peakFillB);
    expect(rmsFillA).not.toBe(rmsFillB);
    stop();
  });
});

describe("OutputMeter readouts and clip lamp", () => {
  it("shows the readouts with units, through i18n", async () => {
    const stop = await setUp();
    const root = render();
    onTelemetry(buildVxtmFrame({ outPeakDbfs: -6, outRmsDbfs: -9 }));
    flushSync();
    expect(root.querySelector('[data-testid="output-meter-peak"]')?.textContent).toBe("Peak −6.0 dBFS");
    expect(root.querySelector('[data-testid="output-meter-rms"]')?.textContent).toBe("RMS −9.0 dBFS");
    stop();
  });

  it("shows silence as the localized dash, not -Infinity or NaN", () => {
    render();
    expect(document.querySelector('[data-testid="output-meter-peak"]')?.textContent).toBe("Peak −∞ dBFS");
  });

  it("lights the clip lamp on OUT_CLIP and clears it on click", async () => {
    const stop = await setUp();
    const root = render();
    const lamp = root.querySelector<HTMLButtonElement>('[data-testid="output-meter-clip"]')!;
    expect(lamp.classList.contains("lit")).toBe(false);

    onTelemetry(buildVxtmFrame({ flags: VXTM_FLAGS.OUT_CLIP, outPeakDbfs: 0 }));
    flushSync();
    expect(lamp.classList.contains("lit")).toBe(true);

    lamp.click();
    flushSync();
    expect(lamp.classList.contains("lit")).toBe(false);
    stop();
  });

  it("has an accessible role=meter with the −60…0 dBFS scale", () => {
    const root = render();
    const meter = root.querySelector('[role="meter"]');
    expect(meter?.getAttribute("aria-valuemin")).toBe("-60");
    expect(meter?.getAttribute("aria-valuemax")).toBe("0");
    expect(meter?.getAttribute("aria-label")).toBe("Output level");
  });
});

// H-41 regression: an early build's tick labels all rendered on top of each other (an actual
// screenshot caught this, no test did) — `alignStyle()` returned a *second* `top:` declaration
// alongside the real `top: {y}px` already in the same inline `style` attribute, and CSS keeps only
// the last declaration of a repeated property, so every tick collapsed onto whatever `top` its
// align happened to produce (usually 50%) instead of its own computed position.
describe("OutputMeter scale ticks (H-41 regression: every tick used to render at the same spot)", () => {
  function renderWithHeight(height: number): HTMLElement {
    originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
    const root = render();
    const ro = FakeResizeObserver.instances.at(-1);
    expect(ro).toBeDefined();
    ro!.trigger(height);
    flushSync();
    return root;
  }

  it("places every visible tick's label at its own distinct pixel position", () => {
    const root = renderWithHeight(400);
    const ticks = [...root.querySelectorAll<HTMLElement>(".tick")];
    expect(ticks.length).toBeGreaterThan(3); // a generous height should keep most of the ladder
    const tops = ticks.map((el) => el.style.top);
    expect(new Set(tops).size).toBe(tops.length); // no two ticks share a `top`
    // The exact bug: a duplicated `top:` in the same style attribute. Every tick's inline style
    // must declare `top` exactly once.
    for (const el of ticks) {
      const topDeclarations = (el.getAttribute("style") ?? "").match(/(?<![a-zA-Z-])top\s*:/g) ?? [];
      expect(topDeclarations).toHaveLength(1);
    }
  });

  it("keeps 0 dBFS pinned to the very top and -∞ to the very bottom", () => {
    const root = renderWithHeight(400);
    const ticks = [...root.querySelectorAll<HTMLElement>(".tick")];
    const zero = ticks.find((el) => el.textContent === "0");
    const inf = ticks.find((el) => el.textContent === "−∞");
    expect(zero?.style.top).toBe("0px");
    expect(inf?.style.top).toBe("400px");
  });

  it("degenerates to just 0 and -∞ at a very short height, without colliding", () => {
    const root = renderWithHeight(50);
    const ticks = [...root.querySelectorAll<HTMLElement>(".tick")];
    expect(ticks.some((el) => el.textContent === "0")).toBe(true);
    expect(ticks.some((el) => el.textContent === "−∞")).toBe(true);
    const tops = ticks.map((el) => el.style.top);
    expect(new Set(tops).size).toBe(tops.length);
  });
});
