/**
 * Turns H-91's findings (with H-94's words already attached) into H-93's `AnnotationItem`s for
 * the "Explain My Voice" graph: the one seam between the three models. The geometry itself is
 * H-93's `layoutAnnotations`, untouched here (H-92 ticket: "pass curve-peak regions as soft
 * `avoid` rects and fixed chrome as hard `reserved` rects").
 *
 * A finding only gets a card on the graph when it has a real measured anchor
 * (`FindingAnchor.kind !== "none"`) — SNR, the broadband noise floor and "no hum detected" don't
 * point at a frequency, so they are never given one; they, plus anything H-93 had to drop for
 * space, come back as `beneath` for the caller to render as full cards instead (H-92 ticket §6:
 * "mobile shows the top three annotations on the graph and the rest as cards beneath" — the same
 * bucket serves the desktop overflow too).
 *
 * A `band` anchor (body, presence, air, sibilance, rumble, the harmonics summary) is placed at
 * the band's own geometric mid — the midpoint on the log axis the band already occupies, not an
 * arbitrary point outside the measured range — and its `y` is read straight off the drawn
 * envelope at that exact frequency, so the leader line always lands on a real point of the curve
 * the reader is looking at.
 */
import { estimateLabelWidthPx, type Rect } from "../../ui/axisLabels";
import { layoutAnnotations, type AnnotationItem, type PlacedAnnotation } from "../annotationLayout";
import type { VoiceFinding } from "./findings";
import type { FindingProse } from "./prose";

export interface ExplainAnnotationItem extends AnnotationItem {
  finding: VoiceFinding;
  prose: FindingProse;
}

export interface ExplainAnnotationGeometry {
  /** Plot-pixel x for a measured frequency (through the shared log/linear axis). */
  xForFreq: (freqHz: number) => number;
  /** Plot-pixel y of the drawn envelope at a measured frequency — where the leader line lands. */
  yForCurveAtFreq: (freqHz: number) => number;
}

/** The compact card's fixed height: title + one line of `measured` (H-92: the graph's floating
 * cards show title + measured only — the full four blocks are for the "also measured" list). */
const CARD_HEIGHT_PX = 34;
const CARD_TITLE_FONT_PX = 11;
const CARD_VALUE_FONT_PX = 10;
const CARD_PADDING_PX = 16;
const CARD_MAX_WIDTH_PX = 220;

/** The frequency a finding's anchor names, or `null` for `{ kind: "none" }`. A `band` anchor
 * uses its geometric mid — the visual centre of that band on a log frequency axis. */
export function anchorFreqHz(finding: VoiceFinding): number | null {
  const a = finding.anchor;
  switch (a.kind) {
    case "frequency":
      return a.freqHz;
    case "band":
      return a.lowHz > 0 && a.highHz > 0 ? Math.sqrt(a.lowHz * a.highHz) : null;
    default:
      return null;
  }
}

/** One (finding, prose) pair → one candidate card, or `null` when it has nothing to point at. */
export function buildAnnotationItem(
  finding: VoiceFinding,
  prose: FindingProse,
  geometry: ExplainAnnotationGeometry,
): ExplainAnnotationItem | null {
  const freqHz = anchorFreqHz(finding);
  if (freqHz === null || !(freqHz > 0) || !Number.isFinite(freqHz)) {
    return null;
  }
  const width = Math.min(
    CARD_MAX_WIDTH_PX,
    Math.max(
      estimateLabelWidthPx(prose.title, CARD_TITLE_FONT_PX),
      estimateLabelWidthPx(prose.measured, CARD_VALUE_FONT_PX),
    ) + CARD_PADDING_PX,
  );
  return {
    id: finding.id,
    finding,
    prose,
    width,
    height: CARD_HEIGHT_PX,
    anchor: { x: geometry.xForFreq(freqHz), y: geometry.yForCurveAtFreq(freqHz) },
    // H-93 sorts ascending ("lower number = more important" — it drops the highest numbers
    // first); `VoiceFinding.priority` is the opposite convention ("highest first", H-91's
    // thresholds.ts). Negating here is the one place the two meet.
    priority: -finding.priority,
  };
}

export interface ExplainLayoutOptions {
  rect: Rect;
  /** Soft: the curve's own significant peaks, so a card prefers not to sit on one. */
  avoid?: readonly Rect[];
  /** Hard: chrome a card may never cover (legend, hover readout). */
  reserved?: readonly Rect[];
  maxLabels?: number;
}

export interface ExplainLayoutResult {
  placed: PlacedAnnotation<ExplainAnnotationItem>[];
  /** Findings with no card on the graph — no measured frequency to anchor to, or dropped by
   * H-93 for space — highest H-91 priority first. */
  beneath: VoiceFinding[];
}

/**
 * Lays out every finding that can go on the graph, and returns the rest (by their `VoiceFinding`,
 * so the caller can look up the matching `FindingProse` for its beneath-the-graph list — the two
 * arrays are the same length, in the same order, one `FindingProse` per `VoiceFinding`).
 */
export function layoutExplainAnnotations(
  findings: readonly VoiceFinding[],
  prose: readonly FindingProse[],
  geometry: ExplainAnnotationGeometry,
  options: ExplainLayoutOptions,
): ExplainLayoutResult {
  const items: ExplainAnnotationItem[] = [];
  const unanchored: VoiceFinding[] = [];
  for (let i = 0; i < findings.length; i++) {
    const finding = findings[i]!;
    const item = buildAnnotationItem(finding, prose[i]!, geometry);
    if (item) {
      items.push(item);
    } else {
      unanchored.push(finding);
    }
  }
  const { placed, dropped } = layoutAnnotations(items, {
    rect: options.rect,
    avoid: options.avoid,
    reserved: options.reserved,
    maxLabels: options.maxLabels,
  });
  const beneath = [...dropped.map((d) => d.finding), ...unanchored].sort((a, b) => b.priority - a.priority);
  return { placed, beneath };
}
