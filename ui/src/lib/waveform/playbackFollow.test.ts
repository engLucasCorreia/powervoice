import { describe, expect, it } from "vitest";
import {
  followPlayhead,
  FOLLOW_BAND_END_FRACTION,
  FOLLOW_BAND_START_FRACTION,
  INITIAL_PLAYBACK_FOLLOW_STATE,
  suspendPlaybackFollow,
} from "./playbackFollow";

/**
 * SPEC-006 §2.8: while playing, the view scrolls to keep the extrapolated playhead inside a
 * follow band spanning the middle 80% of the viewport (10%-90%), shifting `startSample` by
 * exactly the overflow past whichever edge the playhead would otherwise cross. A user-initiated
 * viewport change suspends this until the playhead is next found inside the band.
 */
describe("followPlayhead", () => {
  const VIEWPORT = 1_000;
  const LEN = 1_000_000;

  describe("band edges", () => {
    it("does nothing while the playhead sits inside the band", () => {
      const start = 5_000;
      for (const playhead of [start + 100, start + 500, start + 899]) {
        const result = followPlayhead(start, VIEWPORT, LEN, playhead, INITIAL_PLAYBACK_FOLLOW_STATE);
        expect(result).toEqual({ startSample: start, state: INITIAL_PLAYBACK_FOLLOW_STATE });
      }
    });

    it("the band's near edge (10%) and far edge (90%) both still count as inside", () => {
      const start = 5_000;
      const bandStart = start + FOLLOW_BAND_START_FRACTION * VIEWPORT;
      const bandEnd = start + FOLLOW_BAND_END_FRACTION * VIEWPORT;
      expect(followPlayhead(start, VIEWPORT, LEN, bandStart, INITIAL_PLAYBACK_FOLLOW_STATE)).toEqual({
        startSample: start,
        state: INITIAL_PLAYBACK_FOLLOW_STATE,
      });
      expect(followPlayhead(start, VIEWPORT, LEN, bandEnd, INITIAL_PLAYBACK_FOLLOW_STATE)).toEqual({
        startSample: start,
        state: INITIAL_PLAYBACK_FOLLOW_STATE,
      });
    });

    it("shifts by exactly the overflow once the playhead would cross the trailing (far) edge", () => {
      const start = 5_000;
      const bandEnd = start + FOLLOW_BAND_END_FRACTION * VIEWPORT; // 5_900
      const playhead = bandEnd + 7;
      const result = followPlayhead(start, VIEWPORT, LEN, playhead, INITIAL_PLAYBACK_FOLLOW_STATE);
      expect(result.startSample).toBe(start + 7);
      // The playhead now sits exactly on the (new) far edge.
      const newBandEnd = result.startSample + FOLLOW_BAND_END_FRACTION * VIEWPORT;
      expect(newBandEnd).toBeCloseTo(playhead, 9);
    });

    it("shifts by exactly the overflow once the playhead would cross the leading (near) edge", () => {
      const start = 5_000;
      const bandStart = start + FOLLOW_BAND_START_FRACTION * VIEWPORT; // 5_100
      const playhead = bandStart - 12;
      const result = followPlayhead(start, VIEWPORT, LEN, playhead, INITIAL_PLAYBACK_FOLLOW_STATE);
      expect(result.startSample).toBe(start - 12);
      const newBandStart = result.startSample + FOLLOW_BAND_START_FRACTION * VIEWPORT;
      expect(newBandStart).toBeCloseTo(playhead, 9);
    });
  });

  it("a playhead jump (seek) far outside the view reframes in one call, playhead landing on the near edge", () => {
    // Viewport currently shows [0, 1_000); a seek lands the playhead far ahead.
    const result = followPlayhead(0, VIEWPORT, LEN, 500_000, INITIAL_PLAYBACK_FOLLOW_STATE);
    const bandEnd = result.startSample + FOLLOW_BAND_END_FRACTION * VIEWPORT;
    expect(bandEnd).toBeCloseTo(500_000, 9);
  });

  it("loop wrap: the playhead jumping backward to the loop start reframes to the leading edge", () => {
    // Playback had scrolled forward to show [50_000, 51_000); the loop wraps back to sample 0.
    const result = followPlayhead(50_000, VIEWPORT, LEN, 0, INITIAL_PLAYBACK_FOLLOW_STATE);
    const bandStart = result.startSample + FOLLOW_BAND_START_FRACTION * VIEWPORT;
    expect(bandStart).toBeCloseTo(0, 9);
    expect(result.startSample).toBeLessThan(50_000);
  });

  describe("the zoom at which the whole document fits", () => {
    it("never scrolls, however far outside the nominal band the playhead sits", () => {
      const shortDoc = 800; // <= VIEWPORT
      for (const playhead of [0, 400, 799]) {
        const result = followPlayhead(0, VIEWPORT, shortDoc, playhead, INITIAL_PLAYBACK_FOLLOW_STATE);
        expect(result).toEqual({ startSample: 0, state: INITIAL_PLAYBACK_FOLLOW_STATE });
      }
    });

    it("a viewport exactly equal to the document length also never scrolls", () => {
      const result = followPlayhead(0, VIEWPORT, VIEWPORT, VIEWPORT - 1, INITIAL_PLAYBACK_FOLLOW_STATE);
      expect(result).toEqual({ startSample: 0, state: INITIAL_PLAYBACK_FOLLOW_STATE });
    });
  });

  it("a viewport of 0 (not yet measured) is a no-op", () => {
    const result = followPlayhead(0, 0, LEN, 500_000, INITIAL_PLAYBACK_FOLLOW_STATE);
    expect(result).toEqual({ startSample: 0, state: INITIAL_PLAYBACK_FOLLOW_STATE });
  });

  describe("user-scroll suspension and resume", () => {
    it("suspending holds the view even though the playhead is outside the band", () => {
      const suspended = suspendPlaybackFollow();
      expect(suspended).toEqual({ suspended: true });
      const result = followPlayhead(5_000, VIEWPORT, LEN, 50_000, suspended);
      expect(result).toEqual({ startSample: 5_000, state: suspended });
    });

    it("resumes exactly when the playhead is found back inside the band, with no shift needed", () => {
      const suspended = suspendPlaybackFollow();
      const start = 5_000;
      const insidePlayhead = start + 500; // inside [5_100, 5_900]... use a clearly-inside value
      const result = followPlayhead(start, VIEWPORT, LEN, insidePlayhead, suspended);
      expect(result).toEqual({ startSample: start, state: INITIAL_PLAYBACK_FOLLOW_STATE });
    });

    it("stays suspended across repeated ticks while the playhead remains outside the band", () => {
      let state = suspendPlaybackFollow();
      for (const playhead of [10_000, 20_000, 30_000]) {
        const result = followPlayhead(5_000, VIEWPORT, LEN, playhead, state);
        expect(result.startSample).toBe(5_000);
        expect(result.state).toEqual({ suspended: true });
        state = result.state;
      }
    });

    it("once resumed, a later out-of-band playhead shifts normally again (not stuck suspended)", () => {
      const suspended = suspendPlaybackFollow();
      const start = 5_000;
      const resumed = followPlayhead(start, VIEWPORT, LEN, start + 500, suspended);
      expect(resumed.state).toEqual(INITIAL_PLAYBACK_FOLLOW_STATE);
      const bandEnd = start + FOLLOW_BAND_END_FRACTION * VIEWPORT;
      const next = followPlayhead(resumed.startSample, VIEWPORT, LEN, bandEnd + 20, resumed.state);
      expect(next.startSample).toBe(start + 20);
    });
  });
});
