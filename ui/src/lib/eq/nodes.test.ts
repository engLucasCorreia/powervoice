import { describe, expect, it } from "vitest";
import type {
  CurveHandleDto,
  LocalizedTextDto,
  ParamGroupDto,
  ParamInfoDto,
  ParamValueDto,
  RackSlotDto,
  ResponseCurveDto,
} from "../ipc/bindings";
import { rackSlotDto } from "../test/fixtures";
import { xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";
import {
  EQ_NODE_HIT_RADIUS_PX,
  buildEqNodes,
  hitTestNode,
  nodeFreqsHz,
  nodeFullName,
  nodeGainDb,
  nodeShortLabel,
  nodeValueText,
  paramRangeOf,
} from "./nodes";

/** Node derivation and hit-testing tests (S3-07, SPEC-015 §3 "handles" / §2.6.3 "Nodes"). */

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
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...overrides,
  };
}

function value(id: number, v: number): ParamValueDto {
  return { id, value: v, normalized: 0.5, text: String(v) };
}

/** A minimal 9-band-like fixture: one peak band (freq/gain/q/enable) and one HP-like band
 * (freq/enable only), mirroring the Parametric EQ's `handles()` shape. */
function fixture(): { handles: CurveHandleDto[]; params: ParamInfoDto[]; values: ParamValueDto[] } {
  const handles: CurveHandleDto[] = [
    { component: 0, freq: 11, gain: null, q: null, enable: 10 }, // hp_freq_hz / hp_on
    { component: 2, freq: 31, gain: 32, q: 33, enable: 30 }, // b1_freq_hz / gain / q / on
  ];
  const params = [
    param(10, "hp_on", { unit: { kind: "none" }, min: 0, max: 1, default: 0 }),
    param(11, "hp_freq_hz", { default: 80 }),
    param(14, "hp_slope", { unit: { kind: "none" }, min: 0, max: 7, default: 3 }),
    param(30, "b1_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(31, "b1_freq_hz", { default: 200 }),
    param(32, "b1_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 0 }),
    param(33, "b1_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 1 }),
  ];
  const values = [
    value(10, 0), // HP off
    value(11, 80),
    value(14, 3), // 24 dB/oct
    value(30, 1), // band 1 on
    value(31, 1_000),
    value(32, 6),
    value(33, 2),
  ];
  return { handles, params, values };
}

describe("buildEqNodes", () => {
  it("derives freq/gain/q/enable and the band key from the frequency param's key", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    expect(nodes).toHaveLength(2);

    expect(nodes[0]).toMatchObject({
      component: 0,
      bandKey: "hp",
      freqId: 11,
      gainId: null,
      qId: null,
      enableId: 10,
      slopeId: 14,
      freqHz: 80,
      gainDb: null,
      q: null,
      slope: 3,
      enabled: false,
    });

    expect(nodes[1]).toMatchObject({
      component: 2,
      bandKey: "1",
      freqId: 31,
      gainId: 32,
      qId: 33,
      enableId: 30,
      slopeId: null,
      freqHz: 1_000,
      gainDb: 6,
      q: 2,
      slope: null,
      enabled: true,
    });
  });

  it("falls back to an empty band key for an unrecognized parameter key", () => {
    const handles: CurveHandleDto[] = [{ component: 5, freq: 99, gain: null, q: null, enable: null }];
    const params = [param(99, "cutoff_hz")];
    const nodes = buildEqNodes(handles, params, []);
    expect(nodes[0]!.bandKey).toBe("");
    expect(nodes[0]!.enabled).toBe(true); // no enable param: always enabled
  });
});

describe("nodeFreqsHz", () => {
  it("lists every node's frequency, in order", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    expect(nodeFreqsHz(nodes)).toEqual([80, 1_000]);
  });
});

describe("nodeGainDb", () => {
  it("uses the parameter's own gain for a shelf/peak node", () => {
    const { handles, params, values } = fixture();
    const node = buildEqNodes(handles, params, values)[1]!;
    expect(nodeGainDb(node, null)).toBe(6);
  });

  it("reads the band's own response at the nearest curve point for HP/LP (no gain param)", () => {
    const { handles, params, values } = fixture();
    const node = buildEqNodes(handles, params, values)[0]!; // HP, component 0
    const curve: ResponseCurveDto = {
      freqs_hz: [20, 79, 100, 1_000],
      sample_rate_hz: 48_000,
      total_db: [0, 0, 0, 0],
      components_db: [
        [-24, -3, -1, 0], // component 0 (HP)
        [0, 0, 0, 0], // component 2 (unused here)
      ],
    };
    // node.freqHz = 80, nearest returned point is 79.
    expect(nodeGainDb(node, curve)).toBe(-3);
  });

  it("is null with no curve and no gain parameter", () => {
    const { handles, params, values } = fixture();
    const node = buildEqNodes(handles, params, values)[0]!;
    expect(nodeGainDb(node, null)).toBeNull();
  });
});

describe("hitTestNode", () => {
  it("finds the nearest node within the hit radius", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    // Band 1 (component 2) at 1000 Hz / +6 dB: compute its expected pixel position with the same
    // axes the graph draws with (imported indirectly through hitTestNode's own axis calls) by
    // picking a point exactly at its known screen position via the public axis helpers.
    const width = 400;
    const height = 160;
    const fLo = 20;
    const fHi = 20_000;
    const range = 12;
    // Re-derive x/y the same way EqGraph does, to assert the hit test agrees.
    const x = xForFreq(1_000, width, fLo, fHi);
    const y = yForDb(6, height, range);

    const hit = hitTestNode(nodes, x, y, width, height, fLo, fHi, range, null);
    expect(hit?.component).toBe(2);

    const miss = hitTestNode(nodes, x + 100, y, width, height, fLo, fHi, range, null);
    expect(miss).toBeNull();
  });

  it("respects a custom hit radius", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    const x = xForFreq(1_000, 400, 20, 20_000) + EQ_NODE_HIT_RADIUS_PX + 5;
    const y = yForDb(6, 160, 12);
    expect(hitTestNode(nodes, x, y, 400, 160, 20, 20_000, 12, null)).toBeNull();
    expect(
      hitTestNode(nodes, x, y, 400, 160, 20, 20_000, 12, null, EQ_NODE_HIT_RADIUS_PX + 10),
    ).not.toBeNull();
  });
});

function group(id: number, key: string, enable: number | null): ParamGroupDto {
  return { id, key, name: text(key === "hp" ? "High-pass" : "Band 1"), parent: null, enable_param: enable, collapsed_by_default: true };
}

describe("nodeShortLabel (S3-07 header/tab label)", () => {
  it("uses the eq.band.* short label for a recognized band key", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    expect(nodeShortLabel(nodes[0]!)).toBe("HP");
    expect(nodeShortLabel(nodes[1]!)).toBe("1");
  });

  it("falls back to the component index for an unrecognized band key", () => {
    const handles: CurveHandleDto[] = [{ component: 5, freq: 99, gain: null, q: null, enable: null }];
    const params = [param(99, "cutoff_hz")];
    const node = buildEqNodes(handles, params, [])[0]!;
    expect(nodeShortLabel(node)).toBe("5");
  });
});

describe("nodeFullName (H-84, AC-19 accessible band name)", () => {
  it("uses the slot's own localized group name when one enables this node", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    const groups: ParamGroupDto[] = [group(1, "hp", 10), group(2, "band_1", 30)];
    expect(nodeFullName(nodes[0]!, groups)).toBe("High-pass");
    expect(nodeFullName(nodes[1]!, groups)).toBe("Band 1");
  });

  it("falls back to the short label with no matching group", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    expect(nodeFullName(nodes[0]!, [])).toBe(nodeShortLabel(nodes[0]!));
  });
});

describe("nodeValueText (H-84, aria-valuetext / live-region text)", () => {
  function slotFor(params: ParamInfoDto[], values: ParamValueDto[], groups: ParamGroupDto[] = []): RackSlotDto {
    return rackSlotDto({ params, values, groups });
  }

  it("builds 'band · freq · gain · Q' for a peak/shelf band, from Rust's own value texts", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    const withTexts = values.map((v) =>
      v.id === 31 ? { ...v, text: "1.00 kHz" } : v.id === 32 ? { ...v, text: "+6.0 dB" } : v.id === 33 ? { ...v, text: "2.00" } : v,
    );
    const slot = slotFor(params, withTexts, [group(2, "band_1", 30)]);
    expect(nodeValueText(nodes[1]!, slot)).toBe("Band 1 · 1.00 kHz · +6.0 dB · 2.00");
  });

  it("builds 'band · freq · slope' for HP/LP, and marks a disabled band", () => {
    const { handles, params, values } = fixture();
    const nodes = buildEqNodes(handles, params, values);
    const withTexts = values.map((v) =>
      v.id === 11 ? { ...v, text: "80 Hz" } : v.id === 14 ? { ...v, text: "24 dB/oct" } : v,
    );
    const slot = slotFor(params, withTexts, [group(1, "hp", 10)]);
    expect(nodeValueText(nodes[0]!, slot)).toBe("High-pass · 80 Hz · 24 dB/oct (off)");
  });
});

describe("paramRangeOf", () => {
  it("returns min/max/default for a known id", () => {
    const { params } = fixture();
    expect(paramRangeOf(params, 32)).toEqual({ min: -24, max: 24, default: 0 });
  });

  it("is null for a null id or an id not in params", () => {
    const { params } = fixture();
    expect(paramRangeOf(params, null)).toBeNull();
    expect(paramRangeOf(params, 999)).toBeNull();
  });
});
