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
  transportSetLoop,
  transportSetSelection,
  transportStop,
} from "../ipc/commands";
import { VXTM_FLAGS, decodeVxtm, toArrayBuffer, type TelemetryFrame } from "../ipc/telemetry";
import { nowMs, PeakBallistics, READOUT_SMOOTHING_TAU_MS, SmoothedDb, ThrottledReadout } from "../meters/ballistics";
import { registerAction } from "../shortcuts";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { ClockSync, PlayheadExtrapolator } from "../transport/playhead";
import { pushNotice } from "./notices.svelte";
import { selectionState } from "./selection.svelte";

/**
 * Transport store (S1-01): the engine's transport state (`transport_state` event + command
 * results), the extrapolated playhead (SPEC-003 §2.2) and the output meter, both fed by `VXTM`
 * telemetry. Registers the transport keymap actions (Space, Shift+Space, Home, Ctrl/⌘+L).
 *
 * H-37 (SPEC-003 §2.1): the Loop toggle (`toggleLoop`) and the time selection sync — the engine
 * loops the selection while loop is on (and Play from start starts at it), so every selection
 * change is sent, latest-wins (one call in flight, so a drag's burst can never land out of
 * order). The extrapolated playhead wraps inside the engine's effective `loop_range`.
 *
 * H-41 (output meter): the engine now sends `out_peak_dbfs` as a true max-hold since the last
 * frame and `out_rms_dbfs` over a proper 300 ms sliding window (`crates/engine/src/telemetry.rs`,
 * `Meter`) — this store only adds ballistics/readout pacing on top, the same way the input meter
 * does (`record.svelte.ts`). `OUT_CLIP` latches client-side until {@link clearOutputClip}, like
 * the input meter's clip lamp. Deliberately no `requestAnimationFrame`/`setInterval` loop here:
 * the bar/hold/readout only move in response to a real telemetry frame, and the reactive `meter`
 * assignment is skipped entirely once nothing has actually changed (owner note, H-41: a meter that
 * has decayed to the floor on a silent signal must not keep repainting or holding the main thread
 * busy — see `meterEquals` below).
 */

const IDLE: TransportStateDto = {
  playing: false,
  playhead_samples: 0,
  play_start_samples: 0,
  doc_len_samples: 0,
  doc_rate_hz: 0,
  can_play: false,
  loop_enabled: false,
  loop_range: null,
};

export interface OutputMeter {
  /** Bar (with ballistics): instant attack, 20 dB/s release. */
  peakDbfs: number;
  /** Peak-hold tick: holds ~1.5 s, then falls at the same rate. */
  holdDbfs: number;
  /** 300 ms RMS (engine-windowed), for the bar. */
  rmsDbfs: number;
  /** The hold value, throttled to ~4-5 Hz for a legible numeric readout. */
  peakReadoutDbfs: number;
  /** The RMS value, smoothed and throttled to ~4-5 Hz for a legible numeric readout. */
  rmsReadoutDbfs: number;
  /** OUT_CLIP, latched until {@link clearOutputClip}. */
  clip: boolean;
}

const SILENT: OutputMeter = {
  peakDbfs: Number.NEGATIVE_INFINITY,
  holdDbfs: Number.NEGATIVE_INFINITY,
  rmsDbfs: Number.NEGATIVE_INFINITY,
  peakReadoutDbfs: Number.NEGATIVE_INFINITY,
  rmsReadoutDbfs: Number.NEGATIVE_INFINITY,
  clip: false,
};

function meterEquals(a: OutputMeter, b: OutputMeter): boolean {
  return (
    a.peakDbfs === b.peakDbfs &&
    a.holdDbfs === b.holdDbfs &&
    a.rmsDbfs === b.rmsDbfs &&
    a.peakReadoutDbfs === b.peakReadoutDbfs &&
    a.rmsReadoutDbfs === b.rmsReadoutDbfs &&
    a.clip === b.clip
  );
}

let state = $state<TransportStateDto>({ ...IDLE });
let playheadSamples = $state(0);
let meter = $state<OutputMeter>({ ...SILENT });
let outputClipLatched = false;
const outputBallistics = new PeakBallistics();
const rmsSmoothed = new SmoothedDb(Number.NEGATIVE_INFINITY, READOUT_SMOOTHING_TAU_MS);
const peakReadout = new ThrottledReadout(Number.NEGATIVE_INFINITY);
const rmsReadout = new ThrottledReadout(Number.NEGATIVE_INFINITY);
/** H-28 item 2: `true` once the initial `transport_state`/`transport_get` has been applied. A
 * telemetry frame that beats it (`initTransport` subscribes to telemetry before awaiting
 * `transportGet`) would otherwise seed the extrapolator with an anchor computed against the
 * `IDLE` default's `doc_len_samples: 0` — clamping every position to 0 until the next frame, and
 * (worse) making {@link applyState}'s `!extrapolator.hasAnchor` check see a real anchor already
 * present, so the authoritative `transport_get`/`transport_state` snapshot's own
 * `playhead_samples` is silently dropped instead of applied. */
let ready = false;

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
  // H-32: a malformed/missing IPC response must never corrupt this store — every reader
  // (RackPanel's `rateHz`, the EQ graph, the meter bridge, `extrapolatedPositionAt`, ...) assumes
  // `state` is always a real `TransportStateDto`. Root cause: `document.svelte.ts`'s `applyDoc`
  // fires an automatic `seek()` on open to restore the saved cursor; the dev-preview harness had
  // no `transport_seek` case, so that command resolved `null` and `state = null` stuck forever —
  // racing the real `transport_get` reply that fires from `initTransport` at the same time,
  // which is why this only showed up sometimes (worse odds the busier the page, e.g. more visible
  // panels at a wider window width delaying one side of the race).
  if (!next) {
    return;
  }
  state = next;
  ready = true;
  extrapolator.setLoop(next.loop_range ?? null);
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
/** H-37 (SPEC-003 §2.1): toggles loop playback (Ctrl/⌘+L, the toolbar's Loop button). */
export const toggleLoop = (): Promise<void> => run(() => transportSetLoop(!state.loop_enabled));

type Range = [number, number] | null;
/** What the engine last acknowledged / what the UI wants it to have. */
let selectionSent: Range = null;
let selectionWanted: Range = null;
let selectionInFlight = false;

function sameRange(a: Range, b: Range): boolean {
  return a === b || (a !== null && b !== null && a[0] === b[0] && a[1] === b[1]);
}

async function pushSelection(): Promise<void> {
  if (selectionInFlight) {
    return; // the running loop below picks the newest wish up when its call returns
  }
  while (!sameRange(selectionWanted, selectionSent)) {
    const next = selectionWanted;
    selectionInFlight = true;
    try {
      applyState(await transportSetSelection(next));
    } catch (err) {
      report(err);
    } finally {
      selectionInFlight = false;
    }
    selectionSent = next;
  }
}

/** H-37: hands the current time selection to the engine (`null` or empty: none). */
export function syncSelection(range: { startSample: number; endSample: number } | null): Promise<void> {
  selectionWanted =
    range && range.endSample > range.startSample
      ? [Math.round(range.startSample), Math.round(range.endSample)]
      : null;
  return pushSelection();
}

/** Space: Pause while playing, else Play. */
export function playPause(): Promise<void> {
  return state.playing ? pause() : play();
}

/**
 * S2-03, SPEC-009 §4.3: the heard position at `eventTimeStampMs` (a `KeyboardEvent.timeStamp`,
 * the same clock as `performance.now()`), so a marker added mid-playback lands where the user
 * heard it, not where the key's IPC round trip happened to land. Falls back to the last known
 * playhead sample while stopped or before any telemetry frame has arrived.
 */
export function extrapolatedPositionAt(eventTimeStampMs: number): number {
  if (!extrapolator.hasAnchor) {
    return playheadSamples;
  }
  const nowNs = eventTimeStampMs * 1e6 + clock.offsetNs;
  return extrapolator.position(nowNs, state.doc_len_samples);
}

/**
 * H-21 (SPEC-022 §2.9): {@link extrapolatedPositionAt} without the clamp to the document length —
 * during a take or record operation the telemetry position is the heard position (or `at + k` in
 * the record window), which runs past the current end while an Insert/Overwrite take grows.
 */
export function extrapolatedHeardPositionAt(eventTimeStampMs: number): number {
  if (!extrapolator.hasAnchor) {
    return playheadSamples;
  }
  const nowNs = eventTimeStampMs * 1e6 + clock.offsetNs;
  return extrapolator.position(nowNs, Number.POSITIVE_INFINITY);
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
  if (!ready) {
    // H-28 item 2: ignore this store's own use of the frame (the input meter, the extrapolator
    // anchor) until the initial transport state is known — see `ready`'s doc comment. Other
    // stores (`record.svelte.ts`'s input meter/elapsed time) already saw it via the listener loop
    // above, unaffected by this store's own readiness.
    return;
  }
  if ((frame.flags & VXTM_FLAGS.OUT_CLIP) !== 0) {
    outputClipLatched = true;
  }
  const atMs = nowMs();
  outputBallistics.update(frame.outPeakDbfs, atMs);
  rmsSmoothed.update(frame.outRmsDbfs, atMs);
  peakReadout.update(outputBallistics.hold, atMs);
  rmsReadout.update(rmsSmoothed.value, atMs);
  const next: OutputMeter = {
    peakDbfs: outputBallistics.bar,
    holdDbfs: outputBallistics.hold,
    rmsDbfs: frame.outRmsDbfs,
    peakReadoutDbfs: peakReadout.value,
    rmsReadoutDbfs: rmsReadout.value,
    clip: outputClipLatched,
  };
  // H-41 (owner note): once the meter has settled — decayed to the floor on a silent signal, with
  // nothing left to throttle or smooth toward — skip the reactive write so an idle telemetry
  // stream (which keeps arriving at the telemetry rate regardless of the signal) doesn't keep
  // triggering Svelte re-renders/repaints for a bar that visibly isn't moving anymore.
  if (!meterEquals(meter, next)) {
    meter = next;
  }
  extrapolator.update(
    { sample: frame.playheadSample, timeNs: frame.playheadTimeNs, rate: frame.rate },
    clock.nowNs(),
    state.doc_len_samples,
  );
}

/** Clicking the output meter's clip indicator clears the latch (H-41, like the input meter's). */
export function clearOutputClip(): void {
  outputClipLatched = false;
  meter = { ...meter, clip: false };
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
    registerAction("transport.toggle_loop", () => void toggleLoop()),
  ];
  // H-37: every selection change reaches the engine (Play from start, the loop region).
  const selection = selectionState();
  cleanups.push(
    $effect.root(() => {
      $effect(() => {
        void syncSelection(selection.current);
      });
    }),
  );
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
  outputClipLatched = false;
  outputBallistics.reset();
  rmsSmoothed.reset(Number.NEGATIVE_INFINITY);
  peakReadout.reset(Number.NEGATIVE_INFINITY);
  rmsReadout.reset(Number.NEGATIVE_INFINITY);
  ready = false;
  extrapolator.reset();
  selectionSent = null;
  selectionWanted = null;
  selectionInFlight = false;
  clock.offsetNs = 0;
}
