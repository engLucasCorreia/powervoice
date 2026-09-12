import { Channel, invoke } from "@tauri-apps/api/core";
import { decodeWaveformFrame, describePayloadType } from "./binary";
import { makeCanvas2dWaveformRenderer } from "./canvas2d";
import { runFrameBench } from "./frameBench";
import { reduceToColumns, zoomTriangle } from "./reduce";
import { withTimeout } from "./timeout";
import type { DrawColumns } from "./webgl";
import { makeWebgl2WaveformRenderer } from "./webgl";
import type { FrameStats, WaveformMeta } from "./types";

/** Ticket: "Automated zoom sweep (whole file → ~1 sample/px → back, 10 s)". */
const ZOOM_SWEEP_MS = 10_000;
/** Generous but bounded: the peaks payload is a few MB and generation is a one-off CPU loop, so
 * a healthy run finishes this in well under a second. */
const FETCH_TIMEOUT_MS = 15_000;

export async function fetchWaveformFrame(): Promise<{
  meta: WaveformMeta;
  buf: ArrayBuffer;
  payloadType: string;
}> {
  let resolveFrame: (buf: ArrayBuffer) => void = () => {};
  const framePromise = new Promise<ArrayBuffer>((resolve) => {
    resolveFrame = resolve;
  });
  const channel = new Channel<ArrayBuffer>((message) => resolveFrame(message));
  // The channel message may arrive before or after `invoke()` resolves; awaiting `framePromise`
  // afterwards is safe either way since a resolved promise stays resolved.
  const meta = await invoke<WaveformMeta>("spike_waveform_peaks", { channel });
  const buf = await withTimeout(framePromise, FETCH_TIMEOUT_MS, () => {
    throw new Error(`timed out waiting for the waveform Channel frame after ${FETCH_TIMEOUT_MS}ms`);
  });
  return { meta, buf, payloadType: describePayloadType(buf) };
}

export interface WaveformBenchResult {
  meta: WaveformMeta;
  channelPayloadType: string;
  webgl2: FrameStats | { error: string };
  canvas2d: FrameStats | { error: string };
}

async function bench(
  columnsWidth: number,
  count: number,
  minMax: Float32Array,
  rendererName: string,
  draw: DrawColumns,
): Promise<FrameStats | { error: string }> {
  const columns = new Float32Array(columnsWidth * 2);
  try {
    return await runFrameBench(rendererName, ZOOM_SWEEP_MS, (progress) => {
      const zoom = zoomTriangle(progress);
      const visibleBuckets = Math.max(columnsWidth, Math.round(count - zoom * (count - columnsWidth)));
      const startBucket = Math.round(
        (count - visibleBuckets) * ((Math.sin(progress * Math.PI * 2) + 1) / 2),
      );
      reduceToColumns(minMax, count, startBucket, visibleBuckets, columnsWidth, columns);
      draw(columns, columnsWidth);
    });
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * Fetches the synthetic waveform peaks once, then runs the WebGL2 and Canvas2D zoom-sweep
 * benches back to back against the same data (so any difference in the numbers is the renderer,
 * not the data or the zoom schedule).
 */
export async function runWaveformBenches(
  canvasWebgl2: HTMLCanvasElement,
  canvasCanvas2d: HTMLCanvasElement,
): Promise<WaveformBenchResult> {
  const { meta, buf, payloadType } = await fetchWaveformFrame();
  const frame = decodeWaveformFrame(buf);
  const columnsWidth = canvasWebgl2.width;

  let webgl2: FrameStats | { error: string };
  try {
    const draw = makeWebgl2WaveformRenderer(canvasWebgl2);
    webgl2 = await bench(columnsWidth, frame.count, frame.minMax, "webgl2-waveform", draw);
  } catch (e) {
    webgl2 = { error: e instanceof Error ? e.message : String(e) };
  }

  let canvas2d: FrameStats | { error: string };
  try {
    const draw = makeCanvas2dWaveformRenderer(canvasCanvas2d);
    canvas2d = await bench(canvasCanvas2d.width, frame.count, frame.minMax, "canvas2d-waveform", draw);
  } catch (e) {
    canvas2d = { error: e instanceof Error ? e.message : String(e) };
  }

  return { meta, channelPayloadType: payloadType, webgl2, canvas2d };
}
