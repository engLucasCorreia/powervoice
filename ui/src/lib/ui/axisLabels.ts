/**
 * Axis-label collision maths (H-26), shared by every scale in the app: analyzer dB and frequency
 * axes, the EQ graph, the editor's time ruler, the spectral frequency ruler. Pure, so "no tick
 * label ever overlaps another label, a unit or the axis end" is a tested property rather than a
 * visual hope.
 *
 * Positions are in pixels along one axis (`fitAxisLabels`) or in a plane (`rectsOverlap`, for the
 * canvas-drawn EQ graph where both axes share one surface). Units get a *reserved slot* — an axis
 * title band, a corner cell, or the far end of an axis — that tick labels are fitted around.
 */
export type LabelAlign = "start" | "center" | "end";

export interface Span {
  start: number;
  end: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The span a label of `size` px occupies when anchored at `pos` with `align`. */
export function labelSpan(pos: number, size: number, align: LabelAlign): Span {
  switch (align) {
    case "start":
      return { start: pos, end: pos + size };
    case "end":
      return { start: pos - size, end: pos };
    default:
      return { start: pos - size / 2, end: pos + size / 2 };
  }
}

/** True when the spans overlap or come closer than `gapPx`. Touching at `gapPx = 0` is fine. */
export function spansOverlap(a: Span, b: Span, gapPx = 0): boolean {
  return a.start < b.end + gapPx && b.start < a.end + gapPx;
}

export function rectsOverlap(a: Rect, b: Rect, gapPx = 0): boolean {
  return (
    spansOverlap({ start: a.x, end: a.x + a.width }, { start: b.x, end: b.x + b.width }, gapPx) &&
    spansOverlap({ start: a.y, end: a.y + a.height }, { start: b.y, end: b.y + b.height }, gapPx)
  );
}

/**
 * Centre a label on its tick, except within half a label of either end of the axis, where it
 * aligns inward so it isn't cut off (the first label starts at its tick, the last ends at it).
 */
export function edgeAlign(pos: number, length: number, size: number): LabelAlign {
  const half = size / 2;
  if (pos - half < 0) {
    return "start";
  }
  if (pos + half > length) {
    return "end";
  }
  return "center";
}

export interface AxisLabel {
  /** Tick position along the axis, px. */
  pos: number;
  /** Label extent along the axis, px (text width on a horizontal axis, line height on a vertical one). */
  size: number;
  /** Omit to edge-align automatically. */
  align?: LabelAlign;
}

export interface FitOptions {
  /** Axis length, px: labels must stay within `[0, length]`. */
  length: number;
  /** Slots kept free for units or a crossing axis. */
  reserved?: readonly Span[];
  /** Minimum clear space between neighbouring labels. */
  gapPx?: number;
}

export type Fitted<T> = T & { align: LabelAlign; span: Span };

/**
 * Keeps the labels that fit: inside the axis, clear of every reserved slot and of every label
 * kept before them. Labels are considered in the order given, so callers list the ones that
 * matter most first (the 0 dB line, the axis ends).
 */
export function fitAxisLabels<T extends AxisLabel>(labels: readonly T[], options: FitOptions): Fitted<T>[] {
  const gap = options.gapPx ?? 2;
  const reserved = options.reserved ?? [];
  const kept: Fitted<T>[] = [];
  for (const label of labels) {
    const align = label.align ?? edgeAlign(label.pos, options.length, label.size);
    const span = labelSpan(label.pos, label.size, align);
    if (span.start < -0.5 || span.end > options.length + 0.5) {
      continue;
    }
    if (reserved.some((slot) => spansOverlap(span, slot, gap))) {
      continue;
    }
    if (kept.some((other) => spansOverlap(span, other.span, gap))) {
      continue;
    }
    kept.push({ ...label, align, span });
  }
  return kept;
}

/**
 * Width of a short axis label in px, without measuring text: axis labels are digits, a sign, a
 * decimal point and a unit letter or two, set in tabular figures, so ~0.6 em per character is a
 * safe upper bound for the system UI fonts at 9–11 px.
 */
export function estimateLabelWidthPx(text: string, fontPx: number): number {
  return Math.ceil([...text].length * fontPx * 0.62);
}

export interface GutterOptions {
  /** Gutter height, px (the axis length). */
  length: number;
  /** Gutter width, px; tick labels are right-aligned inside it. */
  width: number;
  fontPx: number;
  lineHeightPx: number;
  /** A unit drawn in the gutter's top-left corner: ticks that would touch it are dropped. */
  unit?: { text: string; fontPx: number };
  /** Inset from the gutter's edges, px. */
  padPx?: number;
}

/**
 * Tick labels for a vertical ruler gutter (spectral frequency ruler, waveform amplitude ruler):
 * right-aligned, edge-aligned at the top and bottom so none is cut off, clear of each other and
 * of the unit in the corner. The unit keeps its slot; a tick that would touch it is dropped.
 */
export function fitGutterLabels<T extends { pos: number; text: string }>(
  ticks: readonly T[],
  options: GutterOptions,
): Fitted<T & AxisLabel>[] {
  const pad = options.padPx ?? 2;
  const fitted = fitAxisLabels(
    ticks.map((tick) => ({ ...tick, size: options.lineHeightPx })),
    { length: options.length },
  );
  if (!options.unit) {
    return fitted;
  }
  const unitRect: Rect = {
    x: pad,
    y: pad,
    width: estimateLabelWidthPx(options.unit.text, options.unit.fontPx),
    height: options.lineHeightPx,
  };
  return fitted.filter((label) => {
    const width = estimateLabelWidthPx(label.text, options.fontPx);
    const rect: Rect = {
      x: options.width - pad - width,
      y: label.span.start,
      width,
      height: label.span.end - label.span.start,
    };
    return !rectsOverlap(rect, unitRect, 2);
  });
}
