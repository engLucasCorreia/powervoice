/**
 * Throttles the EQ graph's `aria-live="polite"` announcements to at most one message per 250 ms
 * (H-84, SPEC-015 §2.6.5): the leading change announces immediately; a burst of key repeats while
 * the window is still open is coalesced into one trailing announcement of the *final* value, so a
 * screen-reader user always eventually hears where the band actually ended up, not a stale
 * mid-adjustment reading. Clock and timer are injectable for deterministic tests (vitest fake
 * timers).
 */

export const EQ_LIVE_REGION_INTERVAL_MS = 250;

export interface AnnouncerClock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const REAL_CLOCK: AnnouncerClock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export class EqLiveAnnouncer {
  /** The text the live region should currently show. */
  text = "";
  private lastAtMs: number | null = null;
  private pending: string | null = null;
  private timer: unknown = null;
  private readonly clock: AnnouncerClock;
  private readonly onChange?: (text: string) => void;

  /** `onChange` fires every time {@link text} actually changes — both immediately and from the
   * deferred trailing timer — so a caller that mirrors it into reactive state (a Svelte
   * `$state`) is notified even for the delayed case, which a caller re-reading `.text` right
   * after calling {@link announce} would miss. */
  constructor(clock: AnnouncerClock = REAL_CLOCK, onChange?: (text: string) => void) {
    this.clock = clock;
    this.onChange = onChange;
  }

  /** Announces `next`, immediately if the throttle window has elapsed, otherwise deferred to fire
   * at the end of the current window with whatever the latest value is by then. */
  announce(next: string): void {
    if (next === this.text || next === this.pending) {
      return;
    }
    const now = this.clock.now();
    if (this.lastAtMs === null || now - this.lastAtMs >= EQ_LIVE_REGION_INTERVAL_MS) {
      this.fire(next, now);
      return;
    }
    this.pending = next;
    if (this.timer === null) {
      const waitMs = EQ_LIVE_REGION_INTERVAL_MS - (now - this.lastAtMs);
      this.timer = this.clock.setTimeout(() => {
        this.timer = null;
        if (this.pending !== null) {
          this.fire(this.pending, this.clock.now());
        }
      }, waitMs);
    }
  }

  private fire(text: string, atMs: number): void {
    this.text = text;
    this.lastAtMs = atMs;
    this.pending = null;
    if (this.timer !== null) {
      this.clock.clearTimeout(this.timer);
      this.timer = null;
    }
    this.onChange?.(text);
  }

  /** Test/teardown helper: cancels a pending trailing announcement. */
  dispose(): void {
    if (this.timer !== null) {
      this.clock.clearTimeout(this.timer);
      this.timer = null;
    }
  }
}
