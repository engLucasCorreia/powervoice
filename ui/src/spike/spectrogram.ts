import { invoke } from "@tauri-apps/api/core";
import { decodeSpectrogramFrame, describePayloadType } from "./binary";
import { makeCanvas2dSpectrogramRenderer } from "./canvas2d";
import { runFrameBench } from "./frameBench";
import { withTimeout } from "./timeout";
import { makeWebgl2SpectrogramRenderer } from "./webgl";
import type { FrameStats } from "./types";

/** Same 10 s budget as the waveform zoom sweep, for a comparable measurement window. */
const SCROLL_BENCH_MS = 10_000;
/** Columns pushed per frame, i.e. simulated STFT hops arriving per animation frame. */
const COLUMNS_PER_FRAME = 4;
/** The texture is ~512KB and generation is a one-off CPU loop; a healthy fetch is near-instant. */
const FETCH_TIMEOUT_MS = 15_000;

export async function fetchSpectrogramFrame(): Promise<{
  width: number;
  height: number;
  pixels: Uint8Array;
  payloadType: string;
}> {
  const buf = await withTimeout(invoke<ArrayBuffer>("spike_spectrogram_texture"), FETCH_TIMEOUT_MS, () => {
    throw new Error(`spike_spectrogram_texture timed out after ${FETCH_TIMEOUT_MS}ms`);
  });
  const frame = decodeSpectrogramFrame(buf);
  return { width: frame.width, height: frame.height, pixels: frame.pixels, payloadType: describePayloadType(buf) };
}

/** A synthetic new column of magnitudes for frame `frameIndex`, standing in for a freshly hopped
 * STFT column (ADR-003 `VXST`) arriving during playback. */
function synthColumn(frameIndex: number, height: number, out: Uint8Array): void {
  const t = frameIndex / 12;
  for (let y = 0; y < height; y++) {
    const freq = y / height;
    const bands = Math.sin(t * 40 + freq * 6) * 0.5 + 0.5;
    const formant = Math.exp(-((freq - 0.3) ** 2) * 40);
    out[y] = Math.max(0, Math.min(255, Math.round((bands * 0.6 + formant * 0.4) * 255)));
  }
}

export interface SpectrogramBenchResult {
  width: number;
  height: number;
  responsePayloadType: string;
  webgl2: FrameStats | { error: string };
  canvas2d: FrameStats | { error: string };
}

export async function runSpectrogramBenches(
  canvasWebgl2: HTMLCanvasElement,
  canvasCanvas2d: HTMLCanvasElement,
): Promise<SpectrogramBenchResult> {
  const { width, height, pixels, payloadType } = await fetchSpectrogramFrame();
  const columnBuf = new Uint8Array(COLUMNS_PER_FRAME * height);
  let frameIndex = 0;

  function nextColumns(): Uint8Array {
    for (let c = 0; c < COLUMNS_PER_FRAME; c++) {
      synthColumn(frameIndex, height, columnBuf.subarray(c * height, (c + 1) * height));
      frameIndex++;
    }
    return columnBuf;
  }

  let webgl2: FrameStats | { error: string };
  try {
    const renderer = makeWebgl2SpectrogramRenderer(canvasWebgl2, width, height);
    renderer.initialize(pixels);
    webgl2 = await runFrameBench("webgl2-spectrogram", SCROLL_BENCH_MS, () => {
      renderer.pushColumnsAndDraw(nextColumns(), COLUMNS_PER_FRAME);
    });
  } catch (e) {
    webgl2 = { error: e instanceof Error ? e.message : String(e) };
  }

  let canvas2d: FrameStats | { error: string };
  try {
    const renderer = makeCanvas2dSpectrogramRenderer(canvasCanvas2d, width, height);
    renderer.initialize(pixels);
    canvas2d = await runFrameBench("canvas2d-spectrogram", SCROLL_BENCH_MS, () => {
      renderer.pushColumnsAndDraw(nextColumns(), COLUMNS_PER_FRAME);
    });
  } catch (e) {
    canvas2d = { error: e instanceof Error ? e.message : String(e) };
  }

  return { width, height, responsePayloadType: payloadType, webgl2, canvas2d };
}
