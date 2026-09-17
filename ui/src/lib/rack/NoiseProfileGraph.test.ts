import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { NoiseProfileCurveDto, NoiseProfileStatusDto, RackSlotDto } from "../ipc/bindings";
import { resetRackForTest } from "./rack.svelte";
import { frameScheduler } from "../render/frameScheduler";
import { paramInfoDto, rackSlotDto } from "../test/fixtures";
import { box } from "../test/reactive.svelte";
import NoiseProfileGraph from "./NoiseProfileGraph.svelte";

/**
 * H-85 (SPEC-014 §2.8 item 2, AC-21 graph part): the profile graph's IPC contract (fetches
 * `noise_profile_curve` for its own slot; the `analyzer_subscribe` attempt is best-effort and
 * never throws without a real Tauri window) and its draw pass, at the level
 * `EqGraph.redraw.test.ts`/`TransferGraph.draw.test.ts` cover the other two rack graphs. The
 * curve's own geometry (auto-fit dB range, the "reduced to" line) is unit-tested directly in
 * `noiseProfilePlot.test.ts`; this file only checks the component wires it up.
 */

const WIDTH = 280;

function curveFixture(): NoiseProfileCurveDto {
  // A handful of SPEC-007 band centres (`20·2^(k/24)`), not the full 246 — the component doesn't
  // care how many points describe() returned.
  return {
    freqs_hz: [100, 1_000, 10_000],
    levels_dbfs: [-40, -55, -70],
  };
}

function slotFixture(status: NoiseProfileStatusDto): RackSlotDto {
  const params = [
    paramInfoDto({ id: 0, key: "reduction_db", unit: { kind: "db" }, min: 0, max: 40, default: 12 }),
    paramInfoDto({ id: 1, key: "amount_pct", unit: { kind: "percent" }, min: 0, max: 100, default: 100 }),
  ];
  return rackSlotDto({
    module: "org.powervoice.noise-reduction@1.0.0",
    module_id: "org.powervoice.noise-reduction",
    name: "Noise Reduction",
    params,
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    noise_profile: status,
  });
}

/** A minimal 2D-context stand-in: every method is a no-op spy, every property settable —
 * `EqGraph.redraw.test.ts`'s `getContext`-stubbing pattern. */
function fakeCtx(): CanvasRenderingContext2D {
  const target: Record<string, unknown> = {};
  const handler: ProxyHandler<Record<string, unknown>> = {
    get(obj, prop) {
      if (prop in obj) {
        return obj[prop as string];
      }
      const fn = (): unknown => undefined;
      obj[prop as string] = fn;
      return fn;
    },
    set(obj, prop, value) {
      obj[prop as string] = value;
      return true;
    },
  };
  return new Proxy(target, handler) as unknown as CanvasRenderingContext2D;
}

let clientWidthDescriptor: PropertyDescriptor | undefined;
let getContextDescriptor: PropertyDescriptor | undefined;

function stubClientWidth(px: number): void {
  clientWidthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
}

function stubGetContext(): void {
  const ctx = fakeCtx();
  getContextDescriptor = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "getContext");
  Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ctx });
}

beforeEach(() => {
  frameScheduler.resetForTest();
  stubClientWidth(WIDTH);
  stubGetContext();
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
});

function render(status: NoiseProfileStatusDto, slotIndex = 0) {
  const slot = slotFixture(status);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(NoiseProfileGraph, { target, props: { slotIndex, rackSlot: slot, status } });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

async function afterFrame(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 50));
  await settle();
}

describe("curve fetch (SPEC-014 §2.8 item 2)", () => {
  it("requests noise_profile_curve for this slot", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "noise_profile_curve") {
        calls.push(args);
        return { freqs_hz: [], levels_dbfs: [] } satisfies NoiseProfileCurveDto;
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { teardown } = render("none", 2);
    await afterFrame();
    expect(calls).toEqual([{ slot: 2 }]);
    teardown();
  });

  it("re-fetches when the status transitions (a capture or Clear Noise Print)", async () => {
    let calls = 0;
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        calls++;
        return curveFixture();
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      throw new Error(`unmocked command`);
    });
    const slot = slotFixture("none");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const status = box<NoiseProfileStatusDto>("none");
    const app = mount(NoiseProfileGraph, {
      target,
      props: {
        slotIndex: 0,
        rackSlot: slot,
        get status() {
          return status.value;
        },
      },
    });
    flushSync();
    await afterFrame();
    expect(calls).toBe(1);

    // Clear Noise Print / a new capture: the caller passes the new status down as a prop.
    status.value = "loaded";
    flushSync();
    await afterFrame();
    expect(calls).toBe(2);
    unmount(app);
  });
});

describe("draw (AC-21: print, reduced-to line and analyzer curve on the log axis)", () => {
  it("draws without throwing with no print (empty graph)", async () => {
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        return { freqs_hz: [], levels_dbfs: [] } satisfies NoiseProfileCurveDto;
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("none");
    await afterFrame();
    expect(target.querySelector('[data-testid="nr-profile-canvas"]')).not.toBeNull();
    teardown();
  });

  it("draws the print, reduced-to line and (once subscribed) the live curve without throwing", async () => {
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        return curveFixture();
      }
      if (cmd === "analyzer_subscribe") {
        return 1;
      }
      if (cmd === "analyzer_unsubscribe") {
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("loaded");
    await afterFrame();
    await afterFrame();
    expect(target.querySelector('[data-testid="nr-profile-canvas"]')).not.toBeNull();
    teardown();
  });
});

describe("legend", () => {
  it("labels the three curves, including \"Output (rack)\" for the live spectrum (SPEC-014 §2.8)", () => {
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        return { freqs_hz: [], levels_dbfs: [] } satisfies NoiseProfileCurveDto;
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("none");
    expect(target.querySelector('[data-testid="nr-graph-legend-print"]')?.textContent).toContain(
      "Noise print",
    );
    expect(target.querySelector('[data-testid="nr-graph-legend-reduced"]')?.textContent).toContain(
      "Reduced to",
    );
    expect(target.querySelector('[data-testid="nr-graph-legend-live"]')?.textContent).toContain(
      "Output (rack)",
    );
    teardown();
  });
});
