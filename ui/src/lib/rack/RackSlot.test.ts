import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type {
  LocalizedTextDto,
  ParamGroupDto,
  ParamInfoDto,
  RackSlotDto,
  TelemetryChannelDto,
} from "../ipc/bindings";
import { VXMT_FIXTURE_HEX } from "../ipc/vxmt_fixture";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import { onModuleTelemetry, resetRackForTest } from "./rack.svelte";
import RackSlot from "./RackSlot.svelte";

/**
 * Rack slot layout tests (SPEC-012 §2.6 AC-12, essential subset): a 40-parameter schema fixture
 * covering every flag/taper/unit/enum/group/nested-group/enable-param combination the built-ins
 * use. Per-widget interaction detail (toggle/enum/slider/text-entry) is in `ParamControl.test.ts`;
 * this file checks the slot omits hidden/bypass parameters, follows declaration order, flattens a
 * nested group as "Parent / Child", and dims a disabled group's body.
 */

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function flags(overrides: Partial<ParamInfoDto["flags"]> = {}): ParamInfoDto["flags"] {
  return {
    automatable: true,
    stepped: false,
    boolean: false,
    read_only: false,
    hidden: false,
    bypass: false,
    ...overrides,
  };
}

let nextId = 0;
function p(key: string, overrides: Partial<ParamInfoDto> = {}): ParamInfoDto {
  const id = nextId++;
  return {
    id,
    key,
    name: text(key),
    group: null,
    unit: { kind: "none" },
    min: 0,
    max: 1,
    default: 0,
    taper: { kind: "linear" },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 0,
    flags: flags(),
    ...overrides,
  };
}

function group(id: number, key: string, overrides: Partial<ParamGroupDto> = {}): ParamGroupDto {
  return {
    id,
    key,
    name: text(key),
    parent: null,
    enable_param: null,
    collapsed_by_default: false,
    ...overrides,
  };
}

/** A 40-parameter fixture: ungrouped, hidden, bypass, a nested group, and an enable-gated group. */
function bigFixture(): { params: ParamInfoDto[]; groups: ParamGroupDto[] } {
  nextId = 0;
  const params: ParamInfoDto[] = [];
  const groups: ParamGroupDto[] = [];

  // Two ungrouped params, declared in this order.
  params.push(p("input_gain_db", { unit: { kind: "db" } }));
  params.push(p("output_gain_db", { unit: { kind: "db" } }));

  // A hidden param and a BYPASS param: both omitted from the rendered panel (AC-12).
  params.push(p("internal_state", { flags: flags({ hidden: true }) }));
  params.push(p("bypass", { flags: flags({ boolean: true, bypass: true }) }));

  // Parent group "Filter" with a nested child "Filter/Detail" (flattened to "Filter / Detail").
  const filterGroup = group(100, "filter", { name: text("Filter") });
  groups.push(filterGroup);
  const detailGroup = group(101, "filter.detail", { name: text("Detail"), parent: filterGroup.id });
  groups.push(detailGroup);
  params.push(p("hp_freq_hz", { group: filterGroup.id, unit: { kind: "hz" }, min: 20, max: 20_000 }));
  params.push(p("hp_slope", { group: detailGroup.id, enum_labels: [text("6"), text("12")] }));

  // An enable-gated group: the enable param plus a stepped and a read-only body param.
  const enableParamId = nextId;
  const gatedGroup = group(102, "compressor", {
    name: text("Compressor"),
    enable_param: enableParamId,
  });
  groups.push(gatedGroup);
  params.push(
    p("compressor_enabled", { group: gatedGroup.id, flags: flags({ boolean: true }) }),
  );
  params.push(
    p("ratio", { group: gatedGroup.id, flags: flags({ stepped: true }), step: 1, min: 1, max: 20 }),
  );
  params.push(
    p("gain_reduction_db", {
      group: gatedGroup.id,
      unit: { kind: "db" },
      flags: flags({ read_only: true }),
    }),
  );

  // Pad up to 40 total declared parameters (extra ungrouped continuous params).
  while (params.length < 40) {
    params.push(p(`extra_${params.length}`, { unit: { kind: "db" } }));
  }

  return { params, groups };
}

function slotFixture(): RackSlotDto {
  const { params, groups } = bigFixture();
  return rackSlotDto({
    module: "org.powervoice.fixture@1.0.0",
    module_id: "org.powervoice.fixture",
    name: "Fixture",
    params,
    groups,
    values: params.map((pp) => ({ id: pp.id, value: pp.default, normalized: 0, text: String(pp.default) })),
  });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
});

function render(slot: RackSlotDto) {
  mockIPC(() => rackStateDto());
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackSlot, {
    target,
    props: {
      slot,
      index: 0,
      rateHz: 48_000,
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

describe("AC-12: generic UI from a 40-parameter schema fixture", () => {
  it("declares exactly 40 parameters in the fixture", () => {
    expect(slotFixture().params).toHaveLength(40);
  });

  it("omits hidden and bypass parameters from the rendered rows", () => {
    const slot = slotFixture();
    const { target, teardown } = render(slot);
    const keys = [...target.querySelectorAll('[data-testid="param-row"]')].map((el) =>
      el.getAttribute("data-key"),
    );
    expect(keys).not.toContain("internal_state");
    expect(keys).not.toContain("bypass");
    teardown();
  });

  it("renders ungrouped parameters first, in declaration order", () => {
    const slot = slotFixture();
    const { target, teardown } = render(slot);
    const ungroupedKeys = [...target.querySelectorAll(".ungrouped [data-testid='param-row']")].map((el) =>
      el.getAttribute("data-key"),
    );
    expect(ungroupedKeys[0]).toBe("input_gain_db");
    expect(ungroupedKeys[1]).toBe("output_gain_db");
    teardown();
  });

  it('flattens a nested group as "Parent / Child"', () => {
    const slot = slotFixture();
    const { target, teardown } = render(slot);
    const titles = [...target.querySelectorAll('[data-testid="param-group"] .title')].map((el) =>
      el.textContent?.replace(/\s+/g, " ").trim(),
    );
    expect(titles.some((t) => t?.endsWith("Filter / Detail"))).toBe(true);
    teardown();
  });

  it("dims a group's body while its enable parameter is off", () => {
    const slot = slotFixture();
    const { target, teardown } = render(slot);
    const compressorGroup = [...target.querySelectorAll('[data-testid="param-group"]')].find((el) =>
      el.querySelector(".title")?.textContent?.includes("Compressor"),
    )!;
    expect(compressorGroup.querySelector(".body")?.classList.contains("dimmed")).toBe(true);
    teardown();
  });

  it("renders the right widget per flag: enable checkbox, stepped slider, and read-only readout", () => {
    const slot = slotFixture();
    const { target, teardown } = render(slot);
    const row = (key: string) => target.querySelector(`[data-testid="param-row"][data-key="${key}"]`)!;
    // The group's enable_param becomes its header checkbox, not a body row (SPEC-012 §2.6 layout).
    const compressorGroup = [...target.querySelectorAll('[data-testid="param-group"]')].find((el) =>
      el.querySelector(".title")?.textContent?.includes("Compressor"),
    )!;
    expect(compressorGroup.querySelector('[data-testid="param-group-enable"]')).toBeTruthy();
    expect(target.querySelector('[data-testid="param-row"][data-key="compressor_enabled"]')).toBeNull();
    expect(row("ratio").querySelector('[data-testid="param-slider"].stepped')).toBeTruthy();
    expect(row("gain_reduction_db").querySelector('[data-testid="param-readout"]')).toBeTruthy();
    teardown();
  });
});

// S3-06, SPEC-014 §2.8: the NR section only renders for a module exposing `NoiseProfile`.
describe("noise-print section (S3-06)", () => {
  it("is absent for a slot with no NoiseProfile extension", () => {
    const { target, teardown } = render(slotFixture());
    expect(target.querySelector('[data-testid="nr-capture"]')).toBeNull();
    teardown();
  });

  it("renders above the parameters for a slot that has one", () => {
    const slot: RackSlotDto = { ...slotFixture(), noise_profile: "loaded" };
    const { target, teardown } = render(slot);
    const body = target.querySelector(".body")!;
    const nrSection = body.querySelector('[data-testid="nr-capture"]');
    expect(nrSection).not.toBeNull();
    // "above the parameters": it's an earlier child of `.body` than the first param row/group.
    const firstParamNode = body.querySelector('[data-testid="param-row"], [data-testid="param-group"]');
    expect(
      nrSection!.compareDocumentPosition(firstParamNode!) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    teardown();
  });
});

// S3-07, SPEC-015 §2.1/§2.6: the EQ graph replaces nothing — it renders above the generic
// parameter body for any module exposing `ResponseCurve` (`curve_handles !== null`), which stays
// available below it.
describe("EQ graph section (S3-07)", () => {
  it("is absent for a slot with no curve_handles", () => {
    const { target, teardown } = render(slotFixture());
    expect(target.querySelector('[data-testid="eq-graph"]')).toBeNull();
    teardown();
  });

  it("renders above the generic parameter body for a slot that has curve_handles", () => {
    const slot: RackSlotDto = {
      ...slotFixture(),
      curve_handles: [{ component: 0, freq: 0, gain: null, q: null, enable: null }],
    };
    const { target, teardown } = render(slot);
    const body = target.querySelector(".body")!;
    const graph = body.querySelector('[data-testid="eq-graph"]');
    expect(graph).not.toBeNull();
    const firstParamNode = body.querySelector('[data-testid="param-row"], [data-testid="param-group"]');
    expect(
      graph!.compareDocumentPosition(firstParamNode!) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    teardown();
  });
});

// H-63, SPEC-016 §2.6 / SPEC-013 §2.7: the transfer graph renders above the generic parameter
// body for any module exposing `TransferCurve` (`transfer_handles !== null`).
describe("transfer graph section (H-63)", () => {
  it("is absent for a slot with no transfer_handles", () => {
    const { target, teardown } = render(slotFixture());
    expect(target.querySelector('[data-testid="transfer-graph"]')).toBeNull();
    teardown();
  });

  it("renders above the generic parameter body for a slot that has transfer_handles", () => {
    const slot: RackSlotDto = {
      ...slotFixture(),
      transfer_handles: [{ component: 0, threshold: 0, enable: null }],
    };
    const { target, teardown } = render(slot);
    const body = target.querySelector(".body")!;
    const graph = body.querySelector('[data-testid="transfer-graph"]');
    expect(graph).not.toBeNull();
    const firstParamNode = body.querySelector('[data-testid="param-row"], [data-testid="param-group"]');
    expect(
      graph!.compareDocumentPosition(firstParamNode!) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    teardown();
  });
});

// H-03 (SPEC-017 §2.3 "Meter"): a module telemetry channel of kind `gain_reduction` placed in the
// header (`group` null) renders as a gain-reduction meter, fed by `VXMT` frames through the store.
describe("gain-reduction meter (H-03)", () => {
  const grChannel: TelemetryChannelDto = {
    id: 0,
    key: "gain_reduction_db",
    name: text("Gain reduction"),
    unit: { kind: "db" },
    min: -24,
    max: 0,
    kind: "gain_reduction",
    group: null,
  };

  function hexToBuffer(hex: string): ArrayBuffer {
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
    }
    return bytes.buffer as ArrayBuffer;
  }

  it("is absent without a header gain-reduction channel", () => {
    for (const telemetry of [[], [{ ...grChannel, group: 1 }], [{ ...grChannel, kind: "level" as const }]]) {
      const { target, teardown } = render({ ...slotFixture(), telemetry });
      expect(target.querySelector('[data-testid="rack-slot-gr-meter"]')).toBeNull();
      teardown();
    }
  });

  it("shows the slot's value from the latest VXMT frame", () => {
    // The golden fixture carries slot uid 3 at −6.5 dB.
    const { target, teardown } = render({ ...slotFixture(), uid: 3, telemetry: [grChannel] });
    const value = () => target.querySelector('[data-testid="rack-slot-gr-value"]')?.textContent;
    expect(value()).toBe("0.0");
    onModuleTelemetry(hexToBuffer(VXMT_FIXTURE_HEX));
    flushSync();
    expect(value()).toBe("−6.5");
    expect(target.querySelector("header [role=meter]")?.getAttribute("aria-label")).toContain(
      "Gain reduction",
    );
    teardown();
  });
});

// T-802: a sandboxed plugin's status badge (Running / Restarting / Plugin failed), "Not
// installed" for an unregistered module, the failure reason, and a Retry action (= Restart).
describe("slot status and Retry (T-802)", () => {
  function badge(target: HTMLElement): string | null {
    return target.querySelector('[data-testid="rack-slot-badge"]')?.textContent?.trim() ?? null;
  }

  it("shows Running for an active sandboxed slot and nothing for an in-process one", () => {
    let r = render({ ...slotFixture(), sandboxed: true });
    expect(badge(r.target)).toBe("Running");
    expect(r.target.querySelector('[data-testid="rack-slot-retry"]')).toBeNull();
    r.teardown();
    r = render(slotFixture());
    expect(badge(r.target)).toBeNull();
    r.teardown();
  });

  it("shows Restarting with the failure reason and no Retry while a restart is pending", () => {
    const { target, teardown } = render({
      ...slotFixture(),
      sandboxed: true,
      has_editor: false,
      editor_open: false,
      status: { kind: "restarting", message: "Gain crashed and was bypassed" },
    });
    try {
      expect(badge(target)).toBe("Restarting…");
      expect(target.querySelector('[data-testid="rack-slot-status"]')?.textContent).toBe(
        "Gain crashed and was bypassed",
      );
      expect(target.querySelector('[data-testid="rack-slot-retry"]')).toBeNull();
      expect(target.querySelector(".body")).toBeNull();
    } finally {
      teardown();
    }
  });

  it("shows Plugin failed with the reason and a Retry that restarts the slot", async () => {
    const { target, teardown } = render({
      ...slotFixture(),
      sandboxed: true,
      has_editor: false,
      editor_open: false,
      status: { kind: "failed", message: "Gain stopped responding and was bypassed" },
    });
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return { slots: [], ab: false, latency_samples: 0 };
    });
    try {
      expect(badge(target)).toBe("Plugin failed");
      expect(target.querySelector('[data-testid="rack-slot-status"]')?.textContent).toBe(
        "Gain stopped responding and was bypassed",
      );
      const retry = target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-retry"]');
      expect(retry?.textContent?.trim()).toBe("Retry");
      retry!.click();
      await Promise.resolve();
      expect(calls).toContain("rack_restart");
    } finally {
      teardown();
    }
  });

  it("shows Not installed (no Retry) for a missing module, and no badge for a too-new state", () => {
    let r = render({
      ...slotFixture(),
      module_id: null,
      status: { kind: "missing", message: "Missing module test:gain@1.0.0", too_new: false },
    });
    expect(badge(r.target)).toBe("Not installed");
    expect(r.target.querySelector('[data-testid="rack-slot-retry"]')).toBeNull();
    r.teardown();
    r = render({
      ...slotFixture(),
      module_id: null,
      status: { kind: "missing", message: "x requires a newer version", too_new: true },
    });
    expect(badge(r.target)).toBeNull();
    expect(r.target.querySelector('[data-testid="rack-slot-retry"]')).toBeNull();
    r.teardown();
  });
});

// T-803: an out-of-process plugin being started off the control thread.
describe("loading slot (T-803)", () => {
  it("shows Loading… with no status message, body or Retry", () => {
    const { target, teardown } = render({
      ...slotFixture(),
      params: [],
      values: [],
      status: { kind: "loading" },
    });
    try {
      expect(target.querySelector('[data-testid="rack-slot-badge"]')?.textContent?.trim()).toBe("Loading…");
      expect(target.querySelector('[data-testid="rack-slot-status"]')).toBeNull();
      expect(target.querySelector('[data-testid="rack-slot-retry"]')).toBeNull();
      expect(target.querySelector(".body")).toBeNull();
    } finally {
      teardown();
    }
  });
});
