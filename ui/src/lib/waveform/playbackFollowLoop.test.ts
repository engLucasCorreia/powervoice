import { describe, expect, it } from "vitest";
import { FOLLOW_BAND_START_FRACTION, INITIAL_PLAYBACK_FOLLOW_STATE, followPlayhead } from "./playbackFollow";

/**
 * H-37 (SPEC-006 §2.8 + SPEC-003 §2.2): at a loop wrap the displayed playhead jumps from the
 * loop end back to the loop start. When the loop is longer than the view, band-follow reframes in
 * that one frame so the loop start sits on the band's leading edge — the view jumps back with it.
 */
describe("band-follow across a loop wrap", () => {
  const VIEWPORT = 1_000;
  const LEN = 1_000_000;
  const [LOOP_START, LOOP_END] = [10_000, 30_000];

  it("follows to the loop end, then jumps back to show the loop start", () => {
    let start = LOOP_START - 100;
    let state = INITIAL_PLAYBACK_FOLLOW_STATE;
    // Play through the loop in 50-sample frames: the view scrolls with the playhead.
    for (let p = LOOP_START; p < LOOP_END; p += 50) {
      ({ startSample: start, state } = followPlayhead(start, VIEWPORT, LEN, p, state));
    }
    expect(start).toBeGreaterThan(LOOP_END - VIEWPORT);
    // The wrap: the next frame's position is the loop start.
    ({ startSample: start, state } = followPlayhead(start, VIEWPORT, LEN, LOOP_START + 10, state));
    expect(start).toBeCloseTo(LOOP_START + 10 - FOLLOW_BAND_START_FRACTION * VIEWPORT, 6);
    expect(state).toBe(INITIAL_PLAYBACK_FOLLOW_STATE);
  });

  it("does not move when the whole loop sits inside the follow band", () => {
    // A 40_000-sample view from LOOP_START − 5_000: the band spans LOOP_START − 1_000 to
    // LOOP_START + 31_000, so both the loop end and the wrapped position stay inside it.
    const start = LOOP_START - 5_000;
    for (const p of [LOOP_END - 5, LOOP_START + 5]) {
      const result = followPlayhead(start, 40_000, LEN, p, INITIAL_PLAYBACK_FOLLOW_STATE);
      expect(result.startSample).toBe(start);
    }
  });
});
