import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { EventName, IpcError, TransportStateDto } from "../ipc/bindings";
import {
  clockNowNs,
  telemetrySubscribe,
  transportGet,
  transportPause,
  transportPlay,
  transportPlayFromStart,
  transportReturnToStart,
  transportSeek,
  transportStop,
} from "../ipc/commands";
import { VXTM_FLAGS, decodeVxtm, toArrayBuffer, type TelemetryFrame } from "../ipc/telemetry";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { ClockSync, PlayheadExtrapolator } from "../transport/playhead";
import { pushNotice } from "./notices.svelte";

/**
 * Transport store (S1-01): the engine's transport state (`transport_state` event + command
 * results), the extrapolated playhead (SPEC-003 §2.2) and the output meter, both fed by `VXTM`
 * telemetry. Registers the transport keymap actions (Space, Shift+Space, Home).
 */

const IDLE: TransportStateDto = {
  playing: false,
  playhead_samples: 0,
  play_start_samples: 0,
  doc_len_samples: 0,
  doc_rate_hz: 0,
  can_play: false,
};

export interface OutputMeter {
  peakDbfs: number;
  rmsDbfs: number;
  clip: boolean;
}

const SILENT: OutputMeter = {
  peakDbfs: Number.NEGATIVE_INFINITY,
  rmsDbfs: Number.NEGATIVE_INFINITY,
  clip: false,
};

let state = $state<TransportStateDto>({ ...IDLE });
let playheadSamples = $state(0);
let meter = $state<OutputMeter>({ ...SILENT });

const extrapolator = new PlayheadExtrapolator();
/** S1-04: other stores (the record panel) see every decoded telemetry frame. */
const telemetryListeners = new Set<(frame: TelemetryFrame) => void>();

/** Adds a telemetry frame listener; returns its removal. */
export function addTelemetryListener(listener: (frame: TelemetryFrame) => void): () => void {
  telemetryListeners.add(listener);
  return () => {
    telemetryListeners.delete(listener);
  };
}
const clock = new ClockSync();

/** Read-only accessor for components. */
export function transportState(): {
  readonly state: TransportStateDto;
  readonly playheadSamples: number;
  readonly meter: OutputMeter;
} {
  return {
    get state() {
      return state;
    },
    get playheadSamples() {
      return playheadSamples;
    },
    get meter() {
      return meter;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

function applyState(next: TransportStateDto): void {
  state = next;
  if (!next.playing && !extrapolator.hasAnchor) {
    playheadSamples = next.playhead_samples;
  }
}

async function run(command: () => Promise<TransportStateDto>): Promise<void> {
  try {
    applyState(await command());
  } catch (err) {
    report(err);
  }
}

export const play = (): Promise<void> => run(transportPlay);
export const pause = (): Promise<void> => run(transportPause);
export const stop = (): Promise<void> => run(transportStop);
export const playFromStart = (): Promise<void> => run(transportPlayFromStart);
export const returnToStart = (): Promise<void> => run(transportReturnToStart);
/** S1-03: click-to-seek on the waveform view moves the playhead to a document sample. */
export const seek = (positionSamples: number): Promise<void> =>
  run(() => transportSeek(positionSamples));

/** Space: Pause while playing, else Play. */
export function playPause(): Promise<void> {
  return state.playing ? pause() : play();
}

/** Handles one telemetry channel message. */
export function onTelemetry(message: unknown): void {
  const buf = toArrayBuffer(message);
  const frame = buf ? decodeVxtm(buf) : null;
  if (!frame) {
    return;
  }
  for (const listener of telemetryListeners) {
    listener(frame);
  }
  meter = {
    peakDbfs: frame.outPeakDbfs,
    rmsDbfs: frame.outRmsDbfs,
    clip: (frame.flags & VXTM_FLAGS.OUT_CLIP) !== 0,
  };
  extrapolator.update(
    { sample: frame.playheadSample, timeNs: frame.playheadTimeNs, rate: frame.rate },
    clock.nowNs(),
    state.doc_len_samples,
  );
}

async function syncClock(): Promise<void> {
  try {
    await clock.sync(clockNowNs);
  } catch {
    // Keep the previous offset; the next periodic sync retries.
  }
}

const CLOCK_SYNC_INTERVAL_MS = 30_000;

/**
 * Wires the store: keymap actions, `transport_state` events, the telemetry channel, clock sync
 * (now and every 30 s) and the per-frame playhead update. Returns the teardown.
 */
export async function initTransport(): Promise<() => void> {
  const cleanups: Array<() => void> = [
    registerAction("transport.play_pause", () => void playPause()),
    registerAction("transport.play_from_start", () => void playFromStart()),
    registerAction("transport.return_to_start", () => void returnToStart()),
  ];
  let disposed = false;

  const requestFrame: (cb: () => void) => number =
    typeof requestAnimationFrame === "function"
      ? (cb) => requestAnimationFrame(cb)
      : (cb) => setTimeout(cb, 16) as unknown as number;
  const cancelFrame: (id: number) => void =
    typeof cancelAnimationFrame === "function" ? (id) => cancelAnimationFrame(id) : (id) => clearTimeout(id);
  let frameId = 0;
  const onFrame = () => {
    if (disposed) {
      return;
    }
    if (extrapolator.hasAnchor) {
      playheadSamples = extrapolator.position(clock.nowNs(), state.doc_len_samples);
    }
    frameId = requestFrame(onFrame);
  };
  frameId = requestFrame(onFrame);
  cleanups.push(() => cancelFrame(frameId));

  const timer = setInterval(() => void syncClock(), CLOCK_SYNC_INTERVAL_MS);
  cleanups.push(() => clearInterval(timer));

  try {
    const unlisten = await listen<TransportStateDto>("transport_state" satisfies EventName, (e) =>
      applyState(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Without events the state still follows command results.
  }
  try {
    await telemetrySubscribe(new Channel<ArrayBuffer>((message) => onTelemetry(message)));
  } catch (err) {
    report(err);
  }
  await syncClock();
  try {
    applyState(await transportGet());
  } catch (err) {
    report(err);
  }

  return () => {
    disposed = true;
    for (const cleanup of cleanups) {
      try {
        cleanup();
      } catch {
        // A failed unlisten during teardown is harmless.
      }
    }
  };
}

/** Test/teardown helper. */
export function resetTransportForTest(): void {
  state = { ...IDLE };
  playheadSamples = 0;
  meter = { ...SILENT };
  extrapolator.reset();
  clock.offsetNs = 0;
}
