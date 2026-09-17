import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type {
  CurveHandleDto,
  LocalizedTextDto,
  ParamGroupDto,
  ParamInfoDto,
  RackSlotDto,
  ResponseCurveDto,
} from "../ipc/bindings";
import { flushPendingPlainDrags, resetRackForTest } from "../rack/rack.svelte";
import { rackSlotDto } from "../test/fixtures";
import EqGraph from "./EqGraph.svelte";

/**
 * Keyboard node tests (H-84, SPEC-015 §2.6.5, AC-19): tab order, the per-key step table, Enter
 * (toggle), Home (reset to defaults — the *spec's* double-click semantics, not S3-07's toggle
 * deviation), ARIA attributes and the throttled live region.
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

function group(id: number, key: string, enable: number | null): ParamGroupDto {
  return { id, key, name: text(key), parent: null, enable_param: enable, collapsed_by_default: true };
}

/** HP, L (low shelf), band 1, H (high shelf), LP — in `handles()` order, one representative of
 * each parameter shape (freq/enable-only, and freq/gain/Q/enable/slope combinations). */
function slotFixture(): RackSlotDto {
  const handles: CurveHandleDto[] = [
    { component: 0, freq: 11, gain: null, q: null, enable: 10 }, // HP
    { component: 1, freq: 21, gain: 22, q: 23, enable: 20 }, // L (low shelf)
    { component: 2, freq: 31, gain: 32, q: 33, enable: 30 }, // band 1
    { component: 7, freq: 81, gain: 82, q: 83, enable: 80 }, // H (high shelf)
    { component: 8, freq: 91, gain: null, q: null, enable: 90 }, // LP
  ];
  const params = [
    param(10, "hp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(11, "hp_freq_hz", { default: 80 }),
    param(14, "hp_slope", { unit: { kind: "none" }, min: 0, max: 7, default: 3 }),
    param(20, "ls_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(21, "ls_freq_hz", { default: 100 }),
    param(22, "ls_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 0 }),
    param(23, "ls_q", { unit: { kind: "none" }, min: 0.3, max: 2, default: 0.7071 }),
    param(30, "b1_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(31, "b1_freq_hz", { default: 1_000 }),
    param(32, "b1_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 0 }),
    param(33, "b1_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 1 }),
    param(80, "hs_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(81, "hs_freq_hz", { default: 10_000 }),
    param(82, "hs_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 0 }),
    param(83, "hs_q", { unit: { kind: "none" }, min: 0.3, max: 2, default: 0.7071 }),
    param(90, "lp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(91, "lp_freq_hz", { default: 12_000 }),
    param(94, "lp_slope", { unit: { kind: "none" }, min: 0, max: 7, default: 3 }),
  ];
  const values = params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) }));
  const groups: ParamGroupDto[] = [
    group(1, "hp", 10),
    group(2, "low_shelf", 20),
    group(3, "band_1", 30),
    group(4, "high_shelf", 80),
    group(5, "lp", 90),
  ];
  return rackSlotDto({
    module: "org.powervoice.parametric-eq@1.0.0",
    module_id: "org.powervoice.parametric-eq",
    name: "Parametric EQ",
    params,
    values,
    groups,
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
  document.body.innerHTML = "";
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    widthDescriptor = undefined;
  }
});

function mockPlainSet(calls: Array<{ id: number; value: number }>): void {
  mockIPC((cmd, args) => {
    if (cmd === "rack_response_curve") {
      return EMPTY_CURVE;
    }
    if (cmd === "param_set_plain") {
      calls.push(args as { id: number; value: number });
      return { slots: [], ab: false, latency_samples: 0 };
    }
    if (cmd === "analyzer_subscribe") {
      return 1;
    }
    if (cmd === "analyzer_unsubscribe") {
      return undefined;
    }
    return { slots: [], ab: false, latency_samples: 0 };
  });
}

function render(slot: RackSlotDto) {
  stubClientWidth(400);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EqGraph, { target, props: { slotIndex: 0, rackSlot: slot, rateHz: 48_000 } });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

function nodeEl(target: HTMLElement, component: number): HTMLElement {
  return target.querySelector<HTMLElement>(`[data-testid="eq-node-${component}"]`)!;
}

describe("tab order (AC-19: HP, L, 1-5, H, LP)", () => {
  it("lists the focus targets in handles() order", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const targets = [...target.querySelectorAll<HTMLElement>(".node-target")];
    const order = targets.map((el) => el.dataset.testid);
    expect(order).toEqual(["eq-node-0", "eq-node-1", "eq-node-2", "eq-node-7", "eq-node-8"]);
    for (const el of targets) {
      expect(el.tabIndex).toBe(0);
    }
    teardown();
  });
});

describe("ARIA (AC-19)", () => {
  it("gives each node a slider role, roledescription and Rust-shaped valuetext", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const hp = nodeEl(target, 0);
    expect(hp.getAttribute("role")).toBe("slider");
    expect(hp.getAttribute("aria-roledescription")).toBe("EQ band");
    expect(hp.getAttribute("aria-label")).toBe("hp"); // localized(group.name) with our text() stub
    expect(hp.getAttribute("aria-valuetext")).toContain("80"); // hp_freq_hz's text
    expect(hp.getAttribute("aria-valuetext")).toContain("3"); // hp_slope's text (HP has a slope, not gain/Q)
    expect(hp.getAttribute("aria-valuetext")).toContain("(off)"); // hp_on is 0 by default

    const band1 = nodeEl(target, 2);
    const valueText = band1.getAttribute("aria-valuetext")!;
    expect(valueText).toContain("band_1");
    expect(valueText).toContain("1000"); // b1_freq_hz's default text
    teardown();
  });
});

describe("← / → (frequency)", () => {
  it("multiplies the frequency by 2^(1/12), or 2^(1/48) with Shift", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2);
    band1.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    await settle();
    let call = calls.find((c) => c.id === 31)!;
    expect(call.value).toBeCloseTo(1_000 * 2 ** (1 / 12), 6);

    calls.length = 0;
    band1.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowLeft", shiftKey: true, bubbles: true, cancelable: true }),
    );
    await settle();
    call = calls.find((c) => c.id === 31)!;
    expect(call.value).toBeCloseTo(1_000 * 2 ** (-1 / 48), 6);
    teardown();
  });
});

describe("↑ / ↓ (gain, or slope for HP/LP)", () => {
  it("moves gain ±0.5 dB (Shift ±0.1 dB) for a peak/shelf band", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2);
    band1.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 32)!.value).toBeCloseTo(0.5, 9);

    calls.length = 0;
    band1.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowDown", shiftKey: true, bubbles: true, cancelable: true }),
    );
    await settle();
    expect(calls.find((c) => c.id === 32)!.value).toBeCloseTo(-0.1, 9);
    teardown();
  });

  it("steps the slope one notch steeper/shallower for HP/LP instead of a gain", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const hp = nodeEl(target, 0);
    hp.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 14)!.value).toBe(4); // default index 3 -> steeper

    calls.length = 0;
    hp.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 14)!.value).toBe(2); // -> shallower
    expect(calls.some((c) => c.id === 11)).toBe(false); // never a frequency id from ↑/↓
    teardown();
  });
});

describe("PageUp / PageDown (Q)", () => {
  it("multiplies Q by 2^(1/6), the same coarse factor as the wheel", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2);
    band1.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 33)!.value).toBeCloseTo(1 * 2 ** (1 / 6), 9);
    teardown();
  });

  it("does nothing for HP/LP (no Q parameter)", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const hp = nodeEl(target, 0);
    const event = new KeyboardEvent("keydown", { key: "PageUp", bubbles: true, cancelable: true });
    hp.dispatchEvent(event);
    await settle();
    expect(calls).toHaveLength(0);
    teardown();
  });
});

describe("Enter (toggle on/off)", () => {
  it("flips the enable parameter", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2); // on by default
    band1.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 30)!.value).toBe(0);
    teardown();
  });
});

describe("Home (reset the band, SPEC-015 §2.6.4 semantics)", () => {
  it("resets frequency, gain and Q to their defaults for a peak band", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const slot = slotFixture();
    // Move band 1 away from its defaults first.
    slot.values = slot.values.map((v) => (v.id === 31 ? { ...v, value: 5_000 } : v.id === 32 ? { ...v, value: 9 } : v.id === 33 ? { ...v, value: 5 } : v));
    const { target, teardown } = render(slot);
    await settle();

    const band1 = nodeEl(target, 2);
    band1.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 31)!.value).toBe(1_000); // b1_freq_hz default
    expect(calls.find((c) => c.id === 32)!.value).toBe(0); // b1_gain_db default
    expect(calls.find((c) => c.id === 33)!.value).toBe(1); // b1_q default
    expect(calls.some((c) => c.id === 30)).toBe(false); // on/off untouched
    teardown();
  });

  it("resets frequency and slope for HP", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const hp = nodeEl(target, 0);
    hp.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
    await settle();
    expect(calls.find((c) => c.id === 11)!.value).toBe(80); // hp_freq_hz default
    expect(calls.find((c) => c.id === 14)!.value).toBe(3); // hp_slope default
    teardown();
  });
});

describe("Space is never consumed (transport shortcut, T-104)", () => {
  it("does not call preventDefault and sends no command", async () => {
    const calls: unknown[] = [];
    mockPlainSet(calls as Array<{ id: number; value: number }>);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2);
    const event = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
    band1.dispatchEvent(event);
    await settle();
    expect(event.defaultPrevented).toBe(false);
    expect(calls).toHaveLength(0);
    teardown();
  });
});

describe("live region (SPEC-015 §2.6.5, throttled announcements)", () => {
  it("announces the focused node's value text", async () => {
    const calls: Array<{ id: number; value: number }> = [];
    mockPlainSet(calls);
    const { target, teardown } = render(slotFixture());
    await settle();

    const band1 = nodeEl(target, 2);
    band1.dispatchEvent(new FocusEvent("focus", { bubbles: true }));
    await settle();

    const region = target.querySelector('[data-testid="eq-live-region"]')!;
    expect(region.getAttribute("aria-live")).toBe("polite");
    expect(region.textContent).toContain("band_1");
    expect(region.textContent).toContain("1000");
    teardown();
  });
});
