import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type {
  DeviceStatusDto,
  DevicesDto,
  NoiseProfileCurveDto,
  NoiseProfileStatusDto,
  RackSlotDto,
} from "../ipc/bindings";
import { resetOutputDeviceStatusForTest } from "../analyzer/outputDeviceStatus.svelte";
import { uForFreq } from "../spectrum/freqAxis";
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
 *
 * H-87 adds the hover readout and the "no output device" state — see the two new `describe`
 * blocks below. `NoiseProfileGraph` mounts `initOutputDeviceStatus()` unconditionally now, so
 * every `mockIPC` handler here answers `devices_list` (defaulting to "healthy", i.e. no effect on
 * the pre-existing tests); a handler that doesn't would just have the fetch fail and caught
 * internally (`outputDeviceStatus.svelte.ts`), but answering it keeps every test's device state
 * explicit.
 */

function devicesDto(outputStatus: DeviceStatusDto = "healthy"): DevicesDto {
  return {
    hosts: ["pipewire"],
    host: "pipewire",
    devices: [],
    default_input: null,
    default_output: null,
    prefs: {
      host: "pipewire",
      input_device: null,
      input_channel: 1,
      output_device: null,
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    output_device: "Speakers",
    output_rate_hz: 48_000,
    output_buffer_frames: 256,
    output_status: outputStatus,
    input_device: null,
    input_status: "not_selected",
  };
}

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
  resetOutputDeviceStatusForTest();
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
      if (cmd === "devices_list") {
        return devicesDto();
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
      if (cmd === "devices_list") {
        return devicesDto();
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
      if (cmd === "devices_list") {
        return devicesDto();
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
      if (cmd === "devices_list") {
        return devicesDto();
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
      if (cmd === "devices_list") {
        return devicesDto();
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

/** The graph's private dB-label gutter width (`DB_LABEL_GUTTER_PX` in the component) — duplicated
 * here the way `SpectrumPlot.test.ts` duplicates its own axis geometry to compute a pointer
 * position, rather than exporting a layout constant no other caller needs. */
const GUTTER_PX = 34;

function xForFreqHz(freqHz: number): number {
  return GUTTER_PX + uForFreq(freqHz, 20, 24_000, "log") * (WIDTH - GUTTER_PX);
}

describe("hover readout (H-87, SPEC-014 §2.8: frequency, print level, live level)", () => {
  function mockCurveAndDevice(): void {
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        return curveFixture();
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      if (cmd === "devices_list") {
        return devicesDto();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
  }

  it("shows the frequency and the print level under the cursor, against a known print", async () => {
    mockCurveAndDevice();
    const { target, teardown } = render("loaded");
    await afterFrame();

    const canvas = target.querySelector("canvas")!;
    canvas.dispatchEvent(
      new MouseEvent("mousemove", { clientX: xForFreqHz(1_000), clientY: 40, bubbles: true }),
    );
    flushSync();

    const hover = target.querySelector('[data-testid="nr-profile-hover"]');
    expect(hover).not.toBeNull();
    // curveFixture(): 1 000 Hz -> -55 dBFS.
    expect(hover!.textContent).toContain("Print −55.0 dB");
    // No live analyzer frame arrived in this test: the live reading stays silent.
    expect(hover!.textContent).toContain("Live −∞ dB");
    expect(hover!.textContent).toMatch(/Hz/);
    teardown();
  });

  it("clears on mouseleave", async () => {
    mockCurveAndDevice();
    const { target, teardown } = render("loaded");
    await afterFrame();

    const canvas = target.querySelector("canvas")!;
    canvas.dispatchEvent(
      new MouseEvent("mousemove", { clientX: xForFreqHz(1_000), clientY: 40, bubbles: true }),
    );
    flushSync();
    expect(target.querySelector('[data-testid="nr-profile-hover"]')).not.toBeNull();

    canvas.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="nr-profile-hover"]')).toBeNull();
    teardown();
  });

  it("is keyboard-reachable: the graph is a focusable tab stop, and focus/arrows/Escape drive the readout", async () => {
    mockCurveAndDevice();
    const { target, teardown } = render("loaded");
    await afterFrame();

    const wrap = target.querySelector<HTMLElement>('[data-testid="nr-profile-canvas-wrap"]')!;
    expect(wrap.tabIndex).toBe(0);

    wrap.dispatchEvent(new FocusEvent("focus", { bubbles: true }));
    flushSync();
    const centred = target.querySelector('[data-testid="nr-profile-hover"]')?.textContent;
    expect(centred).toBeTruthy();

    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    flushSync();
    const moved = target.querySelector('[data-testid="nr-profile-hover"]')?.textContent;
    expect(moved).not.toBe(centred);

    wrap.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="nr-profile-hover"]')).toBeNull();
    teardown();
  });
});

describe('no output device (H-87, SPEC-014 §2.8: "greyed while the analyzer has no output device")', () => {
  it.each(["not_selected", "lost"] as const)(
    "shows the analyzer panel's own honest wording when the output device is %s",
    async (outputStatus) => {
      mockIPC((cmd) => {
        if (cmd === "noise_profile_curve") {
          return curveFixture();
        }
        if (cmd === "analyzer_subscribe") {
          throw new Error("no real Tauri window");
        }
        if (cmd === "devices_list") {
          return devicesDto(outputStatus);
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, teardown } = render("loaded");
      await afterFrame();

      const note = target.querySelector('[data-testid="nr-graph-live-no-device"]');
      expect(note).not.toBeNull();
      // The exact wording the analyzer panel already uses for this fact (`analyzer.no_device`,
      // H-59) — never a new phrase for the same state.
      expect(note!.textContent).toContain("No output device");

      const liveItem = target.querySelector('[data-testid="nr-graph-legend-live"]')!;
      expect(liveItem.classList.contains("legend-item-disabled")).toBe(true);
      const liveSwatch = liveItem.querySelector(".swatch")!;
      expect(liveSwatch.classList.contains("swatch-disabled")).toBe(true);
      teardown();
    },
  );

  it("shows nothing extra once the device is healthy", async () => {
    mockIPC((cmd) => {
      if (cmd === "noise_profile_curve") {
        return curveFixture();
      }
      if (cmd === "analyzer_subscribe") {
        throw new Error("no real Tauri window");
      }
      if (cmd === "devices_list") {
        return devicesDto("healthy");
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("loaded");
    await afterFrame();

    expect(target.querySelector('[data-testid="nr-graph-live-no-device"]')).toBeNull();
    const liveItem = target.querySelector('[data-testid="nr-graph-legend-live"]')!;
    expect(liveItem.classList.contains("legend-item-disabled")).toBe(false);
    teardown();
  });
});
