import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { CurveHandleDto, LocalizedTextDto, ParamInfoDto, RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
import { resetRackForTest } from "../rack/rack.svelte";
import { frameScheduler } from "../render/frameScheduler";
import { rackSlotDto } from "../test/fixtures";
import { deliverChannelMessage } from "../test/liveFrame";
import { encodeVxsa } from "../test/vxsa";
import { closeEqExpanded, eqExpandedState, resetEqExpandedForTest } from "./eqExpanded.svelte";
import EqGraph from "./EqGraph.svelte";

/**
 * Spectrum overlay + Expand button tests (H-84):
 * - AC-21's subscribe/unsubscribe lifecycle (subscribes while visible with Spectrum on,
 *   unsubscribes on toggle-off or unmount).
 * - The overlay must not keep the frame scheduler awake once nothing changes (ticket: "must not
 *   keep the scheduler awake when the analyzer is idle").
 * - The Expand button opens the `eqExpanded.svelte.ts` store the way `EqExpandedView.svelte` reads it.
 *
 * (The x-agreement between an analyzer band and a curve point, and the idle *dedup*, are unit
 * tested at the pure-math/feed level in `spectrumOverlay.test.ts`/`spectrumFeed.test.ts` — this
 * file only exercises the component's own lifecycle wiring.)
 */

const EMPTY_CURVE: ResponseCurveDto = {
  freqs_hz: [],
  sample_rate_hz: 48_000,
  total_db: [],
  components_db: [],
};

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function param(id: number, key: string, overrides: Partial<ParamInfoDto> = {}): ParamInfoDto {
  return {
    id,
    key,
    name: text(key),
    group: null,
    unit: { kind: "hz" },
    min: 20,
    max: 20_000,
    default: 1_000,
    taper: { kind: "log" },
    step: null,
    enum_labels: [],
    decimals: 0,
    smoothing_ms: 20,
    flags: { automatable: true, stepped: false, boolean: false, read_only: false, hidden: false, bypass: false },
    ...overrides,
  };
}

function slotFixture(): RackSlotDto {
  const handles: CurveHandleDto[] = [{ component: 0, freq: 11, gain: null, q: null, enable: 10 }];
  const params = [
    param(10, "hp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(11, "hp_freq_hz", { default: 80 }),
  ];
  return rackSlotDto({
    uid: 7,
    module: "org.powervoice.parametric-eq@1.0.0",
    module_id: "org.powervoice.parametric-eq",
    name: "Parametric EQ",
    params,
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    curve_handles: handles,
  });
}

let widthDescriptor: PropertyDescriptor | undefined;

function stubClientWidth(px: number): void {
  widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  resetEqExpandedForTest();
  document.body.innerHTML = "";
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    widthDescriptor = undefined;
  }
});

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

function render(slot: RackSlotDto, subscribeCalls: unknown[], unsubscribeCalls: unknown[]) {
  mockIPC((cmd, args) => {
    if (cmd === "rack_response_curve") {
      return EMPTY_CURVE;
    }
    if (cmd === "analyzer_subscribe") {
      subscribeCalls.push(args);
      return subscribeCalls.length; // a fresh id per call
    }
    if (cmd === "analyzer_unsubscribe") {
      unsubscribeCalls.push(args);
      return undefined;
    }
    return { slots: [], ab: false, latency_samples: 0 };
  });
  stubClientWidth(400);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EqGraph, { target, props: { slotIndex: 0, rackSlot: slot, rateHz: 48_000 } });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

describe("spectrum subscription lifecycle (AC-21)", () => {
  it("subscribes on mount, since Spectrum defaults on", async () => {
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    const { teardown } = render(slotFixture(), sub, unsub);
    await settle();
    expect(sub.length).toBe(1);
    teardown();
  });

  it("unsubscribes when the Spectrum toggle is switched off", async () => {
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    const { target, teardown } = render(slotFixture(), sub, unsub);
    await settle();
    expect(sub.length).toBe(1);

    const toggle = target.querySelector<HTMLButtonElement>('[data-testid="eq-spectrum-toggle"]')!;
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    toggle.click();
    await settle();
    expect(unsub.length).toBe(1);
    expect(toggle.getAttribute("aria-pressed")).toBe("false");

    // Switching back on subscribes again (a fresh subscriber, not a stale reused one).
    toggle.click();
    await settle();
    expect(sub.length).toBe(2);
    teardown();
  });

  it("unsubscribes on unmount (the graph closed/hidden)", async () => {
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    const { teardown } = render(slotFixture(), sub, unsub);
    await settle();
    expect(sub.length).toBe(1);
    expect(unsub.length).toBe(0);
    teardown();
    await settle();
    expect(unsub.length).toBe(1);
  });
});

describe("idle behaviour (ticket: must not keep the scheduler awake when the analyzer is idle)", () => {
  it("schedules no further frames once settled, with no analyzer traffic", async () => {
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    frameScheduler.resetForTest();
    const { teardown } = render(slotFixture(), sub, unsub);
    await settle();
    // Let any pending rAF actually run and settle (jsdom's rAF is a real ~16 ms timer, H-43).
    await new Promise((resolve) => setTimeout(resolve, 60));
    flushSync();

    const before = frameScheduler.stats.frames;
    await new Promise((resolve) => setTimeout(resolve, 120));
    flushSync();
    expect(frameScheduler.stats.frames).toBe(before); // no perpetual loop, nothing changed
    teardown();
  });
});

describe("live spectrum overlay draws real analyzer data (H-88, from H-87's open question)", () => {
  /** A `CanvasRenderingContext2D` stand-in that counts calls per method — `EqGraph.redraw.test.ts`'s
   * pattern — so this can tell "the overlay fill path ran" apart from "nothing happened". */
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

  let getContextDescriptor: PropertyDescriptor | undefined;
  function stubGetContext(ctx: unknown): void {
    getContextDescriptor = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "getContext");
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ctx });
  }

  afterEach(() => {
    if (getContextDescriptor) {
      Object.defineProperty(HTMLCanvasElement.prototype, "getContext", getContextDescriptor);
      getContextDescriptor = undefined;
    }
  });

  it("fills the overlay only once a live VXSA frame decodes to real band levels — not before", async () => {
    let channel: Channel<ArrayBuffer> | undefined;
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") {
        return EMPTY_CURVE; // no total-curve fill, so any `fill()` call is the overlay's
      }
      if (cmd === "analyzer_subscribe") {
        channel = (args as { channel: Channel<ArrayBuffer> }).channel;
        sub.push(args);
        return sub.length;
      }
      if (cmd === "analyzer_unsubscribe") {
        unsub.push(args);
        return undefined;
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const ctx = fakeCtx();
    stubGetContext(ctx);
    stubClientWidth(400);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqGraph, { target, props: { slotIndex: 0, rackSlot: slotFixture(), rateHz: 48_000 } });
    flushSync();
    await settle();
    expect(channel).toBeDefined();

    // Before any frame arrives, the fixture's disabled HP node and empty curve draw nothing that
    // calls `fill()` — so a later `fill()` can only be the overlay reacting to the live frame.
    await new Promise((resolve) => setTimeout(resolve, 60));
    flushSync();
    expect(ctx.calls.fill ?? 0).toBe(0);

    // Real VXSA bytes (`encodeVxsa`, H-88), decoded through the component's own analyzer feed —
    // not a hand-rolled `AnalyzerFrame` poked into its state.
    const levelsDb = Array.from({ length: 100 }, () => -30);
    deliverChannelMessage(channel!, encodeVxsa({ levelsDb, f0Hz: 20, bandsPerOctave: 24 }));
    await new Promise((resolve) => setTimeout(resolve, 60));
    flushSync();

    expect(ctx.calls.fill ?? 0).toBeGreaterThan(0);
    unmount(app);
  });
});

describe("Expand button (SPEC-015 §2.6.1)", () => {
  it("opens the expanded-view store for this slot's uid", async () => {
    const sub: unknown[] = [];
    const unsub: unknown[] = [];
    const { target, teardown } = render(slotFixture(), sub, unsub);
    await settle();

    expect(eqExpandedState().openUid).toBeNull();
    const expandBtn = target.querySelector<HTMLButtonElement>('[data-testid="eq-expand-button"]')!;
    expandBtn.click();
    expect(eqExpandedState().openUid).toBe(7);
    closeEqExpanded();
    teardown();
  });

  it("has no Expand button in expanded mode", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") {
        return EMPTY_CURVE;
      }
      if (cmd === "analyzer_subscribe") {
        return 1;
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    stubClientWidth(400);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqGraph, {
      target,
      props: { slotIndex: 0, rackSlot: slotFixture(), rateHz: 48_000, mode: "expanded", graphHeightPx: 300 },
    });
    flushSync();
    await settle();
    expect(target.querySelector('[data-testid="eq-expand-button"]')).toBeNull();
    unmount(app);
  });
});
