import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { CurveHandleDto, LocalizedTextDto, ParamInfoDto, RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
import { loadRack, resetRackForTest } from "../rack/rack.svelte";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import { eqExpandedState, openEqExpanded, resetEqExpandedForTest } from "./eqExpanded.svelte";
import EqExpandedView from "./EqExpandedView.svelte";

/**
 * The EQ graph's expanded view (H-84, SPEC-015 §2.6.1): opens for the store's `openUid`, mirrors
 * `SpectrumInspector`'s floating-window mechanics (non-modal dialog, header drag, corner resize,
 * Esc/close-button close), and closes itself if the slot it was showing disappears.
 */

const EMPTY_CURVE: ResponseCurveDto = { freqs_hz: [], sample_rate_hz: 48_000, total_db: [], components_db: [] };

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

function eqSlot(): RackSlotDto {
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

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

async function seedRack(slots: RackSlotDto[]): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "rack_get" || cmd === "rack_list_modules") {
      return cmd === "rack_list_modules" ? [] : rackStateDto(slots);
    }
    if (cmd === "rack_response_curve") {
      return EMPTY_CURVE;
    }
    if (cmd === "analyzer_subscribe") {
      return 1;
    }
    return { slots: [], ab: false, latency_samples: 0 };
  });
  await loadRack();
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  resetEqExpandedForTest();
  document.body.innerHTML = "";
});

describe("EqExpandedView", () => {
  it("renders nothing when no slot is expanded", async () => {
    await seedRack([eqSlot()]);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    expect(target.querySelector('[data-testid="eq-expanded-view"]')).toBeNull();
    unmount(app);
  });

  it("opens for the store's uid, titled with the slot's name, showing the larger graph", async () => {
    await seedRack([eqSlot()]);
    openEqExpanded(7);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    await settle();

    const win = target.querySelector('[data-testid="eq-expanded-view"]');
    expect(win).not.toBeNull();
    expect(target.querySelector("#eq-expanded-title")?.textContent).toContain("Parametric EQ");
    expect(target.querySelector('[data-testid="eq-graph"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="eq-canvas"]')).not.toBeNull();
    // SPEC-015 §2.6.1: the expanded view has no Expand button of its own.
    expect(target.querySelector('[data-testid="eq-expand-button"]')).toBeNull();
    unmount(app);
  });

  it("closes on its close button", async () => {
    await seedRack([eqSlot()]);
    openEqExpanded(7);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="eq-expanded-close"]')!.click();
    flushSync();
    expect(eqExpandedState().openUid).toBeNull();
    expect(target.querySelector('[data-testid="eq-expanded-view"]')).toBeNull();
    unmount(app);
  });

  it("closes on Escape", async () => {
    await seedRack([eqSlot()]);
    openEqExpanded(7);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    await settle();

    const win = target.querySelector('[data-testid="eq-expanded-view"]')!;
    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    expect(eqExpandedState().openUid).toBeNull();
    unmount(app);
  });

  it("is not modal (aria-modal=false)", async () => {
    await seedRack([eqSlot()]);
    openEqExpanded(7);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    await settle();
    expect(target.querySelector('[data-testid="eq-expanded-view"]')?.getAttribute("aria-modal")).toBe("false");
    unmount(app);
  });

  it("closes itself if the slot it was showing disappears", async () => {
    await seedRack([eqSlot()]);
    openEqExpanded(7);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EqExpandedView, { target });
    flushSync();
    await settle();
    expect(target.querySelector('[data-testid="eq-expanded-view"]')).not.toBeNull();

    resetRackForTest(); // the rack goes empty — slot uid 7 no longer exists
    flushSync();
    await settle();
    expect(eqExpandedState().openUid).toBeNull();
    expect(target.querySelector('[data-testid="eq-expanded-view"]')).toBeNull();
    unmount(app);
  });
});
