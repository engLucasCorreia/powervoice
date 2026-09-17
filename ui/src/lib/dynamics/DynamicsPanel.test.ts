import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  LocalizedTextDto,
  ParamGroupDto,
  ParamInfoDto,
  RackSlotDto,
  TelemetryChannelDto,
} from "../ipc/bindings";
import { onModuleTelemetry, resetRackForTest, TELEMETRY_STALE_MS } from "../rack/rack.svelte";
import RackSlot from "../rack/RackSlot.svelte";
import { frameScheduler } from "../render/frameScheduler";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import { encodeVxtc } from "../test/vxtc";

/**
 * The custom Dynamics panel (H-77): SPEC-016 AC-20 (structure) and AC-22 (meters, lamp, stale
 * frames, and the generic fallback with the Noise Gate's schema). Mounts the real `RackSlot`, so
 * what is checked is what the rack renders.
 */

const RATE_HZ = 48_000;

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

const FLAGS: ParamInfoDto["flags"] = {
  automatable: true,
  stepped: false,
  boolean: false,
  read_only: false,
  hidden: false,
  bypass: false,
};

function param(
  id: number,
  key: string,
  group: number | null,
  value: number,
  overrides: Partial<ParamInfoDto> = {},
): ParamInfoDto {
  return {
    id,
    key,
    name: text(key),
    group,
    unit: { kind: "db" },
    min: -80,
    max: 0,
    default: value,
    taper: { kind: "db", neg_inf_at_min: false },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 20,
    flags: FLAGS,
    ...overrides,
  };
}

function toggle(id: number, key: string, group: number): ParamInfoDto {
  return param(id, key, group, 0, {
    unit: { kind: "none" },
    min: 0,
    max: 1,
    taper: { kind: "linear" },
    flags: { ...FLAGS, boolean: true },
  });
}

function group(
  id: number,
  key: string,
  enable: number,
  collapsed_by_default: boolean,
): ParamGroupDto {
  return { id, key, name: text(key), parent: null, enable_param: enable, collapsed_by_default };
}

function gr(id: number, key: string, group: number | null): TelemetryChannelDto {
  return {
    id,
    key,
    name: text(key),
    unit: { kind: "db" },
    min: -60,
    max: 0,
    kind: "gain_reduction",
    group,
  };
}

/** SPEC-016 §3: the global row, the four sections in processing order, and the seven channels. */
function dynamicsSlot(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  const enables: Record<number, number> = { 10: 1, 20: 0, 30: 1, 40: 0 };
  const params: ParamInfoDto[] = [
    param(2, "knee_db", null, 6, { unit: { kind: "db" }, min: 0, max: 20 }),
    param(3, "lookahead_ms", null, 5, { unit: { kind: "ms" }, min: 0, max: 20 }),
    toggle(10, "autogate_enabled", 1),
    param(11, "autogate_threshold_db", 1, -50),
    toggle(20, "expander_enabled", 2),
    param(21, "expander_threshold_db", 2, -45),
    toggle(30, "compressor_enabled", 3),
    param(31, "compressor_threshold_db", 3, -20),
    param(35, "compressor_makeup_db", 3, 4, { unit: { kind: "db" }, min: 0, max: 30 }),
    toggle(40, "limiter_enabled", 4),
    param(41, "limiter_threshold_db", 4, -1),
  ];
  return rackSlotDto({
    uid: 7,
    module: "org.powervoice.dynamics@1.0.0",
    module_id: "org.powervoice.dynamics",
    name: "Dynamics",
    latency_samples: 240,
    params,
    groups: [
      group(1, "autogate", 10, true),
      group(2, "expander", 20, true),
      group(3, "compressor", 30, false),
      group(4, "limiter", 40, true),
    ],
    values: params.map((p) => ({
      id: p.id,
      value: enables[p.id] ?? p.default,
      normalized: 0.5,
      text: String(enables[p.id] ?? p.default),
    })),
    telemetry: [
      gr(0, "gr_total_db", null),
      gr(1, "gr_autogate_db", 1),
      gr(2, "gr_expander_db", 2),
      gr(3, "gr_compressor_db", 3),
      gr(4, "gr_limiter_db", 4),
      {
        id: 5,
        key: "input_level_dbfs",
        name: text("input"),
        unit: { kind: "dbfs" },
        min: -100,
        max: 6,
        kind: "level",
        group: null,
      },
      {
        id: 6,
        key: "autogate_open",
        name: text("gate open"),
        unit: { kind: "none" },
        min: 0,
        max: 1,
        kind: "indicator",
        group: 1,
      },
    ],
    transfer_handles: [
      { component: 0, threshold: 11, enable: 10 },
      { component: 1, threshold: 21, enable: 20 },
      { component: 2, threshold: 31, enable: 30 },
      { component: 3, threshold: 41, enable: 40 },
    ],
    ...overrides,
  });
}

/** SPEC-013 §3: the generic fallback's fixture — a lamp and a GR meter in the slot header, a
 * level bar in the Sidechain group's header, and the graph above the parameters. */
function noiseGateSlot(): RackSlotDto {
  const params: ParamInfoDto[] = [
    param(1, "threshold_db", null, -40),
    param(7, "sc_hpf_enabled", 1, 1, {
      unit: { kind: "none" },
      min: 0,
      max: 1,
      flags: { ...FLAGS, boolean: true },
    }),
    param(8, "sc_hpf_hz", 1, 100, { unit: { kind: "hz" }, min: 20, max: 2000 }),
  ];
  return rackSlotDto({
    uid: 9,
    module: "org.powervoice.noise-gate@1.0.0",
    module_id: "org.powervoice.noise-gate",
    name: "Noise gate",
    params,
    groups: [group(1, "sidechain", 7, false)],
    values: params.map((p) => ({
      id: p.id,
      value: p.default,
      normalized: 0.5,
      text: String(p.default),
    })),
    telemetry: [
      {
        id: 0,
        key: "gate_open",
        name: text("gate open"),
        unit: { kind: "none" },
        min: 0,
        max: 1,
        kind: "indicator",
        group: null,
      },
      gr(1, "gain_db", null),
      {
        id: 2,
        key: "sidechain_level_dbfs",
        name: text("sidechain level"),
        unit: { kind: "dbfs" },
        min: -100,
        max: 6,
        kind: "level",
        group: 1,
      },
    ],
    transfer_handles: [{ component: 0, threshold: 1, enable: null }],
  });
}

/** One `VXMT` frame for `uid` (the layout is pinned by `vxmt_fixture.ts`). */
function vxmt(uid: number, values: number[]): ArrayBuffer {
  const buf = new ArrayBuffer(32 + 8 + 4 * values.length);
  const dv = new DataView(buf);
  for (const [i, ch] of [..."VXMT"].entries()) {
    dv.setUint8(i, ch.charCodeAt(0));
  }
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 32, true);
  dv.setUint32(8, 1, true);
  dv.setUint32(24, 1, true);
  dv.setUint32(32, uid, true);
  dv.setUint16(36, values.length, true);
  values.forEach((v, i) => dv.setFloat32(40 + 4 * i, v, true));
  return buf;
}

let widthDescriptor: PropertyDescriptor | undefined;

function setupIpc(): Array<{ cmd: string; args: unknown }> {
  const calls: Array<{ cmd: string; args: unknown }> = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "module_transfer_curve") {
      return encodeVxtc({
        xMinDb: -80,
        xMaxDb: 6,
        rising: [-80, -40, -20, -6],
        handles: [{ param: 31, xDbfs: -20 }],
      });
    }
    return rackStateDto([]);
  });
  return calls;
}

function render(slot: RackSlotDto) {
  widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get: () => 260,
  });
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackSlot, {
    target,
    props: {
      slot,
      index: 0,
      rateHz: RATE_HZ,
      dragOver: false,
      ondragstart: () => {},
      ondragover: () => {},
      ondrop: () => {},
      ondragend: () => {},
    },
  });
  flushSync();
  return { target, teardown: () => unmount(app) };
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
  vi.useRealTimers();
});

describe("Dynamics panel structure (SPEC-016 AC-20)", () => {
  it("shows the global row, then the graph, then the sections in processing order", () => {
    setupIpc();
    const { target, teardown } = render(dynamicsSlot());
    const body = target.querySelector<HTMLElement>(".body")!;
    const order = [...body.querySelectorAll<HTMLElement>("[data-testid]")]
      .map((el) => el.dataset.testid)
      .filter(
        (id) => id === "dynamics-global" || id === "transfer-graph" || id === "param-group",
      );
    expect(order[0]).toBe("dynamics-global");
    expect(order[1]).toBe("transfer-graph");
    expect(order.slice(2)).toEqual(["param-group", "param-group", "param-group", "param-group"]);
    expect([...body.querySelectorAll<HTMLElement>("[data-testid=param-group]")].map((el) => el.dataset.key)).toEqual([
      "autogate",
      "expander",
      "compressor",
      "limiter",
    ]);
    teardown();
  });

  it("binds each header toggle to its section's enable parameter", async () => {
    const calls = setupIpc();
    const { target, teardown } = render(dynamicsSlot());
    const groups = [...target.querySelectorAll<HTMLElement>("[data-testid=param-group]")];
    const boxes = groups.map(
      (g) => g.querySelector<HTMLInputElement>("[data-testid=param-group-enable]")!,
    );
    expect(boxes.map((b) => b.checked)).toEqual([true, false, true, false]);

    boxes[2]!.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    const sent = calls.filter((c) => c.cmd === "param_set_text").map((c) => c.args);
    expect(sent).toEqual([{ slot: 0, id: 30, text: "0" }]);
    teardown();
  });

  it("honours collapsed_by_default and keeps a disabled section dimmed but editable", () => {
    setupIpc();
    // The Compressor is the one section expanded by default (SPEC-016 §3); here it is switched
    // off, so its body is the dimmed-but-editable case.
    const slot = dynamicsSlot();
    const { target, teardown } = render({
      ...slot,
      values: slot.values.map((v) => (v.id === 30 ? { ...v, value: 0, text: "0" } : v)),
    });
    const groups = [...target.querySelectorAll<HTMLElement>("[data-testid=param-group]")];
    // AutoGate, Expander and Limiter are collapsed by default: no body at all.
    expect(groups[0]!.querySelector(".body")).toBeNull();
    expect(groups[1]!.querySelector(".body")).toBeNull();
    expect(groups[3]!.querySelector(".body")).toBeNull();
    const compressorBody = groups[2]!.querySelector<HTMLElement>(".body")!;
    expect(compressorBody.classList.contains("dimmed")).toBe(true);
    expect([...compressorBody.querySelectorAll("input")].every((i) => !i.disabled)).toBe(true);
    teardown();
  });

  it("shows the look-ahead's latency only while the slot reports one", () => {
    setupIpc();
    const withLatency = render(dynamicsSlot());
    expect(
      withLatency.target.querySelector('[data-testid="dynamics-latency"]')?.textContent?.trim(),
    ).toBe("adds 5.0 ms latency");
    withLatency.teardown();

    const none = render(dynamicsSlot({ latency_samples: 0 }));
    expect(none.target.querySelector('[data-testid="dynamics-latency"]')).toBeNull();
    none.teardown();
  });
});

describe("meters, lamp and the generic fallback (SPEC-016 AC-22)", () => {
  function sectionMeter(target: HTMLElement, index: number) {
    const groups = [...target.querySelectorAll<HTMLElement>("[data-testid=param-group]")];
    const meter = groups[index]!.querySelector<HTMLElement>("[data-testid=rack-slot-gr-meter]")!;
    return {
      fill: meter.querySelector<HTMLElement>("[data-testid=rack-slot-gr-fill]")!.style.width,
      text: meter.querySelector("[data-testid=rack-slot-gr-value]")!.textContent,
    };
  }

  it("puts a frame's section values on the section meters, pinning past the scale", () => {
    setupIpc();
    const { target, teardown } = render(dynamicsSlot());
    // channels: total, autogate, expander, compressor, limiter, level, lamp
    onModuleTelemetry(vxmt(7, [-2, 0, 0, -6, 0, -18, 1]));
    flushSync();
    // −6 dB on the 0 … −30 dB scale is a fifth of the track.
    expect(sectionMeter(target, 2)).toEqual({ fill: "20%", text: "−6.0 dB" });
    // The slot header carries the total.
    expect(
      target.querySelector("header [data-testid=rack-slot-gr-value]")?.textContent,
    ).toBe("−2.0 dB");

    onModuleTelemetry(vxmt(7, [-2, 0, 0, -45, 0, -18, 1]));
    flushSync();
    expect(sectionMeter(target, 2)).toEqual({ fill: "100%", text: "−45.0 dB" });
    teardown();
  });

  it("follows the AutoGate lamp channel, and drops every meter and lamp after 250 ms", () => {
    vi.useFakeTimers();
    setupIpc();
    const { target, teardown } = render(dynamicsSlot());
    const lamp = () =>
      target.querySelector<HTMLElement>("[data-testid=rack-slot-lamp]")!.dataset.lit;

    onModuleTelemetry(vxmt(7, [-2, -3, 0, -6, 0, -18, 1]));
    flushSync();
    expect(lamp()).toBe("true");
    expect(sectionMeter(target, 0).text).toBe("−3.0 dB");

    onModuleTelemetry(vxmt(7, [-2, -3, 0, -6, 0, -18, 0]));
    flushSync();
    expect(lamp()).toBe("false");

    // No frame for 250 ms: everything falls back to rest (SPEC-016 §2.6 "Stale").
    vi.advanceTimersByTime(TELEMETRY_STALE_MS + 1);
    flushSync();
    expect(lamp()).toBe("false");
    expect(sectionMeter(target, 0)).toEqual({ fill: "0%", text: "0.0 dB" });
    expect(sectionMeter(target, 2)).toEqual({ fill: "0%", text: "0.0 dB" });
    teardown();
  });

  it("gives a module with the same extensions the generic panel (SPEC-013 §2.7)", () => {
    setupIpc();
    const { target, teardown } = render(noiseGateSlot());
    // No custom panel: the graph comes first, above the parameters.
    expect(target.querySelector('[data-testid="dynamics-panel"]')).toBeNull();
    const body = target.querySelector<HTMLElement>(".body")!;
    const order = [...body.querySelectorAll<HTMLElement>("[data-testid]")]
      .map((el) => el.dataset.testid)
      .filter((id) => id === "transfer-graph" || id === "param-group");
    expect(order).toEqual(["transfer-graph", "param-group"]);

    // The slot header carries the lamp and the GR meter; the Sidechain group header the level bar.
    const header = target.querySelector<HTMLElement>("header")!;
    expect(header.querySelector("[data-testid=rack-slot-lamp]")).not.toBeNull();
    expect(header.querySelector("[data-testid=rack-slot-gr-meter]")).not.toBeNull();
    const sidechain = target.querySelector<HTMLElement>("[data-testid=param-group]")!;
    expect(sidechain.querySelector("[data-testid=rack-slot-level-meter]")).not.toBeNull();

    onModuleTelemetry(vxmt(9, [1, -12, -35]));
    flushSync();
    expect(header.querySelector<HTMLElement>("[data-testid=rack-slot-lamp]")!.dataset.lit).toBe(
      "true",
    );
    expect(header.querySelector("[data-testid=rack-slot-gr-value]")?.textContent).toBe(
      "−12.0 dB",
    );
    expect(sidechain.querySelector("[data-testid=rack-slot-level-value]")?.textContent).toBe(
      "−35.0 dBFS",
    );
    teardown();
  });
});
