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

/** Decimal places a tick label needs for step `stepSeconds` — `0` at a 1-second-or-coarser step,
 * milliseconds (matching `transport/playhead.ts::formatTime`'s own precision) below that, e.g.
 * `0:05.250` for a 250 ms step. */
function decimalsForStep(stepSeconds: number): number {
  return stepSeconds >= 1 ? 0 : 3;
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
