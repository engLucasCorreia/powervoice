import type { CurveHandleDto, ParamInfoDto, ParamValueDto, ResponseCurveDto } from "../ipc/bindings";
import { xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";

/**
 * One draggable graph node (S3-07, SPEC-015 §3 "ResponseCurve components" / §2.6.3 "Nodes"):
 * derived from a module's `curve_handles` plus its current parameter values. Generic to any
 * module exposing `ResponseCurve` — `bandKey` is only recognized for the Parametric EQ's fixed
 * roles (`hp`, `ls`, `1`…`5`, `hs`, `lp`); any other module's bands fall back to their component
 * index as a label.
 */
export interface EqNode {
  /** Index into `ResponseCurveDto.components_db`. */
  component: number;
  /** `"hp" | "ls" | "1".."5" | "hs" | "lp"`, or `""` if not recognized (falls back to the index). */
  bandKey: string;
  freqId: number;
  gainId: number | null;
  qId: number | null;
  enableId: number | null;
  freqHz: number;
  /** `null` for HP/LP, which have no gain parameter (SPEC-015 §3 "handles"). */
  gainDb: number | null;
  q: number | null;
  enabled: boolean;
}

function valueOf(values: readonly ParamValueDto[], id: number, fallback: number): number {
  return values.find((v) => v.id === id)?.value ?? fallback;
}

/** `"hp_freq_hz"` → `"hp"`, `"b3_freq_hz"` → `"3"`, anything else → `""`. */
function bandKeyFromParamKey(key: string | undefined): string {
  if (!key) {
    return "";
  }
  const m = /^(hp|ls|hs|lp|b[1-5])_/.exec(key);
  if (!m) {
    return "";
  }
  const raw = m[1] ?? "";
  return raw.startsWith("b") ? raw.slice(1) : raw;
}

/** Builds the node list from a slot's `curve_handles`, `params` and current `values`. */
export function buildEqNodes(
  handles: readonly CurveHandleDto[],
  params: readonly ParamInfoDto[],
  values: readonly ParamValueDto[],
): EqNode[] {
  const keyOf = (id: number): string | undefined => params.find((p) => p.id === id)?.key;
  return handles.map((h) => ({
    component: h.component,
    bandKey: bandKeyFromParamKey(keyOf(h.freq)),
    freqId: h.freq,
    gainId: h.gain,
    qId: h.q,
    enableId: h.enable,
    freqHz: valueOf(values, h.freq, 1_000),
    gainDb: h.gain === null ? null : valueOf(values, h.gain, 0),
    q: h.q === null ? null : valueOf(values, h.q, 1),
    enabled: h.enable === null ? true : valueOf(values, h.enable, 1) >= 0.5,
  }));
}

/** The frequency of every node — what `curveRequestFreqs` inserts exactly (SPEC-015 §4.10). */
export function nodeFreqsHz(nodes: readonly EqNode[]): number[] {
  return nodes.map((n) => n.freqHz);
}

/**
 * The y value (dB) for a node's marker: its own gain for shelves/peaks, or the band's own
 * response at its cutoff for HP/LP, read from the curve's component row at the node's frequency
 * (nearest returned point — SPEC-015 §2.6.3 "HP/LP nodes sit on their curve"). `null` when no
 * curve has arrived yet and the node has no gain parameter either.
 */
export function nodeGainDb(node: EqNode, curve: ResponseCurveDto | null): number | null {
  if (node.gainDb !== null) {
    return node.gainDb;
  }
  if (!curve || curve.freqs_hz.length === 0) {
    return null;
  }
  const row = curve.components_db[node.component];
  if (!row) {
    return null;
  }
  let bestI = 0;
  let bestDist = Infinity;
  for (let i = 0; i < curve.freqs_hz.length; i++) {
    const f = curve.freqs_hz[i] ?? node.freqHz;
    const d = Math.abs(f - node.freqHz);
    if (d < bestDist) {
      bestDist = d;
      bestI = i;
    }
  }
  return row[bestI] ?? null;
}

/** Pixel hit-test radius (SPEC-015 §2.6 "Graph constants", `hit_radius`). */
export const EQ_NODE_HIT_RADIUS_PX = 10;

/**
 * The node nearest `(pointerX, pointerY)` within `hitRadiusPx`, or `null`. Positions come from
 * the same axis functions the graph draws with, so hit-testing always agrees with what's on
 * screen.
 */
export function hitTestNode(
  nodes: readonly EqNode[],
  pointerX: number,
  pointerY: number,
  width: number,
  height: number,
  fLo: number,
  fHi: number,
  rangeDb: number,
  curve: ResponseCurveDto | null,
  hitRadiusPx: number = EQ_NODE_HIT_RADIUS_PX,
): EqNode | null {
  let best: EqNode | null = null;
  let bestDist = hitRadiusPx;
  for (const node of nodes) {
    const x = xForFreq(node.freqHz, width, fLo, fHi);
    const y = yForDb(nodeGainDb(node, curve) ?? 0, height, rangeDb);
    const dist = Math.hypot(pointerX - x, pointerY - y);
    if (dist <= bestDist) {
      bestDist = dist;
      best = node;
    }
  }
  return best;
}
