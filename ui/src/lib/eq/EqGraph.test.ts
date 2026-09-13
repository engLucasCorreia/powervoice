import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type {
  CurveHandleDto,
  LocalizedTextDto,
  ParamInfoDto,
  RackSlotDto,
  ResponseCurveDto,
} from "../ipc/bindings";
import { flushPendingPlainDrags, resetRackForTest } from "../rack/rack.svelte";
import EqGraph from "./EqGraph.svelte";
import { freqForX, xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";

/**
 * EQ graph pointer-gesture tests (S3-07, SPEC-015 §2.6.4, ticket scope: drag and wheel — AC-18's
 * lean-slice subset). Uses mocked IPC (`@tauri-apps/api/mocks`) for `rack_response_curve` and
 * `param_set_plain`, and jsdom's zeroed `getBoundingClientRect` (canvas at (0,0)), so
 * `event.clientX`/`clientY` are themselves the in-canvas pixel (same trick `WaveformView.test.ts`
 * uses).
 */

const WIDTH = 400;
const HEIGHT = 160;
const F_LO = 20;
const F_HI = 20_000;
const DEFAULT_RANGE_DB = 12;

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
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...overrides,
  };
}

/** One HP-like node (freq/enable only, no gain/Q) and one peak-like node (freq/gain/Q/enable). */
function slotFixture(): RackSlotDto {
  const handles: CurveHandleDto[] = [
    { component: 0, freq: 11, gain: null, q: null, enable: 10 },
    { component: 2, freq: 31, gain: 32, q: 33, enable: 30 },
  ];
  const params = [
    param(10, "hp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(11, "hp_freq_hz", { default: 80 }),
    param(30, "b1_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(31, "b1_freq_hz", { default: 1_000 }),
    param(32, "b1_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 6 }),
    param(33, "b1_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 2 }),
  ];
  return {
    uid: 1,
    module: "org.powervoice.parametric-eq@1.0.0",
    name: "Parametric EQ",
    bypass: false,
    latency_samples: 0,
    status: { kind: "active" },
    params,
    groups: [],
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    noise_profile: null,
    curve_handles: handles,
  };
}

const EMPTY_CURVE: ResponseCurveDto = {
  freqs_hz: [],
  sample_rate_hz: 48_000,
  total_db: [],
  components_db: [],
};

let widthDescriptor: PropertyDescriptor | undefined;

function stubClientWidth(px: number): void {
  widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    widthDescriptor = undefined;
  }
});

function render(slot: RackSlotDto) {
  stubClientWidth(WIDTH);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EqGraph, { target, props: { slotIndex: 0, rackSlot: slot, rateHz: 48_000 } });
  flushSync();
  const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="eq-canvas"]')!;
  return { target, canvas, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

describe("curve fetch (SPEC-015 §2.6.6)", () => {
  it("requests rack_response_curve for the target slot on mount", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") {
        calls.push(args);
        return EMPTY_CURVE;
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { teardown } = render(slotFixture());
    // The request is coalesced to the next real animation frame (jsdom: a ~16 ms timer), unlike
    // the drag/wheel/dblclick tests below which flush their own coalescer synchronously.
    await new Promise((resolve) => setTimeout(resolve, 50));
    await settle();
    expect(calls.length).toBeGreaterThan(0);
    const call = calls[0] as { slot: number; points: number[] };
    expect(call.slot).toBe(0);
    expect(call.points).toContain(80); // the HP node's exact frequency (SPEC-015 §4.10)
    expect(call.points).toContain(1_000); // the peak node's exact frequency
    teardown();
  });
});

describe("node drag (S3-07, SPEC-015 §2.6.4, AC-18 lean subset)", () => {
  it("dragging the peak node emits param_set_plain for frequency and gain, mapped through the inverse axis", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") {
        return EMPTY_CURVE;
      }
      if (cmd === "param_set_plain") {
        calls.push(args as { slot: number; id: number; value: number });
        return { slots: [], ab: false, latency_samples: 0 };
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    const startX = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const startY = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, clientY: startY, bubbles: true, pointerId: 1 }),
    );
    // Move 20px right (higher freq) and 10px up (higher gain, smaller y).
    canvas.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: startX + 20,
        clientY: startY - 10,
        bubbles: true,
        pointerId: 1,
      }),
    );
    flushPendingPlainDrags();
    await settle();

    const freqCall = calls.find((c) => c.id === 31);
    const gainCall = calls.find((c) => c.id === 32);
    expect(freqCall).toBeDefined();
    expect(gainCall).toBeDefined();
    const expectedFreq = xForFreq(1_000, WIDTH, F_LO, F_HI) + 20;
    expect(freqCall!.value).toBeCloseTo(freqForX(expectedFreq, WIDTH, F_LO, F_HI), 3);
    teardown();
  });

  it("Shift scales the drag delta by 0.1 (fine)", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") {
        calls.push(args as { id: number; value: number });
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    const startX = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const startY = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, clientY: startY, bubbles: true, pointerId: 1 }),
    );
    canvas.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: startX + 100,
        clientY: startY,
        shiftKey: true,
        bubbles: true,
        pointerId: 1,
      }),
    );
    flushPendingPlainDrags();
    await settle();

    const freqCall = calls.find((c) => c.id === 31)!;
    expect(freqCall.value).toBeCloseTo(freqForX(startX + 10, WIDTH, F_LO, F_HI), 3);
    teardown();
  });

  it("HP node drags never emit a gain (no gain parameter)", async () => {
    const calls: Array<{ id: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args as { id: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    // HP node: freq 80 Hz, no curve yet so its y falls back to 0 dB (mid-height).
    const startX = xForFreq(80, WIDTH, F_LO, F_HI);
    const startY = HEIGHT / 2;
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, clientY: startY, bubbles: true, pointerId: 2 }),
    );
    canvas.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: startX + 15,
        clientY: startY + 50, // vertical movement must be ignored for HP
        bubbles: true,
        pointerId: 2,
      }),
    );
    flushPendingPlainDrags();
    await settle();

    expect(calls.some((c) => c.id === 11)).toBe(true); // frequency did move
    expect(calls.some((c) => c.id === 32 || c.id === 33)).toBe(false); // never a gain/Q id
    teardown();
  });
});

describe("wheel = Q (S3-07 ticket scope)", () => {
  it("scrolling over a node with a Q parameter multiplies it by 2^(1/6) per notch", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    const x = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const y = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    canvas.dispatchEvent(
      new WheelEvent("wheel", { clientX: x, clientY: y, deltaY: -100, bubbles: true, cancelable: true }),
    );
    await settle();

    const qCall = calls.find((c) => c.id === 33);
    expect(qCall).toBeDefined();
    expect(qCall!.value).toBeCloseTo(2 * 2 ** (1 / 6), 6); // default Q 2, one notch up
    teardown();
  });

  it("does nothing for a band with no Q (HP), leaving the wheel event free to scroll the panel", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args);
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    const x = xForFreq(80, WIDTH, F_LO, F_HI);
    const y = HEIGHT / 2;
    const event = new WheelEvent("wheel", { clientX: x, clientY: y, deltaY: -100, bubbles: true, cancelable: true });
    canvas.dispatchEvent(event);
    await settle();
    expect(calls).toHaveLength(0);
    expect(event.defaultPrevented).toBe(false);
    teardown();
  });
});

describe("double-click toggles the band (S3-07 ticket scope)", () => {
  it("double-clicking a node flips its enable parameter", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await settle();

    const x = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const y = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    canvas.dispatchEvent(new MouseEvent("dblclick", { clientX: x, clientY: y, bubbles: true }));
    await settle();

    const enableCall = calls.find((c) => c.id === 30);
    expect(enableCall).toBeDefined();
    expect(enableCall!.value).toBe(0); // band 1 defaults on (1) -> toggled off
    teardown();
  });
});
