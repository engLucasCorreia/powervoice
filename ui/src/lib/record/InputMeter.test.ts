import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { InputMeterFloorPref, Settings } from "../ipc/bindings";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { clearActionHandlers } from "../shortcuts";
import { applyRecordStateForTest, onInputTelemetry, resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { settingsFixture } from "../test/fixtures";
import { formatNumber } from "../ui/units";
import InputMeter from "./InputMeter.svelte";

/**
 * H-112 (owner request): "The input level should be the same way and shape [as the output
 * level]... with options to change the mic scale's minimum to −60, −80 or −120". These tests
 * cover the component-level wiring: the shared vertical meter form, the selectable/persisted
 * floor, the Peak/RMS readouts (new) and the Max readout with its click-to-reset (kept from the
 * pre-H-112 horizontal meter). The ballistics/scale maths itself is tested in
 * `meters/ballistics.test.ts` and `meters/meterScale.test.ts`.
 */

let mounted: ReturnType<typeof mount> | null = null;
let settingsSaved: Settings | undefined;

afterEach(() => {
  if (mounted) {
    unmount(mounted);
    mounted = null;
  }
  document.body.innerHTML = "";
  clearMocks();
  clearActionHandlers();
  resetRecordForTest();
  resetSettingsStateForTest();
  settingsSaved = undefined;
});

async function setUp(floor: InputMeterFloorPref = "-60"): Promise<void> {
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ input_meter_floor: floor });
    }
    if (cmd === "settings_set") {
      settingsSaved = (args as { settings: Settings }).settings;
      return settingsSaved;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await loadSettings();
}

function render(): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  mounted = mount(InputMeter, { target });
  flushSync();
  return target;
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

function frame(overrides: Partial<TelemetryFrame> = {}): TelemetryFrame {
  return {
    seq: 0,
    flags: 0,
    playheadSample: 0,
    playheadTimeNs: 0,
    rate: 0,
    outPeakDbfs: Number.NEGATIVE_INFINITY,
    outRmsDbfs: Number.NEGATIVE_INFINITY,
    inPeakDbfs: Number.NEGATIVE_INFINITY,
    inRmsDbfs: Number.NEGATIVE_INFINITY,
    audioRev: 0,
    droppedRtEvents: 0,
    ...overrides,
  };
}

describe("InputMeter visibility (unchanged: SPEC-002 §2.1, visible while armed)", () => {
  it("renders nothing while disarmed", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: false });
    const root = render();
    expect(root.querySelector('[data-testid="input-meter-wrap"]')).toBeNull();
  });

  it("renders while armed", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[data-testid="input-meter-wrap"]')).not.toBeNull();
  });
});

describe("InputMeter form (H-112 item 1: same vertical form as the output meter)", () => {
  it("has an accessible vertical role=meter with peak/rms fills and a hold tick", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[role="meter"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="input-meter-peak-fill"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="input-meter-rms-fill"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="input-meter-hold"]')).not.toBeNull();
  });

  it("shows the same readable numeric Peak/RMS readouts as the output meter", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    render();
    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
    flushSync();
    expect(document.querySelector('[data-testid="input-meter-peak"]')?.textContent).toBe("Peak −6.0 dBFS");
    expect(document.querySelector('[data-testid="input-meter-rms"]')?.textContent).toBe("RMS −9.0 dBFS");
  });

  it("tints the hold tick on a latched clip, like the output meter's own hold tick", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    const root = render();
    const hold = root.querySelector('[data-testid="input-meter-hold"]')!;
    expect(hold.classList.contains("clip")).toBe(false);

    onInputTelemetry(frame({ flags: VXTM_FLAGS.IN_CLIP, inPeakDbfs: 0 }), 0);
    flushSync();
    expect(hold.classList.contains("clip")).toBe(true);
  });

  it("has no clip-lamp button of its own (the transport bar already has one)", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[data-testid="input-meter-clip"]')).toBeNull();
  });
});

describe("InputMeter max readout (H-112 item 3: kept, output has no equivalent)", () => {
  it("shows the highest peak and resets it on click", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    const root = render();

    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
    flushSync();
    const maxButton = root.querySelector<HTMLButtonElement>('[data-testid="input-meter-max"]')!;
    expect(maxButton.textContent).toBe("Max −6.0 dBFS");

    maxButton.click();
    flushSync();
    expect(maxButton.textContent).toBe("Max −∞ dBFS");
  });
});

describe("InputMeter selectable floor (H-112 item 2)", () => {
  it("defaults to the SPEC-002 §2.1 factory floor, -60 dBFS", async () => {
    await setUp("-60");
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[role="meter"]')?.getAttribute("aria-valuemin")).toBe("-60");
    expect(root.querySelector<HTMLSelectElement>('[data-testid="input-meter-floor"]')?.value).toBe("-60");
  });

  it("reads a persisted -80 dBFS floor from settings", async () => {
    await setUp("-80");
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[role="meter"]')?.getAttribute("aria-valuemin")).toBe("-80");
    expect(root.querySelector<HTMLSelectElement>('[data-testid="input-meter-floor"]')?.value).toBe("-80");
  });

  it("reads a persisted -120 dBFS floor from settings", async () => {
    await setUp("-120");
    applyRecordStateForTest({ input_open: true });
    const root = render();
    expect(root.querySelector('[role="meter"]')?.getAttribute("aria-valuemin")).toBe("-120");
  });

  it("saves the chosen floor as a setting (settings_set) when changed", async () => {
    await setUp("-60");
    applyRecordStateForTest({ input_open: true });
    const root = render();
    const select = root.querySelector<HTMLSelectElement>('[data-testid="input-meter-floor"]')!;

    select.value = "-120";
    // Svelte 5 delegates `change` to the root: the event must bubble (record.test.ts convention).
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();

    expect(settingsSaved?.input_meter_floor).toBe("-120");
  });

  it("the selected floor reaches the shared scale as a labelled, non-colliding tick", async () => {
    // jsdom has no ResizeObserver (see OutputMeter.test.ts's identical stand-in) — install one so
    // `trackHeightPx` (and therefore the scale ticks) isn't stuck at 0.
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
    const original = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
    try {
      for (const floor of ["-60", "-80", "-120"] as const) {
        await setUp(floor);
        applyRecordStateForTest({ input_open: true });
        const root = render();
        const ro = FakeResizeObserver.instances.at(-1);
        expect(ro).toBeDefined();
        ro!.trigger(400);
        flushSync();

        const ticks = [...root.querySelectorAll<HTMLElement>(".tick")];
        // The label uses the real minus sign (U+2212, via `formatNumber`), not an ASCII hyphen.
        expect(ticks.some((el) => el.textContent === formatNumber(Number(floor), 0))).toBe(true);
        expect(ticks.some((el) => el.textContent === "−∞")).toBe(true);
        const tops = ticks.map((el) => el.style.top);
        expect(new Set(tops).size).toBe(tops.length); // no two ticks share a `top`

        unmount(mounted!);
        mounted = null;
        document.body.innerHTML = "";
        resetRecordForTest();
        resetSettingsStateForTest();
        clearMocks();
        FakeResizeObserver.instances.length = 0;
      }
    } finally {
      (globalThis as { ResizeObserver?: unknown }).ResizeObserver = original;
    }
  });
});
