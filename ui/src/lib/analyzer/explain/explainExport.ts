/**
 * Data shaping for H-115's "Explain My Voice" export: the plain-object shape both the exported
 * image (`explainExportImage.ts`) and the exported HTML report (`explainExportHtml.ts`) draw
 * from, built once from exactly what is already on screen — H-91's frozen snapshot, H-94's
 * summary and prose, and `ExplainGraph`'s own `exportFrame` (the live canvas plus its current
 * placed-card layout, H-115's addition to that component).
 *
 * Nothing here re-measures or re-classifies anything: the export renders the same words and the
 * same curve the reader is already looking at, at the toggle state they currently have set
 * (`showAnnotations`/`showEqAdvice`), never a recomputation of its own.
 */
import { t } from "../../i18n";
import type { Rect } from "../../ui/axisLabels";
import type { FindingId, FindingSeverity, VoiceFinding } from "./findings";
import type { FindingProse } from "./prose";
import type { VoiceSnapshot } from "./snapshot";
import type { FocusItem, ProfileEntry, VoiceSummary } from "./summary";

/** One placed annotation card, in the graph's own pixel space (H-93/H-102's layout, unchanged
 * here) — enough to redraw the box and its leader without touching Svelte or the DOM. */
export interface ExplainExportCard {
  rect: Rect;
  title: string;
  measured: string;
  severity: FindingSeverity;
}

export interface ExplainExportLeader {
  from: { x: number; y: number };
  to: { x: number; y: number };
}

/** `ExplainGraph.svelte`'s bound-out snapshot of what it is currently showing (H-115). `null`
 * until the canvas has a real size (mirrors the component's own `plot`/`layout` guards). */
export interface ExplainGraphExportFrame {
  /** The live canvas element. Read at export time, not copied here — by then it holds whatever
   * the current toggles last drew, same as what the reader sees. */
  canvas: HTMLCanvasElement;
  /** The canvas's CSS-pixel size (`ExplainGraph`'s `width`/`height`), which is also the
   * coordinate space `plot`, `cards[].rect` and `leaders[]` are given in. */
  widthPx: number;
  heightPx: number;
  plot: Rect;
  cards: ExplainExportCard[];
  leaders: ExplainExportLeader[];
}

/** One "also measured" finding that had no room (or no anchor) on the graph — the modal's own
 * beneath-the-graph cards, carried into the export unchanged. */
export interface ExplainExportBeneathCard {
  title: string;
  measured: string;
}

/** Everything the exported image and the exported report are built from. */
export interface ExplainExportData {
  title: string;
  /** "{n} s of audio analysed" (H-92's subtitle, unchanged). */
  subtitle: string;
  /** The take's duration, spelled out plainly for the report (SPEC's "the take's duration"). */
  durationLabel: string;
  /** When the analysis was taken (`VoiceSnapshot.takenAtMs`) — the raw value (for the suggested
   * export file name) and the same formatted for a reader (`dateLabel`). */
  takenAtMs: number;
  dateLabel: string;
  headline: string;
  basis: string;
  profile: ProfileEntry[];
  focus: FocusItem[];
  /** Every finding's prose, in H-91's priority order — the report's table is built from this,
   * regardless of whether that finding had room on the graph itself. */
  findings: FindingProse[];
  showAnnotations: boolean;
  showEqAdvice: boolean;
  graph: ExplainGraphExportFrame;
  beneath: ExplainExportBeneathCard[];
}

/** A plain-language date/time for the report — `toLocaleString`, like `PreferencesDialog.svelte`'s
 * offset dates and `punch.ts`, not a raw timestamp. */
export function explainExportDateLabel(takenAtMs: number): string {
  return new Date(takenAtMs).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function findingProseById(prose: readonly FindingProse[], id: FindingId): FindingProse | null {
  return prose.find((p) => p.id === id) ?? null;
}

export interface BuildExplainExportDataArgs {
  snapshot: VoiceSnapshot;
  summary: VoiceSummary;
  prose: readonly FindingProse[];
  subtitle: string;
  graph: ExplainGraphExportFrame;
  showAnnotations: boolean;
  showEqAdvice: boolean;
  /** The modal's current "also measured" list (`ExplainGraph`'s bound-out `beneath`). */
  beneathFindings: readonly VoiceFinding[];
}

/** Assembles {@link ExplainExportData} from exactly what the modal already has in hand — no
 * finding is re-evaluated, no number is reformatted differently than the panel already shows it. */
export function buildExplainExportData(args: BuildExplainExportDataArgs): ExplainExportData {
  const beneath: ExplainExportBeneathCard[] = [];
  for (const finding of args.beneathFindings) {
    const prose = findingProseById(args.prose, finding.id);
    if (prose) {
      beneath.push({ title: prose.title, measured: prose.measured });
    }
  }
  return {
    title: t("explain.title"),
    subtitle: args.subtitle,
    durationLabel: t("explain.export.duration", { seconds: args.snapshot.spanS.toFixed(1) }),
    takenAtMs: args.snapshot.takenAtMs,
    dateLabel: explainExportDateLabel(args.snapshot.takenAtMs),
    headline: args.summary.headline,
    basis: args.summary.basis,
    profile: args.summary.profile,
    focus: args.summary.focus,
    findings: [...args.prose],
    showAnnotations: args.showAnnotations,
    showEqAdvice: args.showEqAdvice,
    graph: args.graph,
    beneath,
  };
}

/** The suggested save-dialog file name (no extension) — stable across a session's exports so a
 * PNG and its matching HTML report land side by side under the same base name. */
export function explainExportFileBase(data: Pick<ExplainExportData, "takenAtMs">): string {
  const iso = new Date(data.takenAtMs).toISOString();
  return `voice-spectrum-analysis-${iso.slice(0, 10)}`;
}
