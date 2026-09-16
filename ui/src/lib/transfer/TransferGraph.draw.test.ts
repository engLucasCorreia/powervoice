import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { RackSlotDto, TransferCurveDto } from "../ipc/bindings";
import { resetRackForTest } from "../rack/rack.svelte";
import { frameScheduler } from "../render/frameScheduler";
import { paramInfoDto, rackSlotDto } from "../test/fixtures";
import TransferGraph from "./TransferGraph.svelte";
import { xForLevel, yForLevel } from "./levelAxis";

/**
 * H-63 drawing tests (SPEC-016 §2.6), at the level `EqGraph.redraw.test.ts` covers the EQ graph:
 * the vertices land on the axis-mapped positions, the Falling branch is dashed and only drawn
 * where it differs, handles are drawn for enabled sections only, a muted level breaks the line
 * instead of diving to the corner, an arriving curve wakes the H-43 frame scheduler, and a
 * throwing frame leaves the context stack balanced.
 *
 * jsdom has no 2D context and no `ResizeObserver`, so both are faked (the pattern
 * `EqGraph.redraw.test.ts` and `render/glContext.test.ts` use).
 */

const SIDE = 260;
const TOLERANCE_PX = 0.5;
const THRESHOLD_ID = 31;
const ENABLE_ID = 30;

interface Op {
  name: string;
  args: unknown[];
  /** The path was being stroked with a dash pattern. */
  dash: boolean;
}

/** A `CanvasRenderingContext2D` stand-in that records every call in order, tracking the dash
 * state across `save`/`restore` so a test can tell the solid curve from the dashed one. */
function recordingCtx(onClear?: () => void): CanvasRenderingContext2D & { ops: Op[] } {
  const ops: Op[] = [];
  let dash = false;
  const stack: boolean[] = [];
  const target: Record<string, unknown> = { ops };
  const handler: ProxyHandler<Record<string, unknown>> = {
    get(obj, prop) {
      if (prop in obj) {
        return obj[prop as string];
      }
      const name = prop as string;
      const fn = (...args: unknown[]): unknown => {
        switch (name) {
          case "measureText":
            return { width: String(args[0] ?? "").length * 5 };
          case "setLineDash":
            dash = Array.isArray(args[0]) && (args[0] as number[]).length > 0;
            break;
          case "save":
            stack.push(dash);
            break;
          case "restore":
            dash = stack.pop() ?? false;
            break;
          default:
            break;
        }
        ops.push({ name, args, dash });
        if (name === "clearRect") {
          // Recorded first, so a throw injected here still counts as a frame that started.
          onClear?.();
        }
        return undefined;
      };
      obj[name] = fn;
      return fn;
    },
    set(obj, prop, value) {
      obj[prop as string] = value;
      return true;
    },
  };
  return new Proxy(target, handler) as unknown as CanvasRenderingContext2D & { ops: Op[] };
}

function vertices(ops: Op[], dash: boolean): Array<{ x: number; y: number }> {
  return ops
    .filter((op) => (op.name === "moveTo" || op.name === "lineTo") && op.dash === dash)
    .map((op) => ({ x: op.args[0] as number, y: op.args[1] as number }));
}

function hasVertex(points: Array<{ x: number; y: number }>, x: number, y: number): boolean {
  return points.some((p) => Math.abs(p.x - x) <= TOLERANCE_PX && Math.abs(p.y - y) <= TOLERANCE_PX);
}

function slotFixture(): RackSlotDto {
  const params = [
    paramInfoDto({ id: ENABLE_ID, key: "compressor_enabled", min: 0, max: 1, default: 1 }),
    paramInfoDto({
      id: THRESHOLD_ID,
      key: "compressor_threshold_db",
      unit: { kind: "dbfs" },
      min: -60,
      max: 0,
      default: -20,
    }),
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
    transfer_handles: [{ component: 2, threshold: THRESHOLD_ID, enable: ENABLE_ID }],
  });
}

/** Rising passes below −40 and compresses above it; Falling is 12 dB higher at −40 only (a
 * hysteresis loop), and −80 is muted. */
function curveDto(overrides: Partial<TransferCurveDto> = {}): TransferCurveDto {
  return {
    in_dbfs: [-80, -40, 0, 6],
    rising_db: [-200, -40, -6, -5],
    falling_db: null,
    components_db: [],
    handles: [
      { component: 2, param: THRESHOLD_ID, x_dbfs: -20, offset_db: 0, enabled: true },
    ],
    min_dbfs: -200,
    ...overrides,
  };
}

class FakeResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
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
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
    configurable: true,
    value: () => ctx,
  });
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

beforeEach(() => {
  frameScheduler.resetForTest();
  originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
});

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
});

async function draw(curve: TransferCurveDto, onClear?: () => void) {
  mockIPC((cmd) => {
    if (cmd === "rack_transfer_curve") {
      return curve;
    }
    return { slots: [], ab: false, latency_samples: 0 };
  });
  const ctx = recordingCtx(onClear);
  stubGetContext(ctx);
  stubClientWidth(SIDE);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(TransferGraph, {
    target,
    props: { slotIndex: 0, rackSlot: slotFixture() },
  });
  flushSync();
  // The curve request is coalesced to an animation frame; then the scheduler redraws.
  await new Promise((resolve) => setTimeout(resolve, 50));
  flushSync();
  await nextFrame();
  await nextFrame();
  const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="transfer-canvas"]')!;
  return { ctx, canvas, teardown: () => unmount(app) };
}

describe("transfer graph drawing (SPEC-016 §2.6)", () => {
  it("puts the curve's vertices on the axis-mapped positions and draws the axis labels", async () => {
    const { ctx, teardown } = await draw(curveDto());
    const solid = vertices(ctx.ops, false);
    expect(hasVertex(solid, xForLevel(-40, SIDE), yForLevel(-40, SIDE))).toBe(true);
    expect(hasVertex(solid, xForLevel(0, SIDE), yForLevel(-6, SIDE))).toBe(true);
    expect(hasVertex(solid, xForLevel(6, SIDE), yForLevel(-5, SIDE))).toBe(true);
    expect(ctx.ops.filter((op) => op.name === "fillText").length).toBeGreaterThan(0);
    teardown();
  });

  it("breaks the line at a muted level instead of diving to the corner", async () => {
    const { ctx, teardown } = await draw(curveDto());
    // −80 dBFS reads `min_dbfs`: no vertex is emitted for it, so the polyline starts at −40.
    const solid = vertices(ctx.ops, false);
    expect(hasVertex(solid, xForLevel(-80, SIDE), yForLevel(-200, SIDE))).toBe(false);
    teardown();
  });

  it("draws the dashed 1:1 diagonal", async () => {
    const { ctx, teardown } = await draw(curveDto());
    const dashed = vertices(ctx.ops, true);
    expect(hasVertex(dashed, 0, SIDE)).toBe(true);
    expect(hasVertex(dashed, SIDE, 0)).toBe(true);
    teardown();
  });

  it("dashes the Falling branch only where it differs from Rising", async () => {
    const { ctx, teardown } = await draw(
      curveDto({ falling_db: [-200, -28, -6, -5] }),
    );
    const dashed = vertices(ctx.ops, true);
    // The hysteresis point itself, and the neighbour the range is widened to so the dashed
    // segment meets the solid curve.
    expect(hasVertex(dashed, xForLevel(-40, SIDE), yForLevel(-28, SIDE))).toBe(true);
    expect(hasVertex(dashed, xForLevel(0, SIDE), yForLevel(-6, SIDE))).toBe(true);
    // Where the branches agree the falling branch is not drawn again.
    expect(hasVertex(dashed, xForLevel(6, SIDE), yForLevel(-5, SIDE))).toBe(false);
    teardown();
  });

  it("draws a handle for an enabled section and none for a disabled one", async () => {
    const enabled = await draw(curveDto());
    const triangles = enabled.ctx.ops.filter((op) => op.name === "closePath").length;
    expect(triangles).toBe(1);
    expect(
      vertices(enabled.ctx.ops, false).some(
        (p) => Math.abs(p.x - xForLevel(-20, SIDE)) <= TOLERANCE_PX && p.y === SIDE,
      ),
    ).toBe(true);
    enabled.teardown();

    const dto = curveDto();
    const disabled = await draw({
      ...dto,
      handles: [{ ...dto.handles[0]!, enabled: false }],
    });
    expect(disabled.ctx.ops.filter((op) => op.name === "closePath").length).toBe(0);
    disabled.teardown();
  });

  it("labels the handle with Rust's own text while it is dragged, and not before", async () => {
    const { ctx, canvas, teardown } = await draw(curveDto());
    const texts = () =>
      ctx.ops.filter((op) => op.name === "fillText").map((op) => String(op.args[0]));
    expect(texts()).not.toContain("-20");
    canvas.dispatchEvent(
      new PointerEvent("pointerdown", {
        clientX: xForLevel(-20, SIDE),
        bubbles: true,
        pointerId: 1,
      }),
    );
    flushSync();
    await nextFrame();
    await nextFrame();
    // `rackSlotDto`'s value text for the threshold parameter.
    expect(texts()).toContain("-20");
    teardown();
  });

  it("keeps save/restore balanced and retries when a frame's draw throws", async () => {
    let thrown = false;
    const { ctx, teardown } = await draw(curveDto(), () => {
      if (!thrown) {
        thrown = true;
        throw new Error("transient");
      }
    });
    expect(thrown).toBe(true);
    // The scheduler retries a thrown draw on a later frame (H-43).
    await nextFrame();
    await nextFrame();
    expect(ctx.ops.filter((op) => op.name === "clearRect").length).toBeGreaterThanOrEqual(2);
    expect(ctx.ops.filter((op) => op.name === "save").length).toBe(
      ctx.ops.filter((op) => op.name === "restore").length,
    );
    // The retry completed a full draw: the curve is on screen.
    expect(hasVertex(vertices(ctx.ops, false), xForLevel(0, SIDE), yForLevel(-6, SIDE))).toBe(true);
    teardown();
  });
});
