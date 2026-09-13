import { describe, expect, it } from "vitest";
import type {
  CurveHandleDto,
  LocalizedTextDto,
  ParamInfoDto,
  ParamValueDto,
  ResponseCurveDto,
} from "../ipc/bindings";
import { xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";
import { EQ_NODE_HIT_RADIUS_PX, buildEqNodes, hitTestNode, nodeFreqsHz, nodeGainDb } from "./nodes";

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
    param(30, "b1_on", { unit: { kind: "none" }, min: 0, max: 1, default: 1 }),
    param(31, "b1_freq_hz", { default: 200 }),
    param(32, "b1_gain_db", { unit: { kind: "db" }, min: -24, max: 24, default: 0 }),
    param(33, "b1_q", { unit: { kind: "none" }, min: 0.1, max: 30, default: 1 }),
  ];
  const values = [
    value(10, 0), // HP off
    value(11, 80),
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
      freqHz: 80,
      gainDb: null,
      q: null,
      enabled: false,
    });

    expect(nodes[1]).toMatchObject({
      component: 2,
      bandKey: "1",
      freqId: 31,
      gainId: 32,
      qId: 33,
      enableId: 30,
      freqHz: 1_000,
      gainDb: 6,
      q: 2,
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
