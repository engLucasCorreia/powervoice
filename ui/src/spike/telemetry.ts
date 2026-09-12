import { Channel, invoke } from "@tauri-apps/api/core";
import { decodeTelemetryFrame, describePayloadType } from "./binary";
import type { TelemetryResult } from "./types";

/** Measures the display's actual rAF rate over an idle window, so "dropped frames" is judged
 * against reality rather than an assumed 60 Hz (Hyprland can throttle — or, observed running this
 * spike unfocused, entirely withhold — rAF for a window that isn't visible/focused; see
 * `frameBench.ts` / ADR-009). Races against a plain `setTimeout` watchdog so a rAF that never
 * fires at all still resolves (as `0` Hz) instead of hanging the whole suite. */
export function measureBaselineRafHz(durationMs: number): Promise<number> {
  return new Promise((resolve) => {
    let count = 0;
    let start = -1;
    let settled = false;
    const watchdog = setTimeout(() => {
      if (settled) return;
      settled = true;
      resolve(start < 0 ? 0 : count / ((performance.now() - start) / 1000));
    }, durationMs + 4_000);
    function tick(now: number) {
      if (settled) return;
      if (start < 0) start = now;
      count++;
      if (now - start >= durationMs) {
        settled = true;
        clearTimeout(watchdog);
        resolve(count / ((now - start) / 1000));
        return;
      }
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
  });
}

/** MEMORY.md / ADR-003 follow-up (b): cost of a 72-byte `VXTM` frame over a `Channel` at `hz`,
 * reported as JS-side handling time per frame and `requestAnimationFrame` drops observed during
 * the same window (i.e. does receiving telemetry at this rate visibly compete with rendering). */
export async function measureTelemetry(
  hz: number,
  durationMs: number,
  baselineRafHz: number,
): Promise<TelemetryResult> {
  let framesReceived = 0;
  let totalHandlerUs = 0;
  let maxHandlerUs = 0;
  let payloadType = "";

  const channel = new Channel<ArrayBuffer>((message) => {
    const hStart = performance.now();
    payloadType = describePayloadType(message);
    decodeTelemetryFrame(message);
    const handlerUs = (performance.now() - hStart) * 1000;
    totalHandlerUs += handlerUs;
    if (handlerUs > maxHandlerUs) maxHandlerUs = handlerUs;
    framesReceived++;
  });

  let rafObserved = 0;
  let rafRunning = true;
  const rafLoop = () => {
    if (!rafRunning) return;
    rafObserved++;
    requestAnimationFrame(rafLoop);
  };
  requestAnimationFrame(rafLoop);

  const start = performance.now();
  await invoke("spike_telemetry_run", { channel, hz, durationMs });
  // The command spawns a background thread and returns immediately; wait out the window plus a
  // little slack so the last frame(s) have time to arrive before we stop counting.
  await new Promise((resolve) => setTimeout(resolve, durationMs + 250));
  rafRunning = false;
  const elapsedS = (performance.now() - start) / 1000;

  const expectedFrames = Math.round((durationMs / 1000) * hz);
  const rafFramesExpected = Math.round(elapsedS * baselineRafHz);

  return {
    hz,
    requestedDurationMs: durationMs,
    framesReceived,
    expectedFrames,
    avgHandlerTimeUs: framesReceived > 0 ? totalHandlerUs / framesReceived : 0,
    maxHandlerTimeUs: maxHandlerUs,
    baselineRafHz,
    rafFramesExpected,
    rafFramesObserved: rafObserved,
    rafFramesDropped: Math.max(0, rafFramesExpected - rafObserved),
    payloadType,
  };
}

export async function runTelemetryBenches(durationMs: number): Promise<TelemetryResult[]> {
  const baselineRafHz = await measureBaselineRafHz(1000);
  const results: TelemetryResult[] = [];
  for (const hz of [30, 60]) {
    // eslint-disable-next-line no-await-in-loop -- intentionally sequential, one rate at a time
    results.push(await measureTelemetry(hz, durationMs, baselineRafHz));
  }
  return results;
}
