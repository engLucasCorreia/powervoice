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
import { METER_FLOOR_DB } from "../meters/meterScale";
import { createFrameClient } from "../render/frameScheduler";
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
 * the input meter's clip lamp. The reactive `meter` assignment is skipped entirely once nothing
 * has actually changed (owner note, H-41: a meter that has decayed to the floor on a silent signal
 * must not keep repainting or holding the main thread busy — see `meterEquals` below).
 *
 * H-43 (idle CPU): no perpetual animation-frame loop. The playhead is extrapolated on frames from
 * the shared scheduler only while it moves (playing or recording); an idle engine stops sending
 * telemetry once its meters rest, so the meter's bar and hold finish falling on frames too — and
 * both stop requesting frames once there's nothing left to move.
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
  revision: 0,
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
  // H-81: a command's own response and the `transport_state` event it triggers travel over
  // independent Tauri channels (no ordering guarantee between them), and an unrelated concurrent
  // command's response can arrive later still, carrying an older snapshot (e.g. a selection sync
  // still in flight when Loop is toggled off). `revision` is monotonic on the engine side —
  // dropping anything not newer than what's already applied stops a stale reply from resurrecting
  // old loop/selection state (the loop toggle appearing stuck on, the loop overlay not clearing).
  if (next.revision < state.revision) {
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
  lastFrameAtMs = atMs;
  lastOutPeakDbfs = frame.outPeakDbfs;
  lastOutRmsDbfs = frame.outRmsDbfs;
  // H-41 (owner note): once the meter has settled — decayed to the floor on a silent signal, with
  // nothing left to throttle or smooth toward — `writeMeter` skips the reactive write, so a frame
  // that changes nothing visible never triggers Svelte re-renders/repaints.
  writeMeter(stepMeter(frame.outPeakDbfs, frame.outRmsDbfs, atMs));
  if (meterVisible(meter)) {
    // H-43: an idle engine stops sending once its own meters rest, so the bar and hold finish
    // falling on animation frames (`meterFrame`), which stop at the floor.
    meterClient.invalidate();
  }
  const nowNs = clock.nowNs();
  extrapolator.update(
    { sample: frame.playheadSample, timeNs: frame.playheadTimeNs, rate: frame.rate },
    nowNs,
    state.doc_len_samples,
  );
  moving = frame.rate > 0;
  if (moving) {
    // Playing or recording: the playhead is extrapolated every animation frame (`playheadFrame`).
    playheadClient.invalidate();
  } else {
    // Stopped (rate 0): the anchor is the position — no animation needed.
    playheadSamples = extrapolator.position(nowNs, state.doc_len_samples);
  }
}

// --- H-43: animation frames only while something moves ------------------------------------------

/** With no telemetry frame for this long, the output meter's ballistics run on animation frames
 * (longer than two frame periods at the 30 Hz telemetry setting). */
const METER_STALE_MS = 100;
/** A level at or below this counts as the engine reporting silence (its idle rest floor). */
const SILENT_SOURCE_DBFS = -120;
/** A moving playhead keeps extrapolating for at most this long after its last telemetry frame
 * (the engine sends every frame while playing or recording; a stalled stream mustn't animate
 * forever). */
const PLAYHEAD_STALE_MS = 1_000;

let lastFrameAtMs = Number.NEGATIVE_INFINITY;
let lastOutPeakDbfs = Number.NEGATIVE_INFINITY;
let lastOutRmsDbfs = Number.NEGATIVE_INFINITY;
/** The last telemetry frame had a moving playhead (playing or recording: `rate > 0`). */
let moving = false;

function writeMeter(next: OutputMeter): void {
  if (!meterEquals(meter, next)) {
    meter = next;
  }
}

/** One ballistics step toward `peakDbfs`/`rmsDbfs` at `atMs` (a telemetry frame or, once the
 * stream has stopped, an animation frame repeating the last one). */
function stepMeter(peakDbfs: number, rmsDbfs: number, atMs: number): OutputMeter {
  outputBallistics.update(peakDbfs, atMs);
  rmsSmoothed.update(rmsDbfs, atMs);
  peakReadout.update(outputBallistics.hold, atMs);
  rmsReadout.update(rmsSmoothed.value, atMs);
  return {
    peakDbfs: outputBallistics.bar,
    holdDbfs: outputBallistics.hold,
    rmsDbfs,
    peakReadoutDbfs: peakReadout.value,
    rmsReadoutDbfs: rmsReadout.value,
    clip: outputClipLatched,
  };
}

/** Whether any bar or the hold tick still shows above the meter's scale floor. */
function meterVisible(m: OutputMeter): boolean {
  return m.peakDbfs > METER_FLOOR_DB || m.holdDbfs > METER_FLOOR_DB || m.rmsDbfs > METER_FLOOR_DB;
}

/**
 * H-43 item 4: the output meter's decay once telemetry stops. While frames still arrive they
 * drive the meter (this only keeps watching); once the stream is stale the ballistics keep
 * stepping on animation frames with the last frame's (silent) input until the bar and hold have
 * fallen below the scale, then everything snaps to silence and no further frame is requested.
 * The peak-hold timer doesn't keep a loop alive on its own: a meter that isn't visibly moving
 * any more and has no hold above its bar stops too.
 */
function meterFrame(): boolean {
  const atMs = nowMs();
  if (atMs - lastFrameAtMs < METER_STALE_MS) {
    return meterVisible(meter);
  }
  const prev = meter;
  const next = stepMeter(lastOutPeakDbfs, lastOutRmsDbfs, atMs);
  const sourceSilent = lastOutPeakDbfs <= SILENT_SOURCE_DBFS && lastOutRmsDbfs <= SILENT_SOURCE_DBFS;
  if (sourceSilent && !meterVisible(next)) {
    outputBallistics.reset();
    rmsSmoothed.reset(Number.NEGATIVE_INFINITY);
    peakReadout.reset(Number.NEGATIVE_INFINITY);
    rmsReadout.reset(Number.NEGATIVE_INFINITY);
    writeMeter({ ...SILENT, clip: outputClipLatched });
    return false;
  }
  writeMeter(next);
  return !(meter === prev && next.holdDbfs <= next.peakDbfs);
}

/** H-43: the extrapolated playhead, every animation frame while it moves. Runs before the
 * renderers (priority), so they draw this frame's position. */
function playheadFrame(): boolean {
  if (!extrapolator.hasAnchor) {
    return false;
  }
  playheadSamples = extrapolator.position(clock.nowNs(), state.doc_len_samples);
  return moving && nowMs() - lastFrameAtMs < PLAYHEAD_STALE_MS;
}

const PLAYHEAD_PRIORITY = -100;
const meterClient = createFrameClient(meterFrame, { priority: PLAYHEAD_PRIORITY + 1, name: "output-meter" });
const playheadClient = createFrameClient(playheadFrame, { priority: PLAYHEAD_PRIORITY, name: "playhead" });

/**
 * H-43: whether the playhead is moving (playing or recording) — renderers keep requesting frames
 * while it is, so they draw every frame at the display rate. Not reactive: renderers redraw on
 * `playheadSamples` changes, and ask this from inside their frame callback.
 */
export function isPlayheadMoving(): boolean {
  return moving;
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
  // H-43: no perpetual animation-frame loop here any more — the playhead and the meter's decay
  // request frames from the shared scheduler only while they move (`playheadFrame`/`meterFrame`).
  cleanups.push(() => {
    moving = false;
  });

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
  lastFrameAtMs = Number.NEGATIVE_INFINITY;
  lastOutPeakDbfs = Number.NEGATIVE_INFINITY;
  lastOutRmsDbfs = Number.NEGATIVE_INFINITY;
  moving = false;
}
