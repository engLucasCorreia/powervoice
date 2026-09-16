/**
 * Ballistics for H-42's peak-label markers (the small falling tick drawn where each labelled peak
 * last stood): previously its own hand-rolled hold/fall pair (`MARKER_HOLD_S`/
 * `MARKER_FALL_DB_PER_S` in `SpectrumPlot.svelte`), now built on H-41's shared
 * `meters/ballistics.ts::PeakBallistics` (H-48 item 3) so there is exactly one hold-then-release-
 * then-snap-to-`-Infinity` implementation in the app, and so a marker that has fully decayed
 * actually reaches rest (a stable, repeat-identical value) instead of a per-frame float nudging
 * towards it forever — the thing that used to keep `SpectrumPlot`'s on-demand draw loop
 * (`requestDraw`/`onFrame`) alive after the signal had gone quiet (H-43's idle-CPU concern).
 *
 * Kept as its own canvas-free module (like `peakHold.ts`) so the ballistics are testable without
 * mounting the canvas component.
 */

import { PeakBallistics } from "../meters/ballistics";

export interface PeakMarker {
  freqHz: number;
  readonly pb: PeakBallistics;
}

/** Peaks within this fraction of an octave of an existing marker are the "same" marker (H-42). */
const MATCH_OCTAVE_FRACTION = 1 / 12;

/** The existing marker closest in frequency to `freqHz`, if any is within {@link MATCH_OCTAVE_FRACTION}. */
export function findMarker(markers: readonly PeakMarker[], freqHz: number): PeakMarker | undefined {
  return markers.find((m) => Math.abs(Math.log2(m.freqHz / freqHz)) < MATCH_OCTAVE_FRACTION);
}

/**
 * A freshly picked peak (`levelDb` at `freqHz`, `atMs`): creates a new marker at full hold, or —
 * for an existing one at (about) the same frequency — re-arms it only when the new pick is at
 * least as loud (a quieter re-pick at the same frequency leaves the louder marker's hold/fall
 * alone, same as before this ticket).
 */
export function pickMarker(markers: PeakMarker[], freqHz: number, levelDb: number, atMs: number): void {
  const existing = findMarker(markers, freqHz);
  if (!existing) {
    const pb = new PeakBallistics();
    pb.update(levelDb, atMs);
    markers.push({ freqHz, pb });
    return;
  }
  if (levelDb >= existing.pb.hold) {
    existing.freqHz = freqHz;
    existing.pb.update(levelDb, atMs);
  }
}

/**
 * Advances every marker's ballistics against the plot's current live level at its own frequency
 * (`liveLevelAt`), and drops the ones that are no longer visually distinct from the live curve —
 * either fully decayed to silence (`PeakBallistics` snaps `hold` to `-Infinity`) or, on the way
 * down, close enough to `liveLevelAt` (and below `floorDb`) that a separate tick would show
 * nothing readable. Returns the surviving markers (a fresh array; the input is not mutated by the
 * filter, only each marker's own `pb`).
 */
export function stepMarkers(
  markers: PeakMarker[],
  liveLevelAt: (freqHz: number) => number,
  floorDb: number,
  atMs: number,
): PeakMarker[] {
  for (const m of markers) {
    m.pb.update(liveLevelAt(m.freqHz), atMs);
  }
  return markers.filter((m) => m.pb.hold >= floorDb && m.pb.hold > liveLevelAt(m.freqHz) + 0.05);
}
