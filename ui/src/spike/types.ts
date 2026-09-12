/**
 * Hand-written mirrors of the spike-only Rust DTOs (`src-tauri/src/spike/dto.rs`). These
 * deliberately bypass the ts-rs pipeline (ADR-003 / `just gen-types` / `bindings.ts`): the spike
 * is throwaway dev tooling, not a production IPC surface, and must never make
 * `ui/src/lib/ipc/bindings.ts` (or `just check-types`) depend on a cargo feature.
 */

export interface SpikeEnv {
  autoRun: boolean;
  exitAfter: boolean;
  webkitDmabufDisabled: boolean;
}

export interface WaveformMeta {
  totalSamples: number;
  sampleRateHz: number;
  spp: number;
  count: number;
  generationMs: number;
  frameBytes: number;
}

/** One measured frame-time distribution, shared by the waveform and spectrogram benches. */
export interface FrameStats {
  renderer: string;
  frames: number;
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  maxMs: number;
  droppedFrames: number;
  /** frames slower than this many ms count as "dropped" (missed a 60fps budget with slack) */
  dropThresholdMs: number;
  durationMs: number;
  /** `document.visibilityState` + focus sampled at the start and end of the run, so a throttled
   * rAF loop (window not visible/focused, common on Hyprland when unfocused) is visible in the
   * results instead of silently producing misleadingly-good numbers. */
  visibility: { start: VisibilitySample; end: VisibilitySample };
  likelyThrottled: boolean;
  /** the hard wall-clock watchdog fired before the rAF-driven sweep reached its own completion
   * condition — i.e. rAF delivered too few (possibly zero) callbacks. See ADR-009. */
  timedOut: boolean;
}

export interface VisibilitySample {
  visibilityState: DocumentVisibilityState;
  hasFocus: boolean;
  timestamp: number;
}

export interface IpcThroughputResult {
  mechanism: "channel" | "response";
  bytes: number;
  elapsedMs: number;
  mbPerSecond: number;
  payloadType: string;
}

export interface TelemetryResult {
  hz: number;
  requestedDurationMs: number;
  /** telemetry (Channel) frames actually received vs. how many `hz * duration` implies */
  framesReceived: number;
  expectedFrames: number;
  /** JS-side per-message handling cost (decode only — a stand-in for real handler work) */
  avgHandlerTimeUs: number;
  maxHandlerTimeUs: number;
  /** measured once, idle, before any telemetry runs — the actual display rate to compare against
   * (never assumed to be 60 Hz) */
  baselineRafHz: number;
  rafFramesExpected: number;
  rafFramesObserved: number;
  rafFramesDropped: number;
  payloadType: string;
}

export interface SpikeResults {
  timestamp: string;
  webkitDmabufDisabled: boolean;
  userAgent: string;
  waveform:
    | {
        channelPayloadType: string;
        meta: WaveformMeta;
        webgl2: FrameStats | { error: string };
        canvas2d: FrameStats | { error: string };
      }
    | { error: string };
  spectrogram:
    | {
        responsePayloadType: string;
        width: number;
        height: number;
        webgl2: FrameStats | { error: string };
        canvas2d: FrameStats | { error: string };
      }
    | { error: string };
  ipcThroughput: IpcThroughputResult[] | { error: string };
  telemetry: TelemetryResult[] | { error: string };
  /** true if any step above threw/timed out — a partial-results run, not a clean one. Check this
   * (and each step's own `error`/`timedOut`) before trusting the numbers. */
  incomplete: boolean;
}
