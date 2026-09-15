/**
 * H-27 (SPEC-006 §2.8): the continuous ("smooth") band-follow that keeps the playhead in view
 * during playback — the playback counterpart to H-23's page-flip during a record operation
 * (`recordFollow.ts`). Pure state machine, no canvas or Svelte.
 *
 * **Follow band.** SPEC-006 §2.8 (A-001/T-200, decided autonomously): a follow band spanning the
 * middle 80% of the viewport width (10%-90%). While the playhead stays inside it, the view
 * doesn't move. Once it would cross an edge, `startSample` shifts by exactly the overflow that
 * frame, so the playhead visually holds at the band edge while the waveform scrolls under it.
 * Called every animation frame with the current (already-extrapolated, SPEC-003 §2.2) displayed
 * position, this produces a genuinely continuous scroll during ordinary playback (the overflow
 * each frame is a handful of samples) and, after a seek/loop-wrap/Return-to-Start (SPEC-003 §2.2's
 * "jump" telemetry landing the displayed position far outside the band in a single frame), a
 * one-frame reframe that puts the playhead back on the nearest edge — no special-casing "jump vs.
 * slew" is needed here, since this module only ever reads the *current* displayed position.
 *
 * **Whole document fits.** SPEC-006 §2.8's band-follow has nothing to hold against when the whole
 * document already fits the viewport (`viewportSamples >= lenSamples`): `startSample` is pinned at
 * 0 either way (`coords.ts`'s `clampStartSample`), but this module still special-cases it so "at
 * the zoom that fits the whole file, nothing scrolls" is testable as this module's own guarantee,
 * not an accident of the caller's clamping.
 *
 * **User-scroll suspension.** SPEC-006 §2.8: a user-initiated scroll or zoom during playback
 * suspends band-follow "until the playhead next re-enters the band from a subsequent telemetry
 * anchor" — [`suspendPlaybackFollow`] records that a suspension is in effect; [`followPlayhead`]
 * clears it exactly when the playhead is next found inside the band (never fighting the user by
 * itself re-centering first). Whether a given viewport change was the user's or this module's own
 * last write isn't this module's concern — `WaveformView.svelte` tells the two apart with one
 * `ViewportWriter` (`viewportFollow.ts`) shared with `recordFollow.ts`'s page-flip, rather than
 * each policy duplicating that diff.
 */

/** SPEC-006 §2.8: the follow band is the middle 80% of the viewport (10%-90%). */
export const FOLLOW_BAND_START_FRACTION = 0.1;
export const FOLLOW_BAND_END_FRACTION = 0.9;

export interface PlaybackFollowState {
  /** A user-initiated viewport change suspended the band-follow. */
  suspended: boolean;
}

/** Not suspended — the state before playback starts (or after it stops). */
export const INITIAL_PLAYBACK_FOLLOW_STATE: PlaybackFollowState = { suspended: false };

/**
 * Call every animation frame during playback with the current viewport (`startSample`,
 * `viewportSamples` = the viewport's width in samples), the document length, and the extrapolated
 * playhead's displayed document position (SPEC-003 §2.2). Returns the `startSample` to show
 * (unclamped — the caller applies the usual `[0, len - viewport]` clamp, same as any other
 * viewport write) and the state to keep for the next call.
 *
 * `viewportSamples <= 0` (no measured viewport yet) or `viewportSamples >= lenSamples` (the whole
 * document already fits) is a no-op: there's no edge to hold the playhead against.
 */
export function followPlayhead(
  startSample: number,
  viewportSamples: number,
  lenSamples: number,
  playheadSample: number,
  state: PlaybackFollowState,
): { startSample: number; state: PlaybackFollowState } {
  if (viewportSamples <= 0 || viewportSamples >= lenSamples) {
    return { startSample, state };
  }
  const bandStart = startSample + FOLLOW_BAND_START_FRACTION * viewportSamples;
  const bandEnd = startSample + FOLLOW_BAND_END_FRACTION * viewportSamples;
  if (playheadSample >= bandStart && playheadSample <= bandEnd) {
    // Inside the band: nothing to shift, and a previously suspended follow resumes — the
    // playhead has re-entered (SPEC-006 §2.8's resume rule).
    return { startSample, state: state.suspended ? INITIAL_PLAYBACK_FOLLOW_STATE : state };
  }
  if (state.suspended) {
    // Outside the band, but still waiting for the resume condition above — respect the user's
    // own scroll/zoom a while longer.
    return { startSample, state };
  }
  const overflow = playheadSample > bandEnd ? playheadSample - bandEnd : playheadSample - bandStart;
  return { startSample: startSample + overflow, state };
}

/**
 * Call when the viewport changed for a reason other than [`followPlayhead`]'s own return value (a
 * user scroll, zoom, click-to-seek or the shared scrollbar) while playback is running. Marks
 * band-follow suspended until [`followPlayhead`] finds the playhead back inside the band.
 */
export function suspendPlaybackFollow(): PlaybackFollowState {
  return { suspended: true };
}
