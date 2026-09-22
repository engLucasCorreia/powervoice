import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InputMeterFloorPref, MeterSpeedPref, Settings } from "../ipc/bindings";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { METER_SPEED_PROFILES } from "../meters/ballistics";
import { frameScheduler } from "../render/frameScheduler";
import { clearActionHandlers } from "../shortcuts";
import { applyRecordStateForTest, onInputTelemetry, recordState, resetRecordForTest } from "../state/record.svelte";
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
  frameScheduler.resetForTest();
  settingsSaved = undefined;
});

async function setUp(floor: InputMeterFloorPref = "-60", speed: MeterSpeedPref = "medium"): Promise<void> {
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ input_meter_floor: floor, meter_speed: speed });
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

// H-123 (owner: "they are very leggy and slow, why is that? can i setup the speed?"): the input
// meter's ballistics now run at the persisted `Settings.meter_speed`, the same profiles the output
// meter uses (`meters/ballistics.ts`'s `METER_SPEED_PROFILES`).
describe("InputMeter speed (H-123)", () => {
  it("Medium keeps the pre-H-123 ballistics exactly (20 dB/s release)", async () => {
    await setUp("-60", "medium");
    applyRecordStateForTest({ input_open: true });
    render();
    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
    onInputTelemetry(frame({ inPeakDbfs: Number.NEGATIVE_INFINITY, inRmsDbfs: Number.NEGATIVE_INFINITY }), 500);
    expect(recordState().meter.peakDbfs).toBeCloseTo(-6 - METER_SPEED_PROFILES.medium.peakReleaseDbPerS * 0.5, 5);
  });

  it("Fast releases faster than Medium in the same time; Slow releases slower", async () => {
    async function barAfterHalfSecond(speed: MeterSpeedPref): Promise<number> {
      await setUp("-60", speed);
      applyRecordStateForTest({ input_open: true });
      render();
      onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
      onInputTelemetry(frame({ inPeakDbfs: Number.NEGATIVE_INFINITY, inRmsDbfs: Number.NEGATIVE_INFINITY }), 500);
      const bar = recordState().meter.peakDbfs;
      unmount(mounted!);
      mounted = null;
      document.body.innerHTML = "";
      resetRecordForTest();
      resetSettingsStateForTest();
      clearMocks();
      return bar;
    }
    const fastBar = await barAfterHalfSecond("fast");
    const mediumBar = await barAfterHalfSecond("medium");
    const slowBar = await barAfterHalfSecond("slow");
    expect(fastBar).toBeLessThan(mediumBar); // Fast has fallen further in the same half second
    expect(slowBar).toBeGreaterThan(mediumBar);
  });

  it("a live speed change (e.g. from Preferences) applies to the very next telemetry frame", async () => {
    await setUp("-60", "medium");
    applyRecordStateForTest({ input_open: true });
    render();
    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
    onInputTelemetry(frame({ inPeakDbfs: -40, inRmsDbfs: -40 }), 100);
    const mediumBar = recordState().meter.peakDbfs;

    // Switch to Fast without remounting (matches how the shared setting actually changes live).
    resetSettingsStateForTest();
    await setUp("-60", "fast");
    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 200);
    onInputTelemetry(frame({ inPeakDbfs: -40, inRmsDbfs: -40 }), 300);
    const fastBar = recordState().meter.peakDbfs;

    expect(fastBar).toBeLessThan(mediumBar); // Fast fell further over the identical 100 ms gap
  });
});

// H-123 (parity with H-43's output meter, `state/transport.test.ts`): before this ticket the input
// meter had no animation-frame fallback at all — its bar only ever moved on a real `VXTM` frame,
// so it would visibly stair-step at a throttled `telemetry_rate_hz` or during any gap. It now
// keeps animating at display rate between/after real frames, exactly like the output meter.
describe("InputMeter animation frames (H-123)", () => {
  const FAKE = [
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "performance",
  ] as const;

  function framesDuring(ms: number): number {
    const before = frameScheduler.stats.frames;
    vi.advanceTimersByTime(ms);
    return frameScheduler.stats.frames - before;
  }

  beforeEach(() => {
    vi.useFakeTimers({ toFake: [...FAKE] });
    frameScheduler.resetForTest();
  });

  afterEach(() => {
    frameScheduler.resetForTest();
    vi.useRealTimers();
  });

  it("keeps falling on animation frames after the last telemetry frame, then stops once silent", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    render();
    onInputTelemetry(frame({ inPeakDbfs: -6, inRmsDbfs: -9 }), 0);
    vi.advanceTimersByTime(17);
    // The engine's last real frame before it stops sending (a device drop, or disarming
    // mid-fall): the input has gone silent.
    onInputTelemetry(frame(), 17);
    expect(recordState().meter.peakDbfs).toBeGreaterThan(-60); // still falling

    // No further telemetry: the bar and hold keep falling on animation frames alone. (The input
    // meter's own "at rest" threshold is SILENT_SOURCE_DBFS, −120 — deeper than the output
    // meter's fixed −60 floor, since H-112 lets the input meter's *display* floor go to −120 too
    // — so this needs longer than transport.test.ts's equivalent 6 s for the hold tick, which only
    // starts falling after its own hold duration, to fall all the way past it.)
    expect(framesDuring(8000)).toBeGreaterThan(100);
    const settled = recordState().meter;
    expect(settled.peakDbfs).toBe(Number.NEGATIVE_INFINITY);
    expect(settled.holdDbfs).toBe(Number.NEGATIVE_INFINITY);
    expect(settled.rmsDbfs).toBe(Number.NEGATIVE_INFINITY);

    // Then an idle second costs nothing further (H-43's idle-CPU guarantee, kept for the input
    // meter too).
    expect(framesDuring(1000)).toBe(0);
    expect(recordState().meter).toBe(settled);
  });

  it("does not animate at all while nothing has ever peaked (idle-CPU: no gratuitous frames)", async () => {
    await setUp();
    applyRecordStateForTest({ input_open: true });
    render();
    onInputTelemetry(frame(), 0); // silence from the start
    expect(framesDuring(1000)).toBe(0);
  });
});
