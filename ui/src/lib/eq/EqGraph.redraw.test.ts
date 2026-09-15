import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { CurveHandleDto, ParamInfoDto, RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
import { resetRackForTest } from "../rack/rack.svelte";
import { paramInfoDto, rackSlotDto } from "../test/fixtures";
import EqGraph from "./EqGraph.svelte";

/**
 * H-32 regression: the EQ graph stopped drawing its curve, node handles and axis labels at wide
 * window sizes (the ticket's evidence: a 2126px-wide window) while the app was still settling its
 * first layout, but drew fine at 1280px — and the owner's real app fixed itself after a *manual*
 * window resize. Root cause: `EqGraph.svelte` scheduled its draw from `$effect(() => draw())`, but
 * a Svelte `$effect` only re-subscribes to the state it actually read on its *last* run. A draw
 * that bailed out at an unsettled width (0, or a stale flex-layout guess) never read `curve` or
 * `nodes` that run, so their later arrival never woke the effect up again — only a subsequent
 * change to something it *did* read (`width`, from a real resize) did. Every other canvas
 * renderer in this codebase (`WaveformView`, `SpectralView`, `AnalyzerPanel`) instead draws on a
 * perpetual `requestAnimationFrame` loop that isn't gated by dependency tracking at all, so a bad
 * first measurement just self-heals on the next frame — `EqGraph` now does the same.
 *
 * jsdom has neither a real 2D canvas context nor `ResizeObserver` (see other EQ/renderer tests'
 * comments), so both are faked here, following `render/glContext.test.ts`'s `getContext`-stubbing
 * pattern.
 */

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

  /** Simulates the browser reporting a real content width, e.g. once the rack column settles. */
  trigger(width: number): void {
    this.#callback([{ contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver);
  }
}

/** A `CanvasRenderingContext2D` stand-in that counts calls per method, so a test can assert "did
 * the grid/labels/curve/nodes get drawn" without pinning down every draw call's exact arguments. */
function fakeCtx(): CanvasRenderingContext2D & { calls: Record<string, number> } {
  const calls: Record<string, number> = {};
  const target: Record<string, unknown> = { calls };
  const handler: ProxyHandler<Record<string, unknown>> = {
    get(obj, prop) {
      if (prop in obj) {
        return obj[prop as string];
      }
      const spy = (..._args: unknown[]): void => {
        calls[prop as string] = (calls[prop as string] ?? 0) + 1;
      };
      obj[prop as string] = spy;
      return spy;
    },
    set(obj, prop, value) {
      obj[prop as string] = value;
      return true;
    },
  };
  return new Proxy(target, handler) as unknown as CanvasRenderingContext2D & { calls: Record<string, number> };
}

/** One HP-like node (freq/enable only) and one peak-like node (freq/gain/Q/enable) — same shape
 * as `EqGraph.test.ts`'s fixture. */
function slotFixture(): RackSlotDto {
  const handles: CurveHandleDto[] = [
    { component: 0, freq: 11, gain: null, q: null, enable: 10 },
    { component: 2, freq: 31, gain: 32, q: 33, enable: 30 },
  ];
  const params: ParamInfoDto[] = [
    paramInfoDto({ id: 10, key: "hp_on", unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    paramInfoDto({ id: 11, key: "hp_freq_hz", unit: { kind: "hz" }, min: 20, max: 20_000, default: 80 }),
    paramInfoDto({ id: 30, key: "b1_on", unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    paramInfoDto({ id: 31, key: "b1_freq_hz", unit: { kind: "hz" }, min: 20, max: 20_000, default: 1_000 }),
    paramInfoDto({ id: 32, key: "b1_gain_db", unit: { kind: "db" }, min: -24, max: 24, default: 6 }),
    paramInfoDto({ id: 33, key: "b1_q", unit: { kind: "none" }, min: 0.1, max: 30, default: 2 }),
  ];
  return rackSlotDto({
    module: "org.powervoice.parametric-eq@1.0.0",
    module_id: "org.powervoice.parametric-eq",
    name: "Parametric EQ",
    params,
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    curve_handles: handles,
  });
}

let clientWidthDescriptor: PropertyDescriptor | undefined;
let getContextDescriptor: PropertyDescriptor | undefined;
let originalResizeObserver: unknown;

function stubClientWidth(px: number): void {
  clientWidthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
}

function stubGetContext(ctx: unknown): void {
  getContextDescriptor = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "getContext");
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ctx });
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => resolve());
    } else {
      setTimeout(resolve, 20);
    }
  });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
  if (clientWidthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", clientWidthDescriptor);
    clientWidthDescriptor = undefined;
  }
  if (getContextDescriptor) {
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", getContextDescriptor);
    getContextDescriptor = undefined;
  }
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = originalResizeObserver;
  FakeResizeObserver.instances.length = 0;
});

describe("EqGraph redraw (H-32: first-layout measuring bug)", () => {
  it("draws nothing at an unmeasured (0px) canvas, then draws the grid, labels, curve and node handles once a resize reports the real width — including the exact width that regressed (2126px)", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") {
        return {
          freqs_hz: [100, 1_000],
          sample_rate_hz: 48_000,
          total_db: [1, 2],
          components_db: [],
        } satisfies ResponseCurveDto;
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });

    originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
    const ctx = fakeCtx();
    stubGetContext(ctx);
    stubClientWidth(0); // H-32: the rack column hasn't settled to its real width at first paint

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqGraph, {
      target,
      props: { slotIndex: 0, rackSlot: slotFixture(), rateHz: 48_000 },
    });
    flushSync();

    await nextFrame();
    expect(ctx.calls.arc ?? 0).toBe(0); // nothing to draw yet at width 0

    const ro = FakeResizeObserver.instances.at(-1);
    expect(ro).toBeDefined();
    ro!.trigger(2126); // the exact width H-32's evidence regressed at
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    await nextFrame();

    expect(ctx.calls.arc ?? 0).toBeGreaterThanOrEqual(2); // both fixture nodes
    expect(ctx.calls.fillText ?? 0).toBeGreaterThan(0); // axis labels

    unmount(app);
  });

  it("keeps drawing every frame even if one frame's draw throws (the context stack never gets stuck)", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") {
        return { freqs_hz: [], sample_rate_hz: 48_000, total_db: [], components_db: [] } satisfies ResponseCurveDto;
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });

    originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
    const ctx = fakeCtx();
    // The very first `clearRect` throws once — like a transient failure mid-frame — then behaves.
    let thrown = false;
    (ctx as unknown as Record<string, unknown>).clearRect = () => {
      ctx.calls.clearRect = (ctx.calls.clearRect ?? 0) + 1;
      if (!thrown) {
        thrown = true;
        throw new Error("transient");
      }
    };
    stubGetContext(ctx);
    stubClientWidth(300);

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqGraph, {
      target,
      props: { slotIndex: 0, rackSlot: slotFixture(), rateHz: 48_000 },
    });
    flushSync();

    await nextFrame(); // frame 1: throws inside drawInner
    await nextFrame(); // frame 2: the perpetual loop tries again regardless

    expect(ctx.calls.clearRect ?? 0).toBeGreaterThanOrEqual(2);
    expect(ctx.calls.arc ?? 0).toBeGreaterThanOrEqual(2); // frame 2 completed a full draw
    // save/restore stay balanced across the thrown frame (H-32: `finally` always restores).
    expect(ctx.calls.save ?? 0).toBe(ctx.calls.restore ?? 0);

    unmount(app);
  });
});
