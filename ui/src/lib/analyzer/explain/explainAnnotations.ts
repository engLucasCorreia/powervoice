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
 *
 * **H-102**: a compact card's box used to be a fixed 34 px with a single `nowrap` line, so H-94's
 * full measured sentences were cut off mid-word ("The loudest peak in the spectrum is th…") —
 * worse than not showing them (ticket §2). The box is now sized to its own real content: a fixed
 * width that scales with the plot ({@link compactCardWidthPx}) and a height computed by wrapping
 * the actual `measured` sentence at that width ({@link compactCardHeightPx}/{@link wrapLines}),
 * so the card and H-93's collision layout both agree on the same real box — no truncation, ever.
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

// Mirrors ExplainFindingCard.svelte's compact layout exactly (padding 6px 8px, 3px row gap,
// `--pv-text-xs`/`--pv-leading-sm` = 11/16 px) so the height computed here is the height the DOM
// will actually take — the two must never drift apart, or the layout solver reserves the wrong
// amount of space again.
const CARD_FONT_PX = 11;
const CARD_LINE_PX = 16; // --pv-leading-sm
const CARD_PADDING_V_PX = 12; // 6px top + 6px bottom
const CARD_PADDING_H_PX = 16; // 8px left + 8px right
const CARD_ROW_GAP_PX = 3;
const CARD_MIN_WIDTH_PX = 130;
const CARD_MAX_WIDTH_PX = 260;
const CARD_WIDTH_FRACTION = 0.34;

/**
 * Greedy word-wrap of `text` into lines no wider than `maxWidthPx`, using the same per-character
 * width heuristic as every other estimate in the app ({@link estimateLabelWidthPx}) — deliberately
 * an overestimate for prose, so this errs toward *more* (shorter) lines rather than a line the
 * real font renders wider than expected. A single word that alone exceeds `maxWidthPx` still gets
 * its own line, unsplit, rather than looping or being cut.
 */
export function wrapLines(text: string, maxWidthPx: number, fontPx: number): string[] {
  const words = text.split(/\s+/).filter((w) => w.length > 0);
  if (words.length === 0) {
    return [];
  }
  const lines: string[] = [];
  let current = words[0]!;
  for (let i = 1; i < words.length; i++) {
    const word = words[i]!;
    const candidate = `${current} ${word}`;
    if (estimateLabelWidthPx(candidate, fontPx) <= maxWidthPx) {
      current = candidate;
    } else {
      lines.push(current);
      current = word;
    }
  }
  lines.push(current);
  return lines;
}

/**
 * The compact card's box height for `measured` at `widthPx`: one title line plus as many wrapped
 * lines as the sentence needs at that width — never a fixed guess that then gets cut off
 * mid-sentence (H-102 ticket §2).
 */
export function compactCardHeightPx(measured: string, widthPx: number): number {
  const textWidthPx = Math.max(10, widthPx - CARD_PADDING_H_PX);
  const lines = Math.max(1, wrapLines(measured, textWidthPx, CARD_FONT_PX).length);
  return CARD_PADDING_V_PX + CARD_LINE_PX + CARD_ROW_GAP_PX + lines * CARD_LINE_PX;
}

/**
 * The compact card's width for a plot `rectWidthPx` wide: a consistent fraction of the plot so
 * the cards read as one family, clamped so a card stays legible on a phone-width plot and never
 * dominates a wide desktop one.
 */
export function compactCardWidthPx(rectWidthPx: number): number {
  return Math.min(CARD_MAX_WIDTH_PX, Math.max(CARD_MIN_WIDTH_PX, rectWidthPx * CARD_WIDTH_FRACTION));
}

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

/**
 * One (finding, prose) pair → one candidate card, or `null` when it has nothing to point at.
 * `cardWidthPx` defaults to the widest allowed size for callers (tests, mostly) that don't yet
 * know their plot's width; `layoutExplainAnnotations` always passes the real one.
 */
export function buildAnnotationItem(
  finding: VoiceFinding,
  prose: FindingProse,
  geometry: ExplainAnnotationGeometry,
  cardWidthPx: number = CARD_MAX_WIDTH_PX,
): ExplainAnnotationItem | null {
  const freqHz = anchorFreqHz(finding);
  if (freqHz === null || !(freqHz > 0) || !Number.isFinite(freqHz)) {
    return null;
  }
  return {
    id: finding.id,
    finding,
    prose,
    width: cardWidthPx,
    height: compactCardHeightPx(prose.measured, cardWidthPx),
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
  const cardWidthPx = compactCardWidthPx(options.rect.width);
  const items: ExplainAnnotationItem[] = [];
  const unanchored: VoiceFinding[] = [];
  for (let i = 0; i < findings.length; i++) {
    const finding = findings[i]!;
    const item = buildAnnotationItem(finding, prose[i]!, geometry, cardWidthPx);
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
