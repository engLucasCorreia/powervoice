/**
 * H-21 (SPEC-022 §2.11 "Waveform display"): how the waveform shows a running record operation.
 *
 * - The live take (`record_peaks_get`: the record window only, its sample 0 is the record point
 *   `at`, SPEC-022 §4.5) is drawn at `at` in the record colour.
 * - **Insert:** the existing waveform after `at` is drawn shifted right by the current take
 *   length (and so are the existing markers there).
 * - **Overwrite / Punch:** the take is drawn over the old audio; a punch also shows its region
 *   `[S, E)`.
 *
 * Pure functions (no canvas), shared by the Canvas2D and WebGL2 renderers of `WaveformView`.
 */

import type { MarkerDto, RecordPhaseDto, RecordStartedDto } from "../ipc/bindings";

export type Column = [number, number] | null;

export interface OpLayout {
  /** Insert: the audio after `at` moves right as the take grows. */
  insert: boolean;
  /** The record point `at` (the punch start `S`). */
  at: number;
  /** A punch's end `E` (`null`: open-ended cursor recording). */
  punchEnd: number | null;
  /** Record-window samples so far: the take drawn at `at` (0 during pre-roll). */
  takeLen: number;
}

/**
 * The layout of the running operation `op` (`null`: none, or a plain new recording), from its
 * latest phase and the window samples so far (`elapsedSamples` = heard position − `at`, clamped to
 * the punch length).
 */
export function opLayout(
  op: RecordStartedDto | null,
  phase: RecordPhaseDto | null,
  elapsedSamples: number,
): OpLayout | null {
  if (!op || op.op === "new") {
    return null;
  }
  const punchEnd = op.op === "punch" ? op.end_samples : null;
  const inPreRoll = phase === null ? op.aligned : phase.phase === "preroll";
  let takeLen = inPreRoll ? 0 : Math.max(0, Math.floor(elapsedSamples));
  if (punchEnd !== null) {
    takeLen = Math.min(takeLen, Math.max(0, punchEnd - op.at_samples));
  }
  return { insert: op.op === "insert", at: op.at_samples, punchEnd, takeLen };
}

/** Where existing document sample `q` is drawn (Insert shifts it right past `at`). */
export function displayOfDoc(layout: OpLayout, q: number): number {
  return layout.insert && q >= layout.at ? q + layout.takeLen : q;
}

/**
 * Per-pixel columns for the operation view:
 * - `base`: the existing audio — for Insert, from `docAt(startSample)` left of `at`, nothing over
 *   the take, and from `docAt(startSample − takeLen)` (shifted) after it; otherwise
 *   `docAt(startSample)` everywhere (the take is drawn on top);
 * - `take`: `takeCols` (the live take reduced with its start at display sample `at`), limited to
 *   pixels overlapping `[at, at + takeLen)`.
 *
 * `docAt(s)` returns the document's columns for a viewport whose first pixel starts at display
 * sample `s` (i.e. `reduceColumns(…, s, spp, widthPx)`).
 */
export function opColumns(
  layout: OpLayout,
  docAt: (startSample: number) => Column[],
  takeCols: Column[],
  startSample: number,
  samplesPerPixel: number,
  widthPx: number,
): { base: Column[]; take: Column[] } {
  const base = docAt(startSample);
  const shifted = layout.insert && layout.takeLen > 0 ? docAt(startSample - layout.takeLen) : base;
  const takeEnd = layout.at + layout.takeLen;
  const outBase: Column[] = new Array(widthPx).fill(null);
  const outTake: Column[] = new Array(widthPx).fill(null);
  for (let px = 0; px < widthPx; px++) {
    const lo = startSample + px * samplesPerPixel;
    const hi = lo + samplesPerPixel;
    const overlapsTake = layout.takeLen > 0 && hi > layout.at && lo < takeEnd;
    if (overlapsTake) {
      outTake[px] = takeCols[px] ?? null;
    }
    if (!layout.insert || lo < layout.at) {
      outBase[px] = base[px] ?? null;
    } else if (lo >= takeEnd) {
      outBase[px] = shifted[px] ?? null;
    }
  }
  return { base: outBase, take: outTake };
}

/**
 * The first document sample the waveform's `peaks_get` must cover: during an Insert whose
 * viewport starts past `at`, the shifted part shows document audio from `startSample − takeLen`
 * on (but never before `at`). `takeLen` is rounded up to a quarter of the viewport span so the
 * request changes only every few frames while the take grows, not on every telemetry frame.
 */
export function opPeaksRequestStart(
  layout: OpLayout | null,
  startSample: number,
  spanSamples: number,
): number {
  if (!layout || !layout.insert || startSample <= layout.at || layout.takeLen <= 0) {
    return startSample;
  }
  const step = Math.max(1, Math.floor(spanSamples / 4));
  const shift = Math.ceil(layout.takeLen / step) * step;
  return Math.max(layout.at, startSample - shift);
}

/**
 * Markers as drawn during the operation: for Insert, existing markers at or after `at` move
 * right by the take length (a region spanning `at` grows by it) — markers added during the
 * operation (`isTakeMarker`) are already in the result's coordinates and stay put.
 */
export function opMarkers(
  layout: OpLayout | null,
  markers: MarkerDto[],
  isTakeMarker: (id: number) => boolean,
): MarkerDto[] {
  if (!layout || !layout.insert || layout.takeLen <= 0) {
    return markers;
  }
  return markers.map((m) => {
    if (isTakeMarker(m.id)) {
      return m;
    }
    if (m.pos_samples >= layout.at) {
      return { ...m, pos_samples: m.pos_samples + layout.takeLen };
    }
    if (m.pos_samples + m.len_samples > layout.at) {
      return { ...m, len_samples: m.len_samples + layout.takeLen };
    }
    return m;
  });
}
