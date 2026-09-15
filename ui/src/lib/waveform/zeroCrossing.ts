/**
 * Zero-crossing snap (SPEC-006 §2.10, §4.4): pure scan over an already-fetched window of raw
 * samples, plus the async wrapper that fetches that window through the existing `peaks_get` RAW
 * path (§4.3) — never a separate IPC round trip / new command, and never on the audio thread
 * (CLAUDE.md real-time rules): `peaks_get` runs through `run_blocking` on the Rust side
 * (`src-tauri/src/ipc/document_commands.rs`), same as every other peaks request.
 *
 * The pure scan ({@link findZeroCrossing}) is what §6's "zero-crossing search function (synthetic
 * arrays incl. no-crossing case)" tests target.
 */

import type { PeaksRequestDto } from "../ipc/bindings";
import { RAW_SPP } from "./coords";
import { decodeVxpk } from "./vxpk";

/** SPEC-006 §3 `snap_search_window_samples`: the fixed ±512-sample search window. */
export const ZERO_CROSSING_WINDOW_SAMPLES = 512;

/** `true` when index `i` (into `samples`) is a zero crossing per SPEC-006 §2.10: `samples[i]`
 * exactly `0` ("a sample exactly at zero is its own zero crossing"), or a sign change between
 * `samples[i]` and `samples[i + 1]` (`samples[i] >= 0 > samples[i+1]` or
 * `samples[i] < 0 <= samples[i+1]`). */
function isZeroCrossingAt(samples: ArrayLike<number>, i: number): boolean {
  if (i < 0 || i >= samples.length) {
    return false;
  }
  const a = samples[i]!;
  if (a === 0) {
    return true;
  }
  if (i + 1 >= samples.length) {
    return false;
  }
  const b = samples[i + 1]!;
  return (a >= 0 && b < 0) || (a < 0 && b >= 0);
}

/**
 * The pure scan (SPEC-006 §4.4): given `samples` covering absolute document indices
 * `[windowStart, windowStart + samples.length)`, returns the absolute index of the zero crossing
 * nearest `center` (an absolute index expected to fall inside that range), or `null` if none
 * exists within `maxDistance` samples either side.
 *
 * Distances are checked in increasing order (nearest first); at an equal distance the earlier
 * (lower) index wins (SPEC-006 §4.4: "breaking ties toward the earlier (lower) index") — checking
 * `center - d` before `center + d` at every `d` guarantees that regardless of which side a
 * crossing happens to be found on first.
 */
export function findZeroCrossing(
  samples: ArrayLike<number>,
  windowStart: number,
  center: number,
  maxDistance: number = ZERO_CROSSING_WINDOW_SAMPLES,
): number | null {
  const local = center - windowStart;
  if (isZeroCrossingAt(samples, local)) {
    return windowStart + local;
  }
  for (let d = 1; d <= maxDistance; d++) {
    const before = local - d;
    if (isZeroCrossingAt(samples, before)) {
      return windowStart + before;
    }
    const after = local + d;
    if (isZeroCrossingAt(samples, after)) {
      return windowStart + after;
    }
  }
  return null;
}

/** Fetches a `peaks_get` RAW response and returns an `ArrayBuffer` (same shape `ipc/commands.ts`'s
 * `peaksGet` returns) — kept as a narrow function type so tests can stub it without mocking Tauri
 * IPC. */
export type FetchRawPeaks = (request: PeaksRequestDto) => Promise<ArrayBuffer>;

let nextZeroCrossingRequestId = 1;

/**
 * Snaps `pointerSample` to the nearest zero crossing within `windowSamples` either side (SPEC-006
 * §2.10/§4.4), fetching the raw sample window through `fetchRawPeaks` (the existing `peaks_get`
 * RAW path, §4.3) — the search always reads audio off the audio thread, never through the RT
 * path (CLAUDE.md), because `peaks_get` is a `run_blocking` Tauri command, same as every other
 * peaks request.
 *
 * Returns `pointerSample` unchanged (no error) when: the document is empty; the fetch fails or is
 * stale (`audio_rev` no longer matches — an edit landed while the request was in flight); or no
 * crossing exists in the window (SPEC-006 §2.10: "silently doing nothing is the least surprising
 * behavior").
 */
export async function snapSampleToZeroCrossing(
  fetchRawPeaks: FetchRawPeaks,
  audioRev: number,
  lenSamples: number,
  pointerSample: number,
  windowSamples: number = ZERO_CROSSING_WINDOW_SAMPLES,
): Promise<number> {
  if (!(lenSamples > 0)) {
    return pointerSample;
  }
  const p = Math.max(0, Math.min(Math.round(pointerSample), lenSamples - 1));
  const start = Math.max(0, p - windowSamples);
  // +1 so the far edge's `sample[i+1]` lookahead is available too.
  const end = Math.min(lenSamples, p + windowSamples + 1);
  const count = end - start;
  if (count <= 0) {
    return pointerSample;
  }
  let buf: ArrayBuffer;
  try {
    buf = await fetchRawPeaks({
      request_id: nextZeroCrossingRequestId++,
      audio_rev: audioRev,
      spp: RAW_SPP,
      start_sample: start,
      count,
    });
  } catch {
    return pointerSample;
  }
  const frame = decodeVxpk(buf);
  if (!frame || !frame.raw || frame.audioRev !== audioRev) {
    return pointerSample;
  }
  const samples = frame.buckets.map(([value]) => value);
  const found = findZeroCrossing(samples, frame.startSample, p, windowSamples);
  return found ?? pointerSample;
}
