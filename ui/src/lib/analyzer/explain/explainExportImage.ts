/**
 * H-115's exported image: a print-quality PNG of the whole analysis (header, engineering
 * summary, the annotated graph and the "also measured" cards) — the same content the modal
 * shows, at a resolution that stays legible on paper.
 *
 * Split in two for testability (jsdom has no real canvas context, MEMORY.md: `ExplainGraph`'s own
 * `draw()` is untested for the same reason): {@link measureExplainExportLayout} is pure geometry
 * over {@link ExplainExportData} (fully unit-tested); {@link drawExplainExportImage} walks that
 * plan calling a small drawing surface ({@link DrawCtx2D}, a structural subset of
 * `CanvasRenderingContext2D` a test double can implement without a real canvas — also tested);
 * only {@link renderExplainExportPng}'s canvas creation and `toBlob` glue is unverified under
 * jsdom, the same boundary every other renderer in this codebase draws its untested line at.
 */
import { themeTokenValue } from "../../theme/themeColors";
import { wrapLines } from "./explainAnnotations";
import type { FindingSeverity } from "./findings";
import type { ExplainExportData } from "./explainExport";

const FONT_FAMILY =
  "system-ui, -apple-system, 'Segoe UI', Cantarell, 'Noto Sans', Ubuntu, Roboto, 'Helvetica Neue', Arial, sans-serif";

const PAGE_PADDING_PX = 28;
const MIN_PAGE_WIDTH_PX = 860;
const SECTION_GAP_PX = 20;
const TITLE_FONT_PX = 22;
const TITLE_LINE_PX = 30;
const META_FONT_PX = 13;
const META_LINE_PX = 18;
const HEADLINE_FONT_PX = 15;
const HEADLINE_LINE_PX = 21;
const BASIS_FONT_PX = 12;
const BASIS_LINE_PX = 16;
const HEADING_FONT_PX = 12;
const HEADING_LINE_PX = 22;
const BODY_FONT_PX = 12;
const BODY_LINE_PX = 17;
const CARD_FONT_PX = 11;
const CARD_LINE_PX = 15;
const CARD_PADDING_H_PX = 10;
const CARD_PADDING_V_PX = 8;
const CARD_GAP_PX = 8;
const CARD_STRIPE_PX = 3;

/** The drawing surface this module needs — a structural subset of `CanvasRenderingContext2D`
 * (assignable both from a real one and from a test double), so the composition logic is testable
 * without a real canvas. */
export interface DrawCtx2D {
  save(): void;
  restore(): void;
  scale(x: number, y: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  fillText(text: string, x: number, y: number): void;
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  stroke(): void;
  drawImage(
    image: CanvasImageSource,
    sx: number,
    sy: number,
    sw: number,
    sh: number,
    dx: number,
    dy: number,
    dw: number,
    dh: number,
  ): void;
  // Widened to match `CanvasRenderingContext2D`'s own property type exactly (TS requires mutable
  // properties to match, not merely be assignable) — this module only ever writes plain strings.
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  lineWidth: number;
  font: string;
  textAlign: CanvasTextAlign;
  textBaseline: CanvasTextBaseline;
}

/** Named colour roles the plan draws with — resolved to real CSS colours only at draw time
 * ({@link resolveExportPalette}), so {@link measureExplainExportLayout} stays theme-agnostic and
 * plain to test. */
export type ExportColorKey =
  | "background"
  | "titleText"
  | "metaText"
  | "headlineText"
  | "basisText"
  | "headingText"
  | "bodyText"
  | "cardBg"
  | "cardText"
  | "cardMetaText"
  | "leaderLine"
  | "severityGood"
  | "severityInfo"
  | "severityAttention"
  | "severitySignificant";

export type ExportPalette = Record<ExportColorKey, string>;

/** Which CSS custom property (`design-tokens.css`) backs each colour role above. Names only —
 * never a colour value itself (T-708: this file writes down no colour; {@link resolveExportPalette}
 * reads every one of these through `themeColors.ts`'s `themeTokenValue`, the same
 * `getComputedStyle`-first lookup every renderer's colours already go through). */
const EXPORT_PALETTE_TOKENS: Record<ExportColorKey, string> = {
  background: "--pv-bg-app",
  titleText: "--pv-text-primary",
  metaText: "--pv-text-tertiary",
  headlineText: "--pv-text-primary",
  basisText: "--pv-text-tertiary",
  headingText: "--pv-text-secondary",
  bodyText: "--pv-text-secondary",
  cardBg: "--pv-bg-overlay",
  cardText: "--pv-text-primary",
  cardMetaText: "--pv-text-secondary",
  leaderLine: "--pv-border",
  severityGood: "--pv-success-text",
  severityInfo: "--pv-accent-text",
  severityAttention: "--pv-warning-text",
  severitySignificant: "--pv-danger-text",
};

function severityColorKey(severity: FindingSeverity): ExportColorKey {
  switch (severity) {
    case "good":
      return "severityGood";
    case "info":
      return "severityInfo";
    case "attention":
      return "severityAttention";
    case "significant":
      return "severitySignificant";
  }
}

interface PlanText {
  text: string;
  x: number;
  y: number;
  fontPx: number;
  weight: "400" | "700";
  color: ExportColorKey;
}

interface PlanCard {
  x: number;
  y: number;
  width: number;
  height: number;
  stripeColor: ExportColorKey;
  lines: PlanText[];
}

interface PlanLeader {
  from: { x: number; y: number };
  to: { x: number; y: number };
}

interface PlanGraphImage {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The fully laid-out export image: page size plus every text run, card box and leader line to
 * draw, in page pixels (pre device-pixel scaling — {@link drawExplainExportImage} takes care of
 * that). Pure data: nothing here depends on a canvas or a theme. */
export interface ExplainExportImagePlan {
  widthPx: number;
  heightPx: number;
  texts: PlanText[];
  cards: PlanCard[];
  leaders: PlanLeader[];
  graphImage: PlanGraphImage;
}

function text(lines: PlanText[], line: PlanText): void {
  lines.push(line);
}

/** Wraps `body` at `widthPx`, using the same greedy heuristic every other estimate in this
 * feature uses ({@link wrapLines}) — not a canvas measurement, deliberately: the same estimate
 * `explainAnnotations.ts` already used to size these very cards on screen. */
function wrapped(bodyText: string, widthPx: number, fontPx: number): string[] {
  return wrapLines(bodyText, widthPx, fontPx);
}

/**
 * Pure geometry: where everything goes, and how tall the page ends up being. No `DrawCtx2D`, no
 * theme — {@link drawExplainExportImage} is the only thing that turns this into pixels.
 */
export function measureExplainExportLayout(data: ExplainExportData): ExplainExportImagePlan {
  const pageWidth = Math.max(MIN_PAGE_WIDTH_PX, data.graph.widthPx);
  const contentWidth = pageWidth - PAGE_PADDING_PX * 2;
  const texts: PlanText[] = [];
  let y = PAGE_PADDING_PX;

  // Header: title, then the analysed-span subtitle and the date on one meta line.
  text(texts, { text: data.title, x: PAGE_PADDING_PX, y, fontPx: TITLE_FONT_PX, weight: "700", color: "titleText" });
  y += TITLE_LINE_PX;
  text(texts, {
    text: `${data.subtitle} · ${data.dateLabel}`,
    x: PAGE_PADDING_PX,
    y,
    fontPx: META_FONT_PX,
    weight: "400",
    color: "metaText",
  });
  y += META_LINE_PX + SECTION_GAP_PX;

  // Engineering summary: headline, basis, the voice profile, then the suggested focus — the same
  // three things the modal's summary strip shows, as a plain reading list rather than the
  // strip's two-column grid (a page has room to spell each line out in full).
  text(texts, {
    text: data.headline,
    x: PAGE_PADDING_PX,
    y,
    fontPx: HEADLINE_FONT_PX,
    weight: "700",
    color: "headlineText",
  });
  y += HEADLINE_LINE_PX;
  text(texts, { text: data.basis, x: PAGE_PADDING_PX, y, fontPx: BASIS_FONT_PX, weight: "400", color: "basisText" });
  y += BASIS_LINE_PX + 10;

  text(texts, {
    text: "Voice profile",
    x: PAGE_PADDING_PX,
    y,
    fontPx: HEADING_FONT_PX,
    weight: "700",
    color: "headingText",
  });
  y += HEADING_LINE_PX;
  for (const row of data.profile) {
    const line = `${row.label}: ${row.value || "—"} — ${row.reading}`;
    for (const wrappedLine of wrapped(line, contentWidth, BODY_FONT_PX)) {
      text(texts, { text: wrappedLine, x: PAGE_PADDING_PX, y, fontPx: BODY_FONT_PX, weight: "400", color: "bodyText" });
      y += BODY_LINE_PX;
    }
  }
  y += 10;

  text(texts, {
    text: "Suggested focus",
    x: PAGE_PADDING_PX,
    y,
    fontPx: HEADING_FONT_PX,
    weight: "700",
    color: "headingText",
  });
  y += HEADING_LINE_PX;
  for (const item of data.focus) {
    for (const wrappedLine of wrapped(`• ${item.text}`, contentWidth, BODY_FONT_PX)) {
      text(texts, { text: wrappedLine, x: PAGE_PADDING_PX, y, fontPx: BODY_FONT_PX, weight: "400", color: "bodyText" });
      y += BODY_LINE_PX;
    }
  }
  y += SECTION_GAP_PX;

  // The graph itself, scaled to the page width (usually 1:1 — the page is at least as wide as
  // the graph already was) with its annotation cards and leader lines carried over verbatim when
  // `showAnnotations` is on, in exactly the positions `ExplainGraph` placed them in.
  const graphScale = pageWidth / data.graph.widthPx;
  const graphImage: PlanGraphImage = {
    x: 0,
    y,
    width: pageWidth,
    height: data.graph.heightPx * graphScale,
  };
  const cards: PlanCard[] = [];
  const leaders: PlanLeader[] = [];
  if (data.showAnnotations) {
    for (const card of data.graph.cards) {
      const cardX = card.rect.x * graphScale;
      const cardY = y + card.rect.y * graphScale;
      const cardWidth = card.rect.width * graphScale;
      const textWidth = Math.max(10, cardWidth - CARD_PADDING_H_PX * 2 - CARD_STRIPE_PX);
      const lines: PlanText[] = [];
      let lineY = cardY + CARD_PADDING_V_PX + CARD_LINE_PX;
      text(lines, {
        text: card.title,
        x: cardX + CARD_STRIPE_PX + CARD_PADDING_H_PX,
        y: lineY,
        fontPx: CARD_FONT_PX,
        weight: "700",
        color: "cardText",
      });
      for (const wrappedLine of wrapped(card.measured, textWidth, CARD_FONT_PX)) {
        lineY += CARD_LINE_PX;
        text(lines, {
          text: wrappedLine,
          x: cardX + CARD_STRIPE_PX + CARD_PADDING_H_PX,
          y: lineY,
          fontPx: CARD_FONT_PX,
          weight: "400",
          color: "cardMetaText",
        });
      }
      cards.push({
        x: cardX,
        y: cardY,
        width: cardWidth,
        height: card.rect.height * graphScale,
        stripeColor: severityColorKey(card.severity),
        lines,
      });
    }
    for (const leader of data.graph.leaders) {
      leaders.push({
        from: { x: leader.from.x * graphScale, y: y + leader.from.y * graphScale },
        to: { x: leader.to.x * graphScale, y: y + leader.to.y * graphScale },
      });
    }
  }
  y += graphImage.height + SECTION_GAP_PX;

  // "Also measured": findings the graph itself had no room (or no anchor) for — the modal's own
  // beneath-the-graph cards, unchanged.
  if (data.beneath.length > 0) {
    text(texts, {
      text: "Also measured",
      x: PAGE_PADDING_PX,
      y,
      fontPx: HEADING_FONT_PX,
      weight: "700",
      color: "headingText",
    });
    y += HEADING_LINE_PX;
    for (const item of data.beneath) {
      text(texts, {
        text: item.title,
        x: PAGE_PADDING_PX,
        y,
        fontPx: BODY_FONT_PX,
        weight: "700",
        color: "bodyText",
      });
      y += BODY_LINE_PX;
      for (const wrappedLine of wrapped(item.measured, contentWidth, BODY_FONT_PX)) {
        text(texts, {
          text: wrappedLine,
          x: PAGE_PADDING_PX,
          y,
          fontPx: BODY_FONT_PX,
          weight: "400",
          color: "bodyText",
        });
        y += BODY_LINE_PX;
      }
      y += 6;
    }
  }

  y += PAGE_PADDING_PX - 6;

  return { widthPx: pageWidth, heightPx: Math.round(y), texts, cards, leaders, graphImage };
}

/** Draws {@link ExplainExportImagePlan} onto `ctx` at `scale` (device-pixel ratio-like — the
 * backing canvas must already be `plan.widthPx * scale` × `plan.heightPx * scale`, and `ctx`
 * un-scaled: this applies its own `scale(scale, scale)` so every coordinate above stays in plain
 * page pixels). `graphCanvas` is the live `ExplainGraph` canvas element — drawn at its full
 * intrinsic (already device-pixel-scaled) resolution into the plan's page-pixel destination rect,
 * so the raster never loses resolution regardless of what DPR it was drawn at. */
export function drawExplainExportImage(
  ctx: DrawCtx2D,
  plan: ExplainExportImagePlan,
  graphCanvas: CanvasImageSource & { width: number; height: number },
  palette: ExportPalette,
  scale = 1,
): void {
  ctx.save();
  ctx.fillStyle = palette.background;
  ctx.fillRect(0, 0, plan.widthPx * scale, plan.heightPx * scale);
  // Everything from here draws in plain page pixels — the backing canvas is `scale`-times larger
  // (the caller sized it that way), so the transform below is what stretches it to fill it.
  ctx.scale(scale, scale);

  ctx.drawImage(
    graphCanvas,
    0,
    0,
    graphCanvas.width,
    graphCanvas.height,
    plan.graphImage.x,
    plan.graphImage.y,
    plan.graphImage.width,
    plan.graphImage.height,
  );

  ctx.strokeStyle = palette.leaderLine;
  ctx.lineWidth = 1;
  for (const leader of plan.leaders) {
    ctx.beginPath();
    ctx.moveTo(leader.from.x, leader.from.y);
    ctx.lineTo(leader.to.x, leader.to.y);
    ctx.stroke();
  }

  for (const card of plan.cards) {
    ctx.fillStyle = palette.cardBg;
    ctx.fillRect(card.x, card.y, card.width, card.height);
    ctx.fillStyle = palette[card.stripeColor];
    ctx.fillRect(card.x, card.y, CARD_STRIPE_PX, card.height);
    for (const line of card.lines) {
      ctx.font = `${line.weight} ${line.fontPx}px ${FONT_FAMILY}`;
      ctx.fillStyle = palette[line.color];
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      ctx.fillText(line.text, line.x, line.y);
    }
  }

  for (const run of plan.texts) {
    ctx.font = `${run.weight} ${run.fontPx}px ${FONT_FAMILY}`;
    ctx.fillStyle = palette[run.color];
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    ctx.fillText(run.text, run.x, run.y);
  }

  ctx.restore();
}

/** The current theme's colours for the export — every value comes from `themeColors.ts`'s
 * `themeTokenValue` (`getComputedStyle` on the root, falling back to the parsed
 * `design-tokens.css` text when a stylesheet isn't loaded, e.g. jsdom in tests). Nothing here is
 * a colour literal (T-708): {@link EXPORT_PALETTE_TOKENS} names properties, never values. */
export function resolveExportPalette(): ExportPalette {
  const palette = {} as ExportPalette;
  for (const key of Object.keys(EXPORT_PALETTE_TOKENS) as ExportColorKey[]) {
    palette[key] = themeTokenValue(EXPORT_PALETTE_TOKENS[key]);
  }
  return palette;
}

/** `EXPORT_SCALE`-times the plan's page pixels — 2x reads comfortably on a printed page (roughly
 * 150 dpi at the modal's usual on-screen size) without the file size of a much higher factor. */
export const EXPORT_SCALE = 2;

/** Renders {@link ExplainExportData} to a PNG `Blob`, at {@link EXPORT_SCALE}. The only part of
 * this module not covered by a unit test (jsdom has no real 2D context, MEMORY.md) — verified by
 * opening the file after a real export instead (ticket's own verification step). */
export async function renderExplainExportPng(data: ExplainExportData): Promise<Blob> {
  const plan = measureExplainExportLayout(data);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(plan.widthPx * EXPORT_SCALE));
  canvas.height = Math.max(1, Math.round(plan.heightPx * EXPORT_SCALE));
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("explain export: no 2D canvas context available");
  }
  drawExplainExportImage(ctx, plan, data.graph.canvas, resolveExportPalette(), EXPORT_SCALE);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) {
    throw new Error("explain export: canvas.toBlob returned null");
  }
  return blob;
}
