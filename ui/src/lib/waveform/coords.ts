/**
 * Waveform view pixel ↔ document sample mapping and zoom math (SPEC-006 §4.1, §2.3, §2.6). Pure
 * functions, shared by mouse hit-testing, ruler tick generation and rendering (§4.1: "all three
 * call sites share one implementation").
 */

/** ADR-003 §2's pyramid levels, finest first (also `vox_project::store::PEAK_LEVELS_SPP`). */
export const PEAK_LEVELS_SPP = [64, 256, 1024, 4096, 16_384, 65_536] as const;

/** Below this many samples/pixel the view requests raw samples instead of a pyramid level. */
export const RAW_SPP = 1;

/** SPEC-006 §2.6: "1 sample per 10 px" is the maximum zoom. */
export const MIN_SAMPLES_PER_PIXEL = 0.1;

/** SPEC-006 §2.6: `√2` per keyboard zoom step (doubling every two presses). */
export const ZOOM_STEP_FACTOR = Math.SQRT2;

/** SPEC-006 §2.3: raw-sample dots appear once consecutive samples are this many px apart. */
export const DOT_THRESHOLD_PX_PER_SAMPLE = 3;

/**
 * `sample(px)` (SPEC-006 §4.1): the document sample under device-pixel `px` of a canvas whose
 * left edge is `startSample`, at `samplesPerPixel` zoom.
 */
export function sampleAtPixel(px: number, startSample: number, samplesPerPixel: number): number {
  return startSample + Math.round(px * samplesPerPixel);
}

/** `px(sample)` (SPEC-006 §4.1): the inverse of {@link sampleAtPixel}. */
export function pixelAtSample(sample: number, startSample: number, samplesPerPixel: number): number {
  return Math.round((sample - startSample) / samplesPerPixel);
}

/**
 * The pyramid level to request at `samplesPerPixel` (SPEC-006 §2.3, AC-1; ADR-003 §2): the
 * largest level `≤ samplesPerPixel`, or {@link RAW_SPP} below the finest level. Reducing that
 * level's buckets into one pixel column never combines more than 4 buckets (levels step ×4).
 */
export function pickLevel(samplesPerPixel: number): number {
  if (samplesPerPixel < PEAK_LEVELS_SPP[0]) {
    return RAW_SPP;
  }
  let chosen: number = PEAK_LEVELS_SPP[0];
  for (const level of PEAK_LEVELS_SPP) {
    if (level <= samplesPerPixel) {
      chosen = level;
    }
  }
  return chosen;
}

/** `true` when raw-sample dots should be drawn at `samplesPerPixel` (SPEC-006 §2.3, AC-3). */
export function showsDots(samplesPerPixel: number): boolean {
  return 1 / samplesPerPixel >= DOT_THRESHOLD_PX_PER_SAMPLE;
}

/** `samplesPerPixel` for "zoom full": the whole document exactly fills `viewportPx`. */
export function zoomFullSamplesPerPixel(lenSamples: number, viewportPx: number): number {
  return viewportPx > 0 ? lenSamples / viewportPx : MIN_SAMPLES_PER_PIXEL;
}

/** Clamps `spp` into the valid SPEC-006 §2.6 range, `[0.1, zoom-full]`. */
export function clampSamplesPerPixel(spp: number, lenSamples: number, viewportPx: number): number {
  const max = Math.max(zoomFullSamplesPerPixel(lenSamples, viewportPx), MIN_SAMPLES_PER_PIXEL);
  return Math.min(Math.max(spp, MIN_SAMPLES_PER_PIXEL), max);
}

/**
 * The next `samplesPerPixel` after one keyboard zoom step (SPEC-006 §2.6: `√2` per press),
 * clamped to `[0.1, zoom-full]`. `direction`: `1` zooms in (smaller spp), `-1` zooms out.
 */
export function zoomStep(
  current: number,
  direction: 1 | -1,
  lenSamples: number,
  viewportPx: number,
): number {
  const next = direction === 1 ? current / ZOOM_STEP_FACTOR : current * ZOOM_STEP_FACTOR;
  return clampSamplesPerPixel(next, lenSamples, viewportPx);
}

/**
 * Zooms `samplesPerPixel` from `current` to `next` while keeping `anchorSample` under the same
 * pixel `anchorPx` (SPEC-006 §2.6: cursor/playhead-centred zoom). Returns the new `startSample`.
 */
export function zoomAroundSample(
  anchorSample: number,
  anchorPx: number,
  nextSamplesPerPixel: number,
): number {
  return Math.round(anchorSample - anchorPx * nextSamplesPerPixel);
}

/** One viewport write: the new `startSample`/`samplesPerPixel` pair a zoom command produces. */
export interface ZoomedViewport {
  startSample: number;
  samplesPerPixel: number;
}

/**
 * "Zoom to selection" (SPEC-006 §2.6, H-35): the spec's own exact formula — `startSample =
 * selection.startSample`, `samplesPerPixel = (endSample − startSample) / viewportPx`, clamped to
 * the §2.6 `[0.1, zoom-full]` range (no extra margin either side — the spec is explicit about
 * this formula, so there's nothing to invent). `startSample` is then run through
 * {@link clampStartSample} too, matching every other viewport write in this module — a no-op in
 * the exact case (the fitted viewport already covers exactly the selection), but a guard if
 * clamping `samplesPerPixel` changed the viewport width (e.g. a selection shorter than the
 * `MIN_SAMPLES_PER_PIXEL` floor allows).
 *
 * `null` (no selection, an empty selection, or no known viewport width) is a no-op per SPEC-006
 * §2.6: "a no-op (shows nothing, no error) when there's no selection" — the caller does nothing
 * with a `null` result rather than applying a meaningless viewport.
 */
export function zoomToSelectionViewport(
  selection: { startSample: number; endSample: number } | null,
  lenSamples: number,
  viewportPx: number,
): ZoomedViewport | null {
  if (!selection || selection.endSample <= selection.startSample || viewportPx <= 0) {
    return null;
  }
  const samplesPerPixel = clampSamplesPerPixel(
    (selection.endSample - selection.startSample) / viewportPx,
    lenSamples,
    viewportPx,
  );
  const startSample = clampStartSample(selection.startSample, samplesPerPixel, lenSamples, viewportPx);
  return { startSample, samplesPerPixel };
}

/** "Zoom full" (SPEC-006 §2.6): "sets `samplesPerPixel = len_samples / viewportPx`, `startSample
 * = 0`" — the whole document exactly fills the viewport. */
export function zoomFullViewport(lenSamples: number, viewportPx: number): ZoomedViewport {
  return { startSample: 0, samplesPerPixel: zoomFullSamplesPerPixel(lenSamples, viewportPx) };
}

/** Clamps `startSample` so the viewport never shows before 0 or (when it fits) past the end. */
export function clampStartSample(
  startSample: number,
  samplesPerPixel: number,
  lenSamples: number,
  viewportPx: number,
): number {
  const viewportSamples = samplesPerPixel * viewportPx;
  const maxStart = Math.max(0, lenSamples - viewportSamples);
  return Math.min(Math.max(startSample, 0), Math.max(maxStart, 0));
}

/** One "nice" tick step from the `{1, 2, 5} × 10ⁿ` ladder (SPEC-006 §4.2). */
const NICE_STEPS = [1, 2, 5] as const;

/**
 * The largest step from the `{1, 2, 5} × 10ⁿ` ladder such that `minGap` separates consecutive
 * ticks (SPEC-006 §4.2) — unit-agnostic: seconds for {@link niceTickStepSeconds}
 * (`timecode`/`seconds` ruler formats, both a seconds-based ladder per §4.2), samples for
 * {@link sampleTicks} (the `samples` format's own ladder).
 */
function niceStep(minGap: number): number {
  if (minGap <= 0) {
    return NICE_STEPS[0];
  }
  const exponent = Math.floor(Math.log10(minGap));
  for (let e = exponent - 1; e <= exponent + 1; e++) {
    for (const base of NICE_STEPS) {
      const step = base * 10 ** e;
      if (step >= minGap) {
        return step;
      }
    }
  }
  return NICE_STEPS[0] * 10 ** (exponent + 2);
}

/**
 * The largest step from the `{1, 2, 5} × 10ⁿ` ladder such that `minGapSeconds` of document time
 * separates consecutive ticks (SPEC-006 §4.2). `minGapSeconds` is derived by the caller from a
 * minimum on-screen pixel gap and the current `samplesPerPixel`/`sampleRateHz`.
 */
export function niceTickStepSeconds(minGapSeconds: number): number {
  return niceStep(minGapSeconds);
}

/**
 * Reduces pyramid buckets (already fetched, contiguous, aligned to `level` starting at
 * `bucketsStartSample`) into one `(min, max)` per pixel column covering
 * `[startSample, startSample + viewportPx * samplesPerPixel)` (SPEC-006 §4.3, AC-1/AC-2: never
 * more than 4 buckets combined per column, since levels step ×4; the union never under-reports
 * amplitude). A column with no covering (fetched) bucket is `null` (SPEC-006 §2.3: `NaN` buckets
 * — not produced by this ticket's server, ADR-004 §5 — are skipped the same way).
 */
export function reduceColumns(
  buckets: ReadonlyArray<readonly [number, number]>,
  bucketsStartSample: number,
  level: number,
  startSample: number,
  samplesPerPixel: number,
  viewportPx: number,
): Array<[number, number] | null> {
  const out: Array<[number, number] | null> = new Array(viewportPx);
  for (let px = 0; px < viewportPx; px++) {
    const lo = startSample + px * samplesPerPixel;
    const hi = startSample + (px + 1) * samplesPerPixel;
    const i0 = Math.max(0, Math.floor((lo - bucketsStartSample) / level));
    const i1 = Math.floor((hi - bucketsStartSample - 1) / level);
    let mn = Number.POSITIVE_INFINITY;
    let mx = Number.NEGATIVE_INFINITY;
    let any = false;
    for (let i = i0; i <= i1 && i < buckets.length; i++) {
      const bucket = buckets[i];
      if (!bucket) {
        continue;
      }
      const [bmn, bmx] = bucket;
      if (Number.isNaN(bmn) || Number.isNaN(bmx)) {
        continue;
      }
      mn = Math.min(mn, bmn);
      mx = Math.max(mx, bmx);
      any = true;
    }
    out[px] = any ? [mn, mx] : null;
  }
  return out;
}

/**
 * The device-pixel `[top, bottom)` span of one min/max column (SPEC-006 §2.3), given its
 * `(min, max)` amplitude pair (`-1..1`) and the vertical center `centerY`: `1.0` is the top of the
 * pane, `-1.0` the bottom. At least 1 px tall, so a silent column still draws a visible line
 * (H-13: the one implementation both the Canvas2D and WebGL2 renderers call, so they always agree
 * pixel-for-pixel).
 *
 * `verticalZoom` (H-35, SPEC-006 §2.4: `y = centerY − amplitude × verticalZoom × halfHeightPx`,
 * and `centerY` doubles as `halfHeightPx` here) defaults to `1` (unscaled), so every existing
 * caller keeps its old output.
 */
export function columnYRange(
  min: number,
  max: number,
  centerY: number,
  verticalZoom = 1,
): [top: number, bottom: number] {
  const top = centerY - max * verticalZoom * centerY;
  const bottom = centerY - min * verticalZoom * centerY;
  return [top, Math.max(top + 1, bottom)];
}

export interface TimeTick {
  /** Exact document sample this tick sits at (SPEC-006 AC-6: never rounded twice). */
  sample: number;
  /** Document seconds (for label formatting). */
  seconds: number;
}

/**
 * Time-ruler tick positions for the visible range (SPEC-006 §2.5, §4.2): the `timecode`/
 * `seconds` formats' shared seconds-based ladder (the `samples` format uses its own ladder
 * directly in sample space, {@link sampleTicks}). Each tick's `sample` is
 * `round(seconds × sampleRateHz)`, and every label is generated from that same sample via the
 * shared {@link pixelAtSample} — never rounded independently (AC-6).
 */
export function timeTicks(
  startSample: number,
  samplesPerPixel: number,
  viewportPx: number,
  sampleRateHz: number,
  minLabelGapPx: number,
): TimeTick[] {
  if (sampleRateHz <= 0 || samplesPerPixel <= 0 || viewportPx <= 0) {
    return [];
  }
  const minGapSeconds = (minLabelGapPx * samplesPerPixel) / sampleRateHz;
  const step = niceTickStepSeconds(minGapSeconds);
  const startSeconds = startSample / sampleRateHz;
  const endSeconds = (startSample + viewportPx * samplesPerPixel) / sampleRateHz;
  const firstTick = Math.floor(startSeconds / step) * step;
  const ticks: TimeTick[] = [];
  // Guard against a pathological step of 0 looping forever (can't happen given NICE_STEPS'
  // smallest base is 1, but keeps this function total under any future refactor).
  const maxTicks = 10_000;
  for (let i = 0, seconds = firstTick; seconds <= endSeconds + step && i < maxTicks; i++, seconds += step) {
    if (seconds < 0) {
      continue;
    }
    ticks.push({ sample: Math.round(seconds * sampleRateHz), seconds });
  }
  return ticks;
}

/**
 * Time-ruler tick positions for the `samples` format (SPEC-006 §2.5/§4.2: "1/2/5×10ⁿ samples for
 * `samples`") — a ladder directly in sample space, so a tick's position is never a rounded
 * seconds-domain value: it's exactly `k × step` for an integer sample step, needing no conversion
 * through `sampleRateHz` at all (AC-6's exactness for free).
 */
export function sampleTicks(
  startSample: number,
  samplesPerPixel: number,
  viewportPx: number,
  minLabelGapPx: number,
): number[] {
  if (samplesPerPixel <= 0 || viewportPx <= 0) {
    return [];
  }
  const minGapSamples = minLabelGapPx * samplesPerPixel;
  // Sample positions are integers; the {1,2,5}×10ⁿ ladder can fall below 1 once zoomed in past
  // 1 sample/px, so the step is floored at 1 sample.
  const step = Math.max(1, Math.round(niceStep(minGapSamples)));
  const endSample = startSample + viewportPx * samplesPerPixel;
  const firstTick = Math.floor(startSample / step) * step;
  const ticks: number[] = [];
  const maxTicks = 10_000;
  for (let i = 0, sample = firstTick; sample <= endSample + step && i < maxTicks; i++, sample += step) {
    if (sample < 0) {
      continue;
    }
    ticks.push(sample);
  }
  return ticks;
}
