import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { LocalizedTextDto, ParamGroupDto, ParamInfoDto, RackSlotDto } from "../ipc/bindings";
import { resetRackForTest } from "./rack.svelte";
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
  return {
    uid: 1,
    module: "org.powervoice.fixture@1.0.0",
    name: "Fixture",
    bypass: false,
    latency_samples: 0,
    status: { kind: "active" },
    params,
    groups,
    values: params.map((pp) => ({ id: pp.id, value: pp.default, normalized: 0, text: String(pp.default) })),
    noise_profile: null,
  };
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
});

function render(slot: RackSlotDto) {
  mockIPC(() => ({ slots: [], ab: false, latency_samples: 0 }));
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
