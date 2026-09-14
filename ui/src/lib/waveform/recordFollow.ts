/**
 * H-23 (A-016, ticket item 2): during a record operation the waveform view scrolls to keep the
 * record head in view — page-flip style, the way SPEC-006 §2.8 keeps the playhead in view during
 * playback, but in discrete viewport-wide pages rather than a continuous follow band, and only
 * while an operation (`opLayout.ts`'s `OpLayout`) is running.
 *
 * **Page-flip.** The view is divided into fixed pages of one viewport width each, starting at
 * sample 0. While the record head stays inside the page currently shown, the view doesn't move.
 * Once it leaves that page, the view flips straight to the page that now contains it (no
 * intermediate scrolling) — cheap and predictable while peaks keep streaming in.
 *
 * **Respecting a user scroll.** A scroll, zoom, click-to-seek or drag during the operation is a
 * deliberate look at something else; it must not be immediately overridden by the next page flip.
 * SPEC-006 §2.8 handles this for the continuous playhead follow by suspending it "until the
 * playhead next enters the band"; the page-flip equivalent here is to suspend until the record
 * head reaches the page *after* the one the user's own scroll left on screen — reaching that page
 * is exactly the point a flip would happen next regardless of the user's scroll, so resuming there
 * (rather than snapping back immediately) is the least surprising behaviour.
 *
 * Pure state machine, no canvas or Svelte: `WaveformView.svelte` holds one `RecordFollowState`
 * per view, feeding it the record head's document position on every telemetry tick
 * ([`followRecordHead`]) and any viewport change it didn't itself just make
 * ([`suspendRecordFollow`]).
 */

export interface RecordFollowState {
  /** A user-initiated viewport change suspended the page flip. */
  suspended: boolean;
  /** The page (viewport-widths from sample 0) showing when the user's own change suspended it;
   * meaningless while `suspended` is false. */
  suspendedPage: number;
}

/** No pending flip, not suspended — the state before an operation starts (or after one ends). */
export const INITIAL_RECORD_FOLLOW_STATE: RecordFollowState = {
  suspended: false,
  suspendedPage: 0,
};

function pageOf(sample: number, viewportSamples: number): number {
  return Math.floor(sample / viewportSamples);
}

/**
 * Call on every record-head update (a telemetry tick during the operation) with the current
 * viewport (`startSample`, `viewportSamples` = the viewport's width in samples) and the record
 * head's document position. Returns the `startSample` to show (page-aligned once a flip is due,
 * unchanged otherwise) and the state to keep for the next call.
 *
 * `viewportSamples <= 0` (no measured viewport yet) is a no-op.
 */
export function followRecordHead(
  startSample: number,
  viewportSamples: number,
  headSample: number,
  state: RecordFollowState,
): { startSample: number; state: RecordFollowState } {
  if (viewportSamples <= 0) {
    return { startSample, state };
  }
  if (headSample >= startSample && headSample < startSample + viewportSamples) {
    // Still on the page shown (whether or not a user scroll suspended following) — nothing to do.
    return { startSample, state };
  }
  const headPage = pageOf(headSample, viewportSamples);
  if (state.suspended && headPage <= state.suspendedPage) {
    // The head left the page shown, but not yet past the page the user's own scroll left on
    // screen — respect it a while longer.
    return { startSample, state };
  }
  return {
    startSample: headPage * viewportSamples,
    state: INITIAL_RECORD_FOLLOW_STATE,
  };
}

/**
 * Call when the viewport changed for a reason other than [`followRecordHead`]'s own return value
 * (a user scroll, zoom, click-to-seek or the shared scrollbar) while a record operation is
 * running. Records the page it left on screen so the next [`followRecordHead`] call knows to hold
 * off until the record head reaches the page after it.
 */
export function suspendRecordFollow(
  startSample: number,
  viewportSamples: number,
): RecordFollowState {
  if (viewportSamples <= 0) {
    return INITIAL_RECORD_FOLLOW_STATE;
  }
  return { suspended: true, suspendedPage: pageOf(startSample, viewportSamples) };
}
