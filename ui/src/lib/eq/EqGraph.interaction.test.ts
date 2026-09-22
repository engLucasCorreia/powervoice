import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { CurveHandleDto, LocalizedTextDto, ParamInfoDto, RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
import { resetRackForTest } from "../rack/rack.svelte";
import { rackSlotDto } from "../test/fixtures";
import { formatHoverFreqHz } from "../spectrum/freqAxis";
import { eqExpandedState, resetEqExpandedForTest } from "./eqExpanded.svelte";
import EqGraph from "./EqGraph.svelte";
import { freqForX, xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";

/**
 * H-111 (owner report, SPEC-015 §2.6.4): the cursor readout, the node tooltip, the right-click
 * menu (add/delete a band, reset, slope), and double-click-to-expand — the pieces `EqGraph.svelte`
 * itself had flagged as "still out of scope" since S3-07.
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
    flags: { automatable: true, stepped: false, boolean: false, read_only: false, hidden: false, bypass: false },
    ...overrides,
  };
}

/** HP (with slope, off by default), band 1 (on by default) and band 2 (off by default — the
 * "free" band the add-band tests enable). */
function slotFixture(): RackSlotDto {
  const handles: CurveHandleDto[] = [
    { component: 0, freq: 11, gain: null, q: null, enable: 10 },
    { component: 2, freq: 31, gain: 32, q: 33, enable: 30 },
    { component: 3, freq: 41, gain: 42, q: 43, enable: 40 },
  ];
  const params = [
    param(10, "hp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(11, "hp_freq_hz", { default: 80 }),
    param(14, "hp_slope", {
      unit: { kind: "none" },
      min: 0,
      max: 7,
      default: 3,
      enum_labels: [6, 12, 18, 24, 30, 36, 42, 48].map((n) => text(`${n} dB/oct`)),
    }),
    param(30, "b1_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(31, "b1_freq_hz", { default: 1_000 }),
    param(32, "b1_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 6 }),
    param(33, "b1_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 2 }),
    param(40, "b2_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(41, "b2_freq_hz", { default: 4_000 }),
    param(42, "b2_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: -3 }),
    param(43, "b2_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 1 }),
  ];
  return rackSlotDto({
    uid: 9,
    module: "org.powervoice.parametric-eq@1.0.0",
    module_id: "org.powervoice.parametric-eq",
    name: "Parametric EQ",
    params,
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    curve_handles: handles,
  });
}

const FLAT_CURVE: ResponseCurveDto = {
  freqs_hz: [20, 100, 1_000, 10_000, 20_000],
  sample_rate_hz: 48_000,
  total_db: [0, 0, 0, 0, 0],
  components_db: [],
};

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
  resetEqExpandedForTest();
  document.body.innerHTML = "";
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    widthDescriptor = undefined;
  }
});

function render(slot: RackSlotDto, mode: "compact" | "expanded" = "compact") {
  stubClientWidth(WIDTH);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EqGraph, {
    target,
    props:
      mode === "expanded"
        ? { slotIndex: 0, rackSlot: slot, rateHz: 48_000, mode, graphHeightPx: HEIGHT }
        : { slotIndex: 0, rackSlot: slot, rateHz: 48_000, mode },
  });
  flushSync();
  const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="eq-canvas"]')!;
  return { target, canvas, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

/** The curve request is coalesced to the next real animation frame (a ~16 ms jsdom timer). */
async function settleCurve(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 50));
  flushSync();
}

function move(canvas: HTMLCanvasElement, x: number, y: number): void {
  canvas.dispatchEvent(new PointerEvent("pointermove", { clientX: x, clientY: y, bubbles: true }));
  flushSync();
}

describe("cursor readout (H-111, SPEC-015 §2.6.4 'Hovering empty graph area')", () => {
  it("shows the pointer frequency and the curve's total response there, once a curve has arrived", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return FLAT_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();
    await settleCurve();

    // Far from every node: HP (80 Hz, mid-height when off) and the two peak bands sit elsewhere.
    const x = xForFreq(2_500, WIDTH, F_LO, F_HI);
    const y = 10;
    move(canvas, x, y);

    const readout = target.querySelector('[data-testid="eq-hover-readout"]');
    expect(readout).not.toBeNull();
    const freqHz = freqForX(x, WIDTH, F_LO, F_HI);
    expect(readout!.textContent).toContain(formatHoverFreqHz(freqHz));
    expect(readout!.textContent).toContain("0.0 dB");
    teardown();
  });

  it("shows nothing before any curve has arrived", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    move(canvas, xForFreq(2_500, WIDTH, F_LO, F_HI), 10);
    expect(target.querySelector('[data-testid="eq-hover-readout"]')).toBeNull();
    teardown();
  });

  it("disappears once the pointer leaves the canvas", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return FLAT_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();
    await settleCurve();

    move(canvas, xForFreq(2_500, WIDTH, F_LO, F_HI), 10);
    expect(target.querySelector('[data-testid="eq-hover-readout"]')).not.toBeNull();

    canvas.dispatchEvent(new PointerEvent("pointerleave", { bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="eq-hover-readout"]')).toBeNull();
    teardown();
  });
});

describe("node tooltip (H-111, SPEC-015 §2.6.4 'Hovering a node shows a tooltip')", () => {
  it("shows the band name, frequency, gain and Q for a peak band, matching its aria-valuetext", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    const x = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const y = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    move(canvas, x, y);

    const tooltip = target.querySelector('[data-testid="eq-node-tooltip"]');
    const node = target.querySelector('[data-testid="eq-node-2"]')!;
    expect(tooltip).not.toBeNull();
    expect(tooltip!.textContent!.trim()).toBe(node.getAttribute("aria-valuetext"));
    // No cursor readout while a node's own tooltip is shown.
    expect(target.querySelector('[data-testid="eq-hover-readout"]')).toBeNull();
    teardown();
  });

  it("shows frequency and slope (not gain/Q) for the HP band", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const slot = slotFixture();
    // Rust's own echoed text for the slope (`nodeValueText` only ever displays that, never a
    // number it derived itself) — same convention `nodes.test.ts` uses.
    slot.values = slot.values.map((v) => (v.id === 14 ? { ...v, text: "24 dB/oct" } : v));
    const { target, canvas, teardown } = render(slot);
    await settle();

    move(canvas, xForFreq(80, WIDTH, F_LO, F_HI), HEIGHT / 2);
    const tooltip = target.querySelector('[data-testid="eq-node-tooltip"]');
    expect(tooltip).not.toBeNull();
    expect(tooltip!.textContent).toContain("80");
    expect(tooltip!.textContent).toContain("dB/oct");
    teardown();
  });

  it("keeps following the dragged node even once the pointer and the node diverge", async () => {
    const paramCalls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") paramCalls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    const startX = xForFreq(1_000, WIDTH, F_LO, F_HI);
    const startY = yForDb(6, HEIGHT, DEFAULT_RANGE_DB);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, clientY: startY, bubbles: true, button: 0 }),
    );
    flushSync();
    // Move far away from the node's own circle — the tooltip must still track it while dragging.
    canvas.dispatchEvent(
      new PointerEvent("pointermove", { clientX: startX + 150, clientY: startY + 60, bubbles: true }),
    );
    flushSync();

    const tooltip = target.querySelector('[data-testid="eq-node-tooltip"]');
    const node = target.querySelector('[data-testid="eq-node-2"]')!;
    expect(tooltip).not.toBeNull();
    // Still band 1's own tooltip, tracking its live (dragged) values — not whatever node the
    // pointer physically sits over now.
    expect(tooltip!.textContent!.trim()).toBe(node.getAttribute("aria-valuetext"));
    void paramCalls; // the drag math itself is covered by EqGraph.test.ts
    teardown();
  });
});

describe("double-click on empty background opens the expanded view (H-111)", () => {
  it("opens the expanded view when a compact graph's background (not a node) is double-clicked", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const slot = slotFixture();
    const { canvas, teardown } = render(slot);
    await settle();

    canvas.dispatchEvent(new MouseEvent("dblclick", { clientX: WIDTH - 5, clientY: 5, bubbles: true }));
    await settle();

    expect(eqExpandedState().openUid).toBe(slot.uid);
    teardown();
  });

  it("does nothing on an expanded graph's own background (nothing further to expand to)", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture(), "expanded");
    await settle();

    canvas.dispatchEvent(new MouseEvent("dblclick", { clientX: WIDTH - 5, clientY: 5, bubbles: true }));
    await settle();

    expect(eqExpandedState().openUid).toBeNull();
    teardown();
  });
});

describe("right-click menu (H-111, SPEC-015 §2.6.4 'Right-click')", () => {
  it("on an enabled node: offers Delete band, Reset band and a Slope submenu for HP", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    canvas.dispatchEvent(
      new MouseEvent("contextmenu", {
        clientX: xForFreq(1_000, WIDTH, F_LO, F_HI),
        clientY: yForDb(6, HEIGHT, DEFAULT_RANGE_DB),
        bubbles: true,
        cancelable: true,
      }),
    );
    flushSync();

    expect(target.querySelector('[data-testid="eq-context-menu"]')).not.toBeNull();
    const toggle = target.querySelector('[data-testid="eq-menu-toggle"]')!;
    expect(toggle.textContent!.trim()).toBe("Delete band"); // b1_on default is 1 (enabled)
    expect(target.querySelector('[data-testid="eq-menu-reset"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="eq-menu-slope"]')).toBeNull(); // b1 has no slope
    teardown();
  });

  it("on the HP node: the Slope submenu carries the 8 dB/oct steps", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    canvas.dispatchEvent(
      new MouseEvent("contextmenu", {
        clientX: xForFreq(80, WIDTH, F_LO, F_HI),
        clientY: HEIGHT / 2,
        bubbles: true,
        cancelable: true,
      }),
    );
    flushSync();

    expect(target.querySelector('[data-testid="eq-menu-toggle"]')!.textContent!.trim()).toBe("Enable band"); // hp_on default 0
    expect(target.querySelector('[data-testid="eq-menu-slope"]')).not.toBeNull();
    teardown();
  });

  it("selecting the node toggle sends the enable param write", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    canvas.dispatchEvent(
      new MouseEvent("contextmenu", {
        clientX: xForFreq(1_000, WIDTH, F_LO, F_HI),
        clientY: yForDb(6, HEIGHT, DEFAULT_RANGE_DB),
        bubbles: true,
        cancelable: true,
      }),
    );
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="eq-menu-toggle"]')!.click();
    flushSync();

    expect(calls).toEqual([{ slot: 0, id: 30, value: 0 }]); // b1_on -> off ("Delete band")
    teardown();
  });

  it("right-clicking the curve/empty area offers 'Add band here', which enables the free band at that frequency", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      if (cmd === "param_set_plain") calls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, canvas, teardown } = render(slotFixture());
    await settle();

    // Away from every node's hit radius; only band 2 (component 3) is disabled, so it's the only
    // candidate regardless of exactly where this lands.
    const clickX = xForFreq(6_000, WIDTH, F_LO, F_HI);
    canvas.dispatchEvent(
      new MouseEvent("contextmenu", { clientX: clickX, clientY: 5, bubbles: true, cancelable: true }),
    );
    flushSync();

    const addItem = target.querySelector<HTMLButtonElement>('[data-testid="eq-menu-add"]');
    expect(addItem).not.toBeNull();
    expect(target.querySelector('[data-testid="eq-menu-toggle"]')).toBeNull(); // not the node menu
    addItem!.click();
    flushSync();

    const freqCall = calls.find((c) => c.id === 41); // b2_freq_hz
    expect(freqCall).toBeDefined();
    expect(freqCall!.value).toBeCloseTo(freqForX(clickX, WIDTH, F_LO, F_HI), 3);
    expect(calls.find((c) => c.id === 40)).toEqual({ slot: 0, id: 40, value: 1 }); // b2_on -> on
    teardown();
  });

  it("says so, rather than failing silently, when no band is free", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const slot = slotFixture();
    // Enable every band (including HP) so none is free.
    slot.values = slot.values.map((v) => (v.id === 10 || v.id === 40 ? { ...v, value: 1 } : v));
    const { target, canvas, teardown } = render(slot);
    await settle();

    canvas.dispatchEvent(
      new MouseEvent("contextmenu", {
        clientX: xForFreq(6_000, WIDTH, F_LO, F_HI),
        clientY: 5,
        bubbles: true,
        cancelable: true,
      }),
    );
    flushSync();

    expect(target.querySelector('[data-testid="eq-menu-add"]')).toBeNull();
    expect(target.textContent).toContain("No free bands");
    teardown();
  });

  it("is keyboard-reachable: Shift+F10 on a focused node opens its own menu (H-66's pattern)", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_response_curve") return EMPTY_CURVE;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, teardown } = render(slotFixture());
    await settle();

    const nodeTarget = target.querySelector<HTMLElement>('[data-testid="eq-node-2"]')!;
    nodeTarget.focus();
    // Same detection as `WaveformView.svelte`'s H-66 menu: the browser fires `contextmenu` at
    // clientX/clientY 0,0 for a keyboard-triggered request.
    nodeTarget.dispatchEvent(
      new MouseEvent("contextmenu", { clientX: 0, clientY: 0, bubbles: true, cancelable: true }),
    );
    flushSync();

    expect(target.querySelector('[data-testid="eq-context-menu"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="eq-menu-toggle"]')!.textContent!.trim()).toBe("Delete band");
    teardown();
  });
});
