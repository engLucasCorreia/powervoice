import { describe, expect, it } from "vitest";
import { PEAK_HOLD_MS, PEAK_RELEASE_DB_PER_S, PeakBallistics } from "../meters/ballistics";
import { findMarker, pickMarker, stepMarkers, type PeakMarker } from "./peakMarkers";

const FLOOR_DB = -120;
const SILENT = () => Number.NEGATIVE_INFINITY;

describe("peakMarkers (H-48 item 3: peak-label markers share H-41's PeakBallistics)", () => {
  it("a fresh pick creates a marker built on the shared PeakBallistics, at full level", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -6, 0);
    expect(markers).toHaveLength(1);
    expect(markers[0]!.pb).toBeInstanceOf(PeakBallistics);
    expect(markers[0]!.pb.hold).toBe(-6);
    expect(markers[0]!.freqHz).toBe(1000);
  });

  it("matches an existing marker within a twelfth of an octave instead of creating a duplicate", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -6, 0);
    pickMarker(markers, 1002, -3, 10); // effectively the same bin re-picked a moment later
    expect(markers).toHaveLength(1);
    expect(findMarker(markers, 999)).toBe(markers[0]);
  });

  it("a louder re-pick at (about) the same frequency re-arms the hold", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -20, 0);
    // Let it fall well past its hold window.
    stepMarkers(markers, SILENT, FLOOR_DB, PEAK_HOLD_MS + 500);
    const fallen = markers[0]!.pb.hold;
    expect(fallen).toBeLessThan(-20);

    pickMarker(markers, 1000, -6, PEAK_HOLD_MS + 500);
    expect(markers[0]!.pb.hold).toBe(-6);
    // Re-armed: doesn't immediately fall again on the very next step.
    stepMarkers(markers, SILENT, FLOOR_DB, PEAK_HOLD_MS + 600);
    expect(markers[0]!.pb.hold).toBe(-6);
  });

  it("a quieter re-pick at the same frequency leaves the louder marker alone", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -6, 0);
    pickMarker(markers, 1000, -20, 10);
    expect(markers[0]!.pb.hold).toBe(-6);
  });

  it("holds, then releases at the shared PEAK_RELEASE_DB_PER_S, exactly like PeakBallistics itself", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -10, 0);

    stepMarkers(markers, SILENT, FLOOR_DB, PEAK_HOLD_MS - 1);
    expect(markers[0]!.pb.hold).toBe(-10); // still within the hold window

    stepMarkers(markers, SILENT, FLOOR_DB, PEAK_HOLD_MS + 1000);
    const expected = -10 - PEAK_RELEASE_DB_PER_S * 1.0; // ~1 s past the hold window
    expect(markers[0]!.pb.hold).toBeCloseTo(expected, 0);
  });

  it("reaches rest: a fully decayed marker is dropped, and stepping the empty result changes nothing", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -10, 0);

    let atMs = 0;
    let survivors = markers;
    // Enough elapsed time for the shared ballistics to fall from -10 dB all the way past its
    // silence floor (SILENCE_FLOOR_DB in ballistics.ts, -300 dB) at 20 dB/s.
    for (let i = 0; i < 40; i++) {
      atMs += 1000;
      survivors = stepMarkers(survivors, SILENT, FLOOR_DB, atMs);
    }
    expect(survivors).toHaveLength(0);

    // At rest: stepping an already-empty marker list is a stable no-op (nothing left to animate).
    const stillEmpty = stepMarkers(survivors, SILENT, FLOOR_DB, atMs + 1000);
    expect(stillEmpty).toHaveLength(0);
  });

  it("never falls below the live level at its frequency, and is dropped once it converges to it", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -10, 0);
    const liveAt = (freqHz: number) => (freqHz === 1000 ? -40 : Number.NEGATIVE_INFINITY);

    let atMs = 0;
    let survivors = markers;
    for (let i = 0; i < 20 && survivors.length > 0; i++) {
      atMs += 500;
      survivors = stepMarkers(survivors, liveAt, FLOOR_DB, atMs);
    }
    // Converged to the live level and dropped, rather than lingering forever a hair above it.
    expect(survivors).toHaveLength(0);
  });

  it("keeps a marker at or above the display floor as long as it's held, regardless of a quiet live level", () => {
    const markers: PeakMarker[] = [];
    pickMarker(markers, 1000, -10, 0);
    const survivors = stepMarkers(markers, SILENT, FLOOR_DB, 100);
    expect(survivors).toHaveLength(1);
    expect(survivors[0]!.pb.hold).toBe(-10);
  });
});
