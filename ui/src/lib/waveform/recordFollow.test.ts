import { describe, expect, it } from "vitest";
import {
  followRecordHead,
  INITIAL_RECORD_FOLLOW_STATE,
  suspendRecordFollow,
  type RecordFollowState,
} from "./recordFollow";

/**
 * H-23 (A-016, ticket item 2): the record head stays in view while it's inside the page currently
 * shown (a viewport-wide, page-aligned span starting at sample 0); once it leaves, the view flips
 * straight to the page that now contains it. A user-initiated viewport change suspends flipping
 * until the head reaches the page after the one the user's own change left on screen.
 */
describe("followRecordHead", () => {
  const VIEWPORT = 1_000;

  it("does nothing while the head is inside the page shown", () => {
    const state = INITIAL_RECORD_FOLLOW_STATE;
    for (const head of [0, 1, 500, 999]) {
      const result = followRecordHead(0, VIEWPORT, head, state);
      expect(result).toEqual({ startSample: 0, state });
    }
  });

  it("flips to the page containing the head once it leaves the page shown", () => {
    const result = followRecordHead(0, VIEWPORT, 1_000, INITIAL_RECORD_FOLLOW_STATE);
    expect(result.startSample).toBe(1_000);
    expect(result.state).toEqual(INITIAL_RECORD_FOLLOW_STATE);
  });

  it("flips straight to a page several pages ahead (no intermediate scrolling)", () => {
    const result = followRecordHead(0, VIEWPORT, 5_432, INITIAL_RECORD_FOLLOW_STATE);
    expect(result.startSample).toBe(5_000);
  });

  it("a viewport of 0 (not yet measured) is a no-op", () => {
    const result = followRecordHead(0, 0, 5_000, INITIAL_RECORD_FOLLOW_STATE);
    expect(result).toEqual({ startSample: 0, state: INITIAL_RECORD_FOLLOW_STATE });
  });

  it("suspending records the page the user's own scroll left on screen", () => {
    // The user scrolled to show samples [3 000, 4 000) — page 3.
    const state = suspendRecordFollow(3_000, VIEWPORT);
    expect(state).toEqual({ suspended: true, suspendedPage: 3 });
  });

  it("respects a user scroll: no flip while the head is still on or before the suspended page", () => {
    const suspended = suspendRecordFollow(3_000, VIEWPORT);
    // The head is on page 3 too (inside [3 000, 4 000)) but outside the *shown* range if the user
    // scrolled elsewhere first — simulate by showing a different page than the head's.
    const result = followRecordHead(0, VIEWPORT, 3_500, suspended);
    expect(result).toEqual({ startSample: 0, state: suspended });
  });

  it("resumes once the head reaches the page after the one the user scrolled to", () => {
    const suspended = suspendRecordFollow(3_000, VIEWPORT);
    // The head is now on page 4 — the page right after the suspended page (3) — so flip resumes.
    const result = followRecordHead(0, VIEWPORT, 4_200, suspended);
    expect(result.startSample).toBe(4_000);
    expect(result.state).toEqual(INITIAL_RECORD_FOLLOW_STATE);
  });

  it("a later user scroll re-suspends with its own page, overriding an earlier suspension", () => {
    const first = suspendRecordFollow(3_000, VIEWPORT);
    const second = suspendRecordFollow(7_000, VIEWPORT);
    expect(second).toEqual({ suspended: true, suspendedPage: 7 });
    // The head reached the page after the *first* suspension but not the second: still held.
    const result = followRecordHead(0, VIEWPORT, 4_200, second);
    expect(result).toEqual({ startSample: 0, state: second });
  });

  it("suspending with an unmeasured viewport clears any pending suspension", () => {
    const state: RecordFollowState = { suspended: true, suspendedPage: 3 };
    expect(suspendRecordFollow(3_000, 0)).toEqual(INITIAL_RECORD_FOLLOW_STATE);
    // Sanity: the pre-existing (unrelated) state object is untouched.
    expect(state).toEqual({ suspended: true, suspendedPage: 3 });
  });

  it("the head sitting exactly on the next page boundary counts as reaching it", () => {
    const suspended = suspendRecordFollow(3_000, VIEWPORT);
    const result = followRecordHead(0, VIEWPORT, 4_000, suspended);
    expect(result.startSample).toBe(4_000);
    expect(result.state).toEqual(INITIAL_RECORD_FOLLOW_STATE);
  });
});
