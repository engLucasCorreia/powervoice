/** Peak bar release (SPEC-002 §3 `meter_peak_release_db_per_s`). */
export const PEAK_RELEASE_DB_PER_S = 20;
/** Peak-hold tick duration (SPEC-002 §3 `meter_peak_hold_s`). */
export const PEAK_HOLD_MS = 1500;

/**
 * Input peak ballistics (SPEC-002 §2.1; the engine sends per-frame max-hold peaks, the UI only
 * applies ballistics, §4.5): the bar has instant attack and falls at 20 dB/s; the hold tick
 * stays for 1.5 s, then falls at the same rate (never below the bar).
 */
export class PeakBallistics {
  bar = Number.NEGATIVE_INFINITY;
  hold = Number.NEGATIVE_INFINITY;
  private holdSinceMs = 0;
  private lastMs: number | null = null;

  update(peakDbfs: number, nowMs: number): void {
    const dt = this.lastMs === null ? 0 : Math.max(0, nowMs - this.lastMs) / 1000;
    this.lastMs = nowMs;
    const fall = PEAK_RELEASE_DB_PER_S * dt;
    this.bar = Math.max(peakDbfs, this.bar - fall);
    if (peakDbfs >= this.hold) {
      this.hold = peakDbfs;
      this.holdSinceMs = nowMs;
    } else if (nowMs - this.holdSinceMs > PEAK_HOLD_MS) {
      this.hold = Math.max(this.bar, this.hold - fall);
    }
  }

  reset(): void {
    this.bar = Number.NEGATIVE_INFINITY;
    this.hold = Number.NEGATIVE_INFINITY;
    this.holdSinceMs = 0;
    this.lastMs = null;
  }
}
