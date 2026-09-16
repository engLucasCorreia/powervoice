/**
 * Compact, zoom-adaptive time-ruler labels (H-24 item 7: "the time ruler adapts to zoom with
 * 'nice' steps and compact labels (e.g. `0:05`, `0:05.250`, `1:02:03`) plus minor ticks"). This is
 * a *ruler label* format, distinct from `transport/playhead.ts::formatTime` (the always-full
 * `hh:mm:ss.mmm` transport readout, still used verbatim elsewhere) — the ruler needs a label whose
 * precision tracks the current tick step (`coords.ts::niceTickStepSeconds`) instead of always
 * showing milliseconds, so ticks stay short at every zoom level instead of overflowing into each
 * other or wasting width on trailing zeros a 1-second-spaced tick ladder can't need.
 */

function pad(n: number, width = 2): string {
  return String(n).padStart(width, "0");
}

/**
 * Decimal places a tick label needs for step `stepSeconds` — `0` at a 1-second-or-coarser step,
 * otherwise enough decimals that two ticks `stepSeconds` apart always print visibly different
 * labels, e.g. `0:05.250` for a 250 ms step.
 *
 * H-60 (SPEC-006 §2.5/§4.2, extreme zoom): a fixed 3-decimal (millisecond) precision was exact
 * only down to a 1 ms step. At the documented zoom ceiling (`samplesPerPixel` down to 0.1,
 * SPEC-006 §2.6 AC-4) the `{1,2,5}×10ⁿ` tick ladder (`coords.ts::niceTickStepSeconds`) can pick a
 * step well under 1 ms — e.g. 0.2 ms at 48 kHz with a 70 px label gap — so consecutive ticks
 * rounded to 3 decimals collided on an identical label (several ticks in a row reading
 * "0:00.001"), silently violating §4.2's "consecutive labels are >= some minimum pixel gap"
 * requirement (a pixel gap is only useful if the label text at that gap actually differs). Floors
 * at 3 (unchanged for every step >= 1 ms, matching `transport/playhead.ts::formatTime`'s usual
 * precision) and caps at 6: even at the fastest accepted document rate (384 kHz,
 * `doc_rate_range_hz`, SPEC-005 §3) and the 0.1 samples/px zoom floor, the ladder's step never
 * needs more than 5.
 */
function decimalsForStep(stepSeconds: number): number {
  if (stepSeconds >= 1) {
    return 0;
  }
  return Math.min(6, Math.max(3, Math.ceil(-Math.log10(stepSeconds))));
}

/**
 * Formats `seconds` for the time ruler at tick spacing `stepSeconds` (H-24 item 7): `[h:]m:ss`
 * when the step is `>= 1` second, `[h:]m:ss.fff` (as many decimals as the step needs) below that.
 * `includeHours` (the document is `>= 1` hour, SPEC-006 §2.5's own `hh:` rule) adds the hour group
 * even at `0`, so every label in one ruler has the same shape.
 */
export function formatRulerTime(seconds: number, stepSeconds: number, includeHours: boolean): string {
  const clamped = Math.max(0, seconds);
  const decimals = decimalsForStep(stepSeconds);
  // Round at the label's own precision so e.g. 4.9996 at 3 decimals prints "5.000", not "4.999"
  // then carries wrongly into the integer part below.
  const scale = 10 ** decimals;
  const totalUnits = Math.round(clamped * scale);
  const wholeSeconds = Math.floor(totalUnits / scale);
  const frac = totalUnits - wholeSeconds * scale;

  const h = Math.floor(wholeSeconds / 3600);
  const m = Math.floor(wholeSeconds / 60) % 60;
  const s = wholeSeconds % 60;

  const hh = includeHours || h > 0 ? `${h}:` : "";
  const mm = includeHours || h > 0 ? pad(m) : String(m);
  const ss = pad(s);
  const base = `${hh}${mm}:${ss}`;
  if (decimals === 0) {
    return base;
  }
  return `${base}.${pad(frac, decimals)}`;
}

/**
 * Formats `seconds` for the time ruler's `seconds` format (SPEC-006 §2.5) at tick spacing
 * `stepSeconds`: a plain decimal, `0` decimals at a `>= 1` second step, milliseconds (same
 * precision rule as {@link formatRulerTime}) below that — so, unlike `timecode`, no `h:m:s`
 * grouping, just the number of seconds.
 */
export function formatRulerSeconds(seconds: number, stepSeconds: number): string {
  return Math.max(0, seconds).toFixed(decimalsForStep(stepSeconds));
}
