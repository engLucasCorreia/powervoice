import { invoke } from "@tauri-apps/api/core";
import { runIpcThroughputBenches } from "./ipcThroughput";
import { runSpectrogramBenches } from "./spectrogram";
import { runTelemetryBenches } from "./telemetry";
import type { SpikeResults } from "./types";
import { runWaveformBenches } from "./waveform";

const TELEMETRY_DURATION_MS = 5_000;

export interface SpikeCanvases {
  waveformWebgl2: HTMLCanvasElement;
  waveformCanvas2d: HTMLCanvasElement;
  spectrogramWebgl2: HTMLCanvasElement;
  spectrogramCanvas2d: HTMLCanvasElement;
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Runs the full automated measurement suite (ticket scope items 1-3 + the ADR-003 telemetry
 * follow-up) and returns the assembled results. Every step is individually guarded: a hang or
 * throw in one step (rAF withheld entirely, a Channel/Response that never arrives — both observed
 * running this spike on an unfocused Hyprland window, see ADR-009) must not stop the others from
 * running or stop results from being written, so `POWERVOICE_SPIKE_EXIT=1` always eventually closes
 * the app instead of leaving it stuck open.
 */
export async function runFullSpikeSuite(
  canvases: SpikeCanvases,
  webkitDmabufDisabled: boolean,
  onProgress?: (step: string) => void,
): Promise<SpikeResults> {
  let incomplete = false;

  onProgress?.("waveform");
  let waveform: SpikeResults["waveform"];
  try {
    const w = await runWaveformBenches(canvases.waveformWebgl2, canvases.waveformCanvas2d);
    waveform = {
      channelPayloadType: w.channelPayloadType,
      meta: w.meta,
      webgl2: w.webgl2,
      canvas2d: w.canvas2d,
    };
  } catch (e) {
    incomplete = true;
    waveform = { error: errorMessage(e) };
  }

  onProgress?.("spectrogram");
  let spectrogram: SpikeResults["spectrogram"];
  try {
    const s = await runSpectrogramBenches(canvases.spectrogramWebgl2, canvases.spectrogramCanvas2d);
    spectrogram = {
      responsePayloadType: s.responsePayloadType,
      width: s.width,
      height: s.height,
      webgl2: s.webgl2,
      canvas2d: s.canvas2d,
    };
  } catch (e) {
    incomplete = true;
    spectrogram = { error: errorMessage(e) };
  }

  onProgress?.("ipc-throughput");
  let ipcThroughput: SpikeResults["ipcThroughput"];
  try {
    ipcThroughput = await runIpcThroughputBenches();
  } catch (e) {
    incomplete = true;
    ipcThroughput = { error: errorMessage(e) };
  }

  onProgress?.("telemetry");
  let telemetry: SpikeResults["telemetry"];
  try {
    telemetry = await runTelemetryBenches(TELEMETRY_DURATION_MS);
  } catch (e) {
    incomplete = true;
    telemetry = { error: errorMessage(e) };
  }

  return {
    timestamp: new Date().toISOString(),
    webkitDmabufDisabled,
    userAgent: navigator.userAgent,
    waveform,
    spectrogram,
    ipcThroughput,
    telemetry,
    incomplete,
  };
}

/** A last-resort placeholder for when the whole suite (not just one step — those are already
 * caught individually above) fails or blows through the overall watchdog in `SpikeApp.svelte`, so
 * `POWERVOICE_SPIKE_EXIT=1` still has *something* to write and exit after instead of hanging. */
export function makeFailureResults(webkitDmabufDisabled: boolean, message: string): SpikeResults {
  return {
    timestamp: new Date().toISOString(),
    webkitDmabufDisabled,
    userAgent: navigator.userAgent,
    waveform: { error: message },
    spectrogram: { error: message },
    ipcThroughput: { error: message },
    telemetry: { error: message },
    incomplete: true,
  };
}

/** Writes results to `bench-results/spike-<timestamp>.json` via the Rust side (so the path is
 * resolved relative to the repo root regardless of the webview's notion of a working directory)
 * and returns the path written. */
export async function writeResults(results: SpikeResults): Promise<string> {
  return invoke<string>("spike_write_results", { resultsJson: JSON.stringify(results, null, 2) });
}

export async function exitApp(): Promise<void> {
  await invoke("spike_exit");
}
