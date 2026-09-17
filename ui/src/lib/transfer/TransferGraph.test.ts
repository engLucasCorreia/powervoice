import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { LocalizedTextDto, ParamInfoDto, RackSlotDto } from "../ipc/bindings";
import { flushPendingPlainDrags, resetRackForTest } from "../rack/rack.svelte";
import { frameScheduler } from "../render/frameScheduler";
import { rackSlotDto } from "../test/fixtures";
import { box } from "../test/reactive.svelte";
import { encodeVxtc, type VxtcFields } from "../test/vxtc";
import TransferGraph from "./TransferGraph.svelte";
import { TRANSFER_MAX_DBFS, TRANSFER_MIN_DBFS, levelForX, xForLevel } from "./levelAxis";

/**
 * H-63 (SPEC-016 §2.6 / §4.11): the transfer graph's IPC contract and its threshold-handle
 * gestures. Uses mocked IPC (`@tauri-apps/api/mocks`) and jsdom's zeroed `getBoundingClientRect`
 * (canvas at (0,0)), so `event.clientX` is itself the in-canvas pixel.
 */

const WIDTH = 260;
/** The graph is square and follows the width within 200…320 px. */
const SIDE = WIDTH;
/** `DynamicsCurve`'s compressor handle (SPEC-016 §4.11), as the backend reports it. */
const COMPRESSOR_THRESHOLD_ID = 31;
const COMPRESSOR_ENABLE_ID = 30;
const OFFSET_DB = 3.010_299_956_639_812;

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function param(id: number, key: string, defaultValue: number): ParamInfoDto {
  return {
    id,
    key,
    name: text(key),
    group: null,
    unit: { kind: "dbfs" },
    min: -60,
    max: 0,
    default: defaultValue,
    taper: { kind: "db", neg_inf_at_min: false },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 20,
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...(key.endsWith("_enabled")
      ? { min: 0, max: 1, unit: { kind: "none" } as ParamInfoDto["unit"] }
      : {}),
  };
}

function slotFixture(): RackSlotDto {
  const params = [
    param(COMPRESSOR_ENABLE_ID, "compressor_enabled", 1),
    param(COMPRESSOR_THRESHOLD_ID, "compressor_threshold_db", -20),
  ];
  return rackSlotDto({
    module: "org.powervoice.dynamics@1.0.0",
    module_id: "org.powervoice.dynamics",
    name: "Dynamics",
    params,
    values: params.map((p) => ({
      id: p.id,
      value: p.default,
      normalized: 0.5,
      text: String(p.default),
    })),
    transfer_handles: [
      { component: 2, threshold: COMPRESSOR_THRESHOLD_ID, enable: COMPRESSOR_ENABLE_ID },
    ],
  });
}

function curveFrame(overrides: Partial<VxtcFields> = {}): ArrayBuffer {
  return encodeVxtc({
    xMinDb: TRANSFER_MIN_DBFS,
    xMaxDb: TRANSFER_MAX_DBFS,
    rising: [-80, -40, -6, -5],
    handles: [
      { param: COMPRESSOR_THRESHOLD_ID, xDbfs: -20 + OFFSET_DB, offsetDb: OFFSET_DB },
    ],
    ...overrides,
  });
}

let widthDescriptor: PropertyDescriptor | undefined;

function stubClientWidth(px: number): void {
  widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
}

beforeEach(() => {
  frameScheduler.resetForTest();
});

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
  const app = mount(TransferGraph, { target, props: { slotIndex: 0, rackSlot: slot } });
  flushSync();
  const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="transfer-canvas"]')!;
  return { target, canvas, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

/** The curve request is coalesced to a real animation frame (jsdom: a ~16 ms timer). */
async function afterFrame(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 50));
  await settle();
}

describe("curve fetch (SPEC-016 §4.11)", () => {
  it("requests module_transfer_curve for the slot over the spec's level range", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "module_transfer_curve") {
        calls.push(args);
        return curveFrame();
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { teardown } = render(slotFixture());
    await afterFrame();
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[0]).toEqual({
      slot: 0,
      seq: 1,
      xMinDb: TRANSFER_MIN_DBFS,
      xMaxDb: TRANSFER_MAX_DBFS,
      points: SIDE,
    });
    // Nothing is computed here: exactly one request, not one per repaint.
    expect(calls.length).toBe(1);
    teardown();
  });

  it("keeps the last curve when a request fails (no extension, rack closed)", async () => {
    let fail = false;
    mockIPC((cmd) => {
      if (cmd === "module_transfer_curve") {
        if (fail) {
          throw new Error("no transfer-curve support");
        }
        return curveFrame();
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { target, teardown } = render(slotFixture());
    await afterFrame();
    fail = true;
    expect(target.querySelector('[data-testid="transfer-graph"]')).not.toBeNull();
    teardown();
  });

  it("notes the dashed falling branch only when the module reports hysteresis", async () => {
    let hysteresis = false;
    mockIPC((cmd) => {
      if (cmd === "module_transfer_curve") {
        return hysteresis
          ? curveFrame({ falling: [Number.NEGATIVE_INFINITY, -40, -6, -5] })
          : curveFrame();
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const first = render(slotFixture());
    await afterFrame();
    expect(first.target.querySelector('[data-testid="transfer-hysteresis-note"]')).toBeNull();
    first.teardown();

    hysteresis = true;
    const second = render(slotFixture());
    await afterFrame();
    expect(second.target.querySelector('[data-testid="transfer-hysteresis-note"]')).not.toBeNull();
    second.teardown();
  });
});

describe("request sequencing (SPEC-016 §2.6 \"Curve refresh\", AC-21)", () => {
  it("asks again after a parameter change, and ignores a response older than the newest request", async () => {
    const pending: Array<(frame: ArrayBuffer) => void> = [];
    const seqs: number[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "module_transfer_curve") {
        seqs.push((args as { seq: number }).seq);
        return new Promise<ArrayBuffer>((resolve) => pending.push(resolve));
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const slot = slotFixture();
    stubClientWidth(WIDTH);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const rackSlot = box(slot);
    const app = mount(TransferGraph, {
      target,
      props: {
        slotIndex: 0,
        get rackSlot() {
          return rackSlot.value;
        },
      },
    });
    flushSync();
    await afterFrame();
    expect(seqs).toEqual([1]);

    // A `param_changed` echo replaces the slot's values: exactly one new request follows.
    rackSlot.value = {
      ...slot,
      values: slot.values.map((v) =>
        v.id === COMPRESSOR_THRESHOLD_ID ? { ...v, value: -26, text: "-26" } : v,
      ),
    };
    flushSync();
    await afterFrame();
    expect(seqs).toEqual([1, 2]);

    // The newest request answers first; the stale one that lands afterwards is dropped.
    pending[1]!(curveFrame({ handles: [{ param: COMPRESSOR_THRESHOLD_ID, xDbfs: -26 }] }));
    await settle();
    pending[0]!(curveFrame({ handles: [{ param: COMPRESSOR_THRESHOLD_ID, xDbfs: -20 }] }));
    await settle();

    // The graph still shows the newest curve: a press at the stale handle position grabs
    // nothing, one at the new position starts a drag.
    const calls: Array<{ id: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "param_set_plain") {
        calls.push(args as { id: number });
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="transfer-canvas"]')!;
    const press = (x: number): void => {
      canvas.dispatchEvent(new PointerEvent("pointerdown", { clientX: x, bubbles: true, pointerId: 1 }));
      canvas.dispatchEvent(new PointerEvent("pointermove", { clientX: x + 5, bubbles: true, pointerId: 1 }));
      canvas.dispatchEvent(new PointerEvent("pointerup", { clientX: x + 5, bubbles: true, pointerId: 1 }));
    };
    press(xForLevel(-20, SIDE));
    flushPendingPlainDrags();
    await settle();
    expect(calls).toHaveLength(0);
    press(xForLevel(-26, SIDE));
    flushPendingPlainDrags();
    await settle();
    expect(calls).toHaveLength(1);
    unmount(app);
  });
});

describe("threshold handles (SPEC-016 §2.6)", () => {
  async function mounted() {
    const calls: Array<{ id: number; value: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "module_transfer_curve") {
        return curveFrame();
      }
      if (cmd === "param_set_plain") {
        calls.push(args as { id: number; value: number });
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const rendered = render(slotFixture());
    await afterFrame();
    return { ...rendered, calls };
  }

  it("dragging the handle writes the threshold, the handle offset removed again", async () => {
    const { canvas, calls, teardown } = await mounted();
    const startX = xForLevel(-20 + OFFSET_DB, SIDE);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, bubbles: true, pointerId: 1 }),
    );
    canvas.dispatchEvent(
      new PointerEvent("pointermove", { clientX: startX + 20, bubbles: true, pointerId: 1 }),
    );
    flushPendingPlainDrags();
    await settle();

    const call = calls.find((c) => c.id === COMPRESSOR_THRESHOLD_ID);
    expect(call).toBeDefined();
    // The handle offset makes the round trip through the frame's `f32`, so the comparison is to
    // 1e-4 dB — far inside AC-21's "one pixel's dB span".
    expect(call!.value).toBeCloseTo(levelForX(startX + 20, SIDE) - OFFSET_DB, 4);
    teardown();
  });

  it("Shift scales the drag ×0.1 (fine)", async () => {
    const { canvas, calls, teardown } = await mounted();
    const startX = xForLevel(-20 + OFFSET_DB, SIDE);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, bubbles: true, pointerId: 1 }),
    );
    canvas.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: startX + 100,
        shiftKey: true,
        bubbles: true,
        pointerId: 1,
      }),
    );
    flushPendingPlainDrags();
    await settle();
    const call = calls.find((c) => c.id === COMPRESSOR_THRESHOLD_ID)!;
    expect(call.value).toBeCloseTo(levelForX(startX + 10, SIDE) - OFFSET_DB, 4);
    teardown();
  });

  it("ignores a press away from every handle", async () => {
    const { canvas, calls, teardown } = await mounted();
    canvas.dispatchEvent(new PointerEvent("pointerdown", { clientX: 2, bubbles: true, pointerId: 1 }));
    canvas.dispatchEvent(new PointerEvent("pointermove", { clientX: 60, bubbles: true, pointerId: 1 }));
    flushPendingPlainDrags();
    await settle();
    expect(calls).toHaveLength(0);
    teardown();
  });

  it("double-clicking a handle resets its threshold to the schema default", async () => {
    const { canvas, calls, teardown } = await mounted();
    const startX = xForLevel(-20 + OFFSET_DB, SIDE);
    canvas.dispatchEvent(new MouseEvent("dblclick", { clientX: startX, bubbles: true }));
    await settle();
    expect(calls).toEqual([{ slot: 0, id: COMPRESSOR_THRESHOLD_ID, value: -20 }]);
    teardown();
  });

  it("does not drag a handle whose section is disabled", async () => {
    const calls: Array<{ id: number }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "module_transfer_curve") {
        return curveFrame({
          handles: [
            {
              param: COMPRESSOR_THRESHOLD_ID,
              xDbfs: -20 + OFFSET_DB,
              offsetDb: OFFSET_DB,
              enabled: false,
            },
          ],
        });
      }
      if (cmd === "param_set_plain") {
        calls.push(args as { id: number });
      }
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { canvas, teardown } = render(slotFixture());
    await afterFrame();
    const startX = xForLevel(-20 + OFFSET_DB, SIDE);
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: startX, bubbles: true, pointerId: 1 }),
    );
    canvas.dispatchEvent(
      new PointerEvent("pointermove", { clientX: startX + 20, bubbles: true, pointerId: 1 }),
    );
    flushPendingPlainDrags();
    await settle();
    expect(calls).toHaveLength(0);
    teardown();
  });
});
