/**
 * Where the EQ graph's scale labels go (H-26), pure so "no label ever overlaps another label or a
 * unit" is tested. The graph draws its labels on the canvas over the plot:
 *
 * - gain labels down the left edge, one per grid line, edge-aligned at the top and bottom so
 *   none is cut off; the 0 dB line, then the range ends, win any conflict;
 * - frequency labels along the bottom edge;
 * - the frequency unit ("Hz") in a reserved slot at the right end of the frequency row — the
 *   last frequency label gives way to it;
 * - the bottom-left corner belongs to the gain scale: a frequency label that would touch a gain
 *   label is dropped.
 *
 * The gain unit ("dB") is the axis title in the toolbar row above the canvas (`EqGraph.svelte`),
 * outside the plot, so it can't collide with anything.
 */
import {
  estimateLabelWidthPx,
  fitAxisLabels,
  rectsOverlap,
  type LabelAlign,
  type Rect,
} from "../ui/axisLabels";
import { eqFrequencyTicks } from "./freqAxis";
import { gainAxisTicks } from "./gainAxis";

export const EQ_AXIS_FONT_PX = 10;
const LINE_PX = 12;
/** Distance of the labels from the canvas edge. */
const INSET_PX = 3;
/** Clear space kept between labels. */
const GAP_PX = 3;
/** Frequency and gain labels meeting in the bottom-left corner keep a clearer gap ("−12 50"). */
const CORNER_GAP_PX = 6;
/** Gain labels sit on grid lines 20 px apart at the compact graph height; 2 px keeps them all. */
const GAIN_GAP_PX = 2;

export interface EqAxisLabel {
  text: string;
  /** Canvas anchor for `fillText` with `align`/`baseline`. */
  x: number;
  y: number;
  align: CanvasTextAlign;
  baseline: CanvasTextBaseline;
  /** The box the text occupies, px. */
  rect: Rect;
}

export interface EqAxisLayout {
  gain: EqAxisLabel[];
  freq: EqAxisLabel[];
  freqUnit: EqAxisLabel;
}

function horizontal(text: string, start: number, align: LabelAlign, y: number): EqAxisLabel {
  const width = estimateLabelWidthPx(text, EQ_AXIS_FONT_PX);
  const rect: Rect = { x: start, y: y - LINE_PX, width, height: LINE_PX };
  const canvasAlign: CanvasTextAlign = align === "start" ? "left" : align === "end" ? "right" : "center";
  const x = align === "start" ? start : align === "end" ? start + width : start + width / 2;
  return { text, x, y, align: canvasAlign, baseline: "bottom", rect };
}

export function eqAxisLayout(
  width: number,
  height: number,
  rangeDb: number,
  fLo: number,
  fHi: number,
  formatFreq: (hz: number) => string,
  freqUnitText: string,
): EqAxisLayout {
  const rowY = height - INSET_PX;

  // The unit's reserved slot at the right end of the frequency row.
  const unitWidth = estimateLabelWidthPx(freqUnitText, EQ_AXIS_FONT_PX);
  const freqUnit = horizontal(freqUnitText, width - INSET_PX - unitWidth, "start", rowY);

  // Gain labels: the 0 dB line first, then the range ends, then the rest — earlier ones win.
  const gainTicks = gainAxisTicks(height, rangeDb);
  const rank = (db: number): number => (db === 0 ? 0 : Math.abs(db) === rangeDb ? 1 : 2);
  const ordered = [...gainTicks].sort((a, b) => rank(a.db) - rank(b.db));
  const gainFit = fitAxisLabels(
    ordered.map((tick) => ({ ...tick, pos: tick.y, size: LINE_PX })),
    { length: height, gapPx: GAIN_GAP_PX },
  );
  const gain: EqAxisLabel[] = gainFit
    .map((label): EqAxisLabel => {
      const textWidth = estimateLabelWidthPx(label.label, EQ_AXIS_FONT_PX);
      const baseline: CanvasTextBaseline =
        label.align === "start" ? "top" : label.align === "end" ? "bottom" : "middle";
      const y = label.align === "start" ? label.span.start : label.align === "end" ? label.span.end : label.pos;
      return {
        text: label.label,
        x: INSET_PX,
        y,
        align: "left",
        baseline,
        rect: { x: INSET_PX, y: label.span.start, width: textWidth, height: LINE_PX },
      };
    })
    .sort((a, b) => a.rect.y - b.rect.y);

  // Frequency labels: along [INSET, width − INSET], clear of the unit slot and the gain column.
  const usable = width - 2 * INSET_PX;
  const freqTicks = eqFrequencyTicks(fLo, fHi, width, 30, formatFreq);
  const freqFit = fitAxisLabels(
    freqTicks.map((tick) => ({
      ...tick,
      pos: Math.min(Math.max(tick.x - INSET_PX, 0), usable),
      size: estimateLabelWidthPx(tick.label, EQ_AXIS_FONT_PX),
    })),
    {
      length: usable,
      gapPx: GAP_PX,
      reserved: [{ start: freqUnit.rect.x - INSET_PX, end: freqUnit.rect.x - INSET_PX + unitWidth }],
    },
  );
  const freq = freqFit
    .map((label) => horizontal(label.label, label.span.start + INSET_PX, label.align, rowY))
    .filter((label) => !gain.some((g) => rectsOverlap(label.rect, g.rect, CORNER_GAP_PX)));

  return { gain, freq, freqUnit };
}
