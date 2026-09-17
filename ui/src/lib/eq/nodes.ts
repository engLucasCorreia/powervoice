import type {
  CurveHandleDto,
  ParamGroupDto,
  ParamInfoDto,
  ParamValueDto,
  RackSlotDto,
  ResponseCurveDto,
} from "../ipc/bindings";
import { localized } from "../rack/localized";
import { t, tDynamic } from "../i18n";
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
  /** `hp_slope`/`lp_slope`'s id (H-84, SPEC-015 §3 "the panel finds the slope through the
   * group"): `curve_handles` carries no slope field, so this is found by convention
   * (`${bandKey}_slope`) among the slot's own `params`, not through the handle. `null` for any
   * band with no slope parameter. */
  slopeId: number | null;
  freqHz: number;
  /** `null` for HP/LP, which have no gain parameter (SPEC-015 §3 "handles"). */
  gainDb: number | null;
  q: number | null;
  /** The slope's current enum index (0…7, `hp_slope`/`lp_slope`), or `null` with no slope
   * parameter. */
  slope: number | null;
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
  return handles.map((h) => {
    const bandKey = bandKeyFromParamKey(keyOf(h.freq));
    const slopeParam = bandKey ? params.find((p) => p.key === `${bandKey}_slope`) : undefined;
    return {
      component: h.component,
      bandKey,
      freqId: h.freq,
      gainId: h.gain,
      qId: h.q,
      enableId: h.enable,
      slopeId: slopeParam?.id ?? null,
      freqHz: valueOf(values, h.freq, 1_000),
      gainDb: h.gain === null ? null : valueOf(values, h.gain, 0),
      q: h.q === null ? null : valueOf(values, h.q, 1),
      slope: slopeParam ? valueOf(values, slopeParam.id, slopeParam.default) : null,
      enabled: h.enable === null ? true : valueOf(values, h.enable, 1) >= 0.5,
    };
  });
}

/** `min`/`max` of parameter `id` in `params`, or `null` if it isn't one of them (defensive —
 * every id an `EqNode` carries comes from the same slot's `params`). */
export function paramRangeOf(
  params: readonly ParamInfoDto[],
  id: number | null,
): { min: number; max: number; default: number } | null {
  if (id === null) {
    return null;
  }
  const p = params.find((info) => info.id === id);
  return p ? { min: p.min, max: p.max, default: p.default } : null;
}

/** The header/tab short label ("HP", "L", "1"…"5", "H", "LP"), or the raw component index for a
 * non-Parametric-EQ module (S3-07). */
export function nodeShortLabel(node: EqNode): string {
  return node.bandKey ? tDynamic(`eq.band.${node.bandKey}`) : String(node.component);
}

/** The band's full accessible name (H-84, AC-19): the slot's own `ParamGroupDto` whose
 * `enable_param` matches this node's enable id — Rust's already-localized group name (ADR-005
 * §2), generic to any module, not just the EQ's fixed roles. Falls back to the short label when
 * no such group exists (a node with no `enableId`, or a non-EQ module with no groups at all). */
export function nodeFullName(node: EqNode, groups: readonly ParamGroupDto[]): string {
  const group =
    node.enableId !== null ? groups.find((g) => g.enable_param === node.enableId) : undefined;
  return group ? localized(group.name) : nodeShortLabel(node);
}

function paramTextOf(values: readonly ParamValueDto[], id: number | null): string | null {
  return id === null ? null : (values.find((v) => v.id === id)?.text ?? null);
}

/**
 * The node's accessible value text (H-84, SPEC-015 §2.6.4/§2.6.5): "Band 3 · 1.20 kHz · +3.0 dB ·
 * Q 1.00" for a peak/shelf, "High-pass · 80 Hz · 24 dB/oct" for HP/LP — every number Rust's own
 * `param_changed` text (§2.6.4 "the texts are Rust's"), never formatted here. Used for both
 * `aria-valuetext` and the throttled live-region announcement, and suffixed when the band is off.
 */
export function nodeValueText(node: EqNode, rackSlot: RackSlotDto): string {
  const band = nodeFullName(node, rackSlot.groups);
  const freq = paramTextOf(rackSlot.values, node.freqId) ?? "";
  const base =
    node.slopeId !== null
      ? t("eq.graph.node_value_slope", {
          band,
          freq,
          slope: paramTextOf(rackSlot.values, node.slopeId) ?? "",
        })
      : t("eq.graph.node_value", {
          band,
          freq,
          gain: paramTextOf(rackSlot.values, node.gainId) ?? "",
          q: paramTextOf(rackSlot.values, node.qId) ?? "",
        });
  return node.enabled ? base : t("eq.graph.node_value_off", { value: base });
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
