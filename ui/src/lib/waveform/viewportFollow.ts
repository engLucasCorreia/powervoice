/**
 * H-27 (ticket item 3): the one piece of machinery both waveform-follow policies need and neither
 * should duplicate — telling a *policy-driven* viewport write (the follow effect's own last
 * `startSample`) apart from a *user-driven* one (a scroll, zoom, click-to-seek, or the shared
 * scrollbar — H-23's `recordFollow.ts` calls this a "user-initiated viewport change").
 *
 * Both policies (`recordFollow.ts`'s page-flip during a record operation, `playbackFollow.ts`'s
 * continuous band-follow during playback) need to suspend themselves the instant something other
 * than their own last write moves the viewport, and both did so by comparing `startSample` to a
 * remembered "what I last set it to" value. `WaveformView.svelte` holds exactly one
 * `ViewportWriter` and funnels whichever policy is active through it (recording and playback are
 * mutually exclusive transport states, so only one policy ever runs at a time) — this is the
 * "single viewport-writer hook" the ticket asks for, and it's what lets switching from one policy
 * to the other mid-session (Stop a record operation, then Play) not mistake the other policy's
 * last write for a user change.
 */

export class ViewportWriter {
  private lastSet: number | null = null;

  /**
   * `true` when `startSample` differs from the last value this writer applied via {@link set} —
   * i.e. something else moved the viewport since. `null` (nothing applied yet, e.g. right after
   * the follow policy became active) never counts as a user change, so a policy's first tick
   * doesn't immediately suspend itself against its own starting position.
   */
  isUserChange(startSample: number): boolean {
    return this.lastSet !== null && startSample !== this.lastSet;
  }

  /** Records `startSample` as this writer's own — call once per tick, after applying whichever
   * follow policy's (possibly unchanged) result. */
  set(startSample: number): void {
    this.lastSet = startSample;
  }

  /** Forgets the last-applied value (the active policy became inactive, e.g. playback stopped or
   * the record operation ended) — the next {@link isUserChange} call returns `false` regardless of
   * what `startSample` does meanwhile, since there's no policy running to protect from it. */
  reset(): void {
    this.lastSet = null;
  }
}
