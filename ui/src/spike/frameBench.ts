import type { FrameStats, VisibilitySample } from "./types";

/** ~1.5x a 60fps frame budget (PROMPT §4 target: 60 fps scroll/zoom) — a frame slower than this
 * counts as "dropped" for these stats. */
const DROP_THRESHOLD_MS = 25;

/** How much longer than the requested duration we'll wait for rAF before giving up and reporting
 * whatever we got (possibly zero frames) as a timed-out/throttled result. Hyprland can withhold
 * rAF entirely for a mapped-but-unfocused window (observed running this spike headlessly) — see
 * ADR-009 — so this must not depend on rAF ever firing. */
const WATCHDOG_GRACE_MS = 4_000;

export function sampleVisibility(): VisibilitySample {
  return {
    visibilityState: document.visibilityState,
    hasFocus: document.hasFocus(),
    timestamp: performance.now(),
  };
}

/**
 * Drives `draw(progress)` from `requestAnimationFrame` for `durationMs` (`progress` runs 0..1
 * across the run — used for the scripted zoom sweep) and returns frame-time percentiles/dropped
 * frames. Records `document.visibilityState`/focus at start and end, and — critically — races the
 * rAF loop against a plain `setTimeout` watchdog (`durationMs + WATCHDOG_GRACE_MS`) that does NOT
 * depend on rAF firing at all, so a throttled/withheld rAF (window not visible/focused) always
 * still resolves with `timedOut: true` and whatever frames were captured, rather than hanging.
 */
export function runFrameBench(
  renderer: string,
  durationMs: number,
  draw: (progress: number) => void,
): Promise<FrameStats> {
  return new Promise((resolve) => {
    const deltas: number[] = [];
    const startVisibility = sampleVisibility();
    let start = -1;
    let last = -1;
    let settled = false;
    let rafHandle = 0;

    const watchdog = setTimeout(() => {
      finish(start >= 0 ? performance.now() - start : 0, true);
    }, durationMs + WATCHDOG_GRACE_MS);

    function frame(now: number): void {
      if (settled) return;
      if (start < 0) {
        start = now;
        last = now;
        draw(0);
        rafHandle = requestAnimationFrame(frame);
        return;
      }
      deltas.push(now - last);
      last = now;
      const elapsed = now - start;
      if (elapsed >= durationMs) {
        finish(elapsed, false);
        return;
      }
      draw(elapsed / durationMs);
      rafHandle = requestAnimationFrame(frame);
    }

    function finish(actualDurationMs: number, timedOut: boolean): void {
      if (settled) return;
      settled = true;
      clearTimeout(watchdog);
      cancelAnimationFrame(rafHandle);
      const endVisibility = sampleVisibility();
      const samples = [...deltas].sort((a, b) => a - b);
      const percentile = (p: number): number => {
        if (samples.length === 0) return 0;
        const idx = Math.min(samples.length - 1, Math.floor(samples.length * p));
        return samples[idx] ?? 0;
      };
      const droppedFrames = samples.filter((d) => d > DROP_THRESHOLD_MS).length;
      // Judged on measured behaviour, not just focus/visibility: an unfocused-but-visible window
      // was observed (ADR-009) to still get a healthy rAF rate on this Hyprland setup, so focus
      // loss alone would be a misleading "throttled" label on an otherwise-clean run. A window
      // that's actually hidden/minimized (visibilityState !== "visible"), or delivered far fewer
      // frames than a 60 Hz display would in this long, or ran slower than the drop threshold at
      // the median, is the real signal.
      const expectedFramesAt60Hz = actualDurationMs / (1000 / 60);
      const likelyThrottled =
        timedOut ||
        samples.length === 0 ||
        startVisibility.visibilityState !== "visible" ||
        endVisibility.visibilityState !== "visible" ||
        samples.length < expectedFramesAt60Hz * 0.5 ||
        percentile(0.5) > DROP_THRESHOLD_MS;
      resolve({
        renderer,
        frames: samples.length,
        p50Ms: percentile(0.5),
        p95Ms: percentile(0.95),
        p99Ms: percentile(0.99),
        maxMs: samples[samples.length - 1] ?? 0,
        droppedFrames,
        dropThresholdMs: DROP_THRESHOLD_MS,
        durationMs: actualDurationMs,
        visibility: { start: startVisibility, end: endVisibility },
        likelyThrottled,
        timedOut,
      });
    }

    rafHandle = requestAnimationFrame(frame);
  });
}
