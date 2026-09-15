/**
 * Meter ballistics shared by every level meter in the app (H-41, moved here from
 * `record/ballistics.ts` so the output meter — and, later, any other meter — can reuse the same
 * maths instead of re-deriving it): the input meter (SPEC-002 §2.1/§3/§4.5 — the engine sends
 * per-frame max-hold peaks and a properly windowed RMS, the UI only applies ballistics) and the
 * output meter (no output-specific spec section exists; ADR-003's `VXTM` table and this ticket
 * both reuse SPEC-002's fixed values verbatim rather than inventing new ones).
 *
 * Everything here is pure/canvas-free and driven by whatever calls `update()` — there is
 * deliberately no `requestAnimationFrame`/`setInterval` loop in this module. The bar/hold/readout
 * only move when a real telemetry frame arrives (H-41 owner note: don't add an animation that
 * keeps the main thread busy once the signal has settled at the floor).
 */

/** Peak bar release (SPEC-002 §3 `meter_peak_release_db_per_s`). */
export const PEAK_RELEASE_DB_PER_S = 20;
/** Peak-hold tick duration (SPEC-002 §3 `meter_peak_hold_s`). */
export const PEAK_HOLD_MS = 1500;
/**
 * How often a meter's *numeric* readout may visibly change (H-41: "numeric readouts refreshed at
 * about 4-5 Hz"). The bar and hold tick still update on every telemetry frame for the lively
 * motion the owner likes; redrawing digits that fast reads as flicker rather than information.
 * 220 ms ≈ 4.5 Hz, the middle of the ticket's "4-5 Hz".
 */
export const READOUT_INTERVAL_MS = 220;
/**
 * Time constant for a readout's extra smoothing (H-41: "the RMS readout is smoothed"). The RMS
 * *bar* already tracks the engine's proper 300 ms window and stays lively; only the text gets an
 * additional light low-pass so its last digit doesn't hunt between two throttled samples.
 */
export const READOUT_SMOOTHING_TAU_MS = 300;

/** Wall-clock ms, monotonic where available (falls back for non-browser test environments). */
export function nowMs(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/**
 * Once the bar/hold has decayed this far below digital silence, snap straight to `-Infinity`
 * instead of continuing to subtract a finite `fall` from it forever while the input stays silent
 * (H-41 owner note: a meter that has audibly hit bottom must actually *stop* changing — every
 * per-frame telemetry tick would otherwise nudge `bar` to a slightly-more-negative finite number
 * essentially forever, defeating any "skip the redraw once nothing changed" check upstream, since
 * two different finite numbers are never `===`). Far below any real meter's display floor (−60
 * dBFS and up), so this never visibly affects where any meter bottoms out.
 */
const SILENCE_FLOOR_DB = -300;

/** `value - fall`, snapped to `-Infinity` once it's fallen far past digital silence. */
function decay(value: number, fall: number): number {
  const next = value - fall;
  return next <= SILENCE_FLOOR_DB ? Number.NEGATIVE_INFINITY : next;
}

/**
 * Peak-bar ballistics (SPEC-002 §2.1/§4.5): the bar has instant attack and falls at
 * `PEAK_RELEASE_DB_PER_S`; a hold tick stays at the peak for `PEAK_HOLD_MS`, then falls at the
 * same rate (never below the bar itself).
 */
export class PeakBallistics {
  bar = Number.NEGATIVE_INFINITY;
  hold = Number.NEGATIVE_INFINITY;
  private holdSinceMs = 0;
  private lastMs: number | null = null;

  update(peakDbfs: number, atMs: number): void {
    const dt = this.lastMs === null ? 0 : Math.max(0, atMs - this.lastMs) / 1000;
    this.lastMs = atMs;
    const fall = PEAK_RELEASE_DB_PER_S * dt;
    this.bar = Math.max(peakDbfs, decay(this.bar, fall));
    if (peakDbfs >= this.hold) {
      this.hold = peakDbfs;
      this.holdSinceMs = atMs;
    } else if (atMs - this.holdSinceMs > PEAK_HOLD_MS) {
      this.hold = Math.max(this.bar, decay(this.hold, fall));
    }
  }

  reset(): void {
    this.bar = Number.NEGATIVE_INFINITY;
    this.hold = Number.NEGATIVE_INFINITY;
    this.holdSinceMs = 0;
    this.lastMs = null;
  }
}

/**
 * Exponential smoothing for a dB readout (H-41). Every call steps `value` partway toward `target`
 * with time constant `tauMs`. `-Infinity` (digital silence) jumps instantly in either direction —
 * there's nothing meaningful to smooth into or out of silence, and hanging at some quiet-but-
 * finite number while the signal is actually gone would misreport it.
 */
export class SmoothedDb {
  value: number;
  private lastMs: number | null = null;

  constructor(
    initial: number,
    private readonly tauMs: number,
  ) {
    this.value = initial;
  }

  update(target: number, atMs: number): void {
    if (this.lastMs === null || !Number.isFinite(this.value) || !Number.isFinite(target)) {
      this.value = target;
    } else {
      const dt = Math.max(0, atMs - this.lastMs);
      const alpha = dt <= 0 ? 0 : 1 - Math.exp(-dt / this.tauMs);
      this.value += (target - this.value) * alpha;
    }
    this.lastMs = atMs;
  }

  reset(value: number): void {
    this.value = value;
    this.lastMs = null;
  }
}

/**
 * Throttles a readout's *visible* value to at most once per `READOUT_INTERVAL_MS` (H-41). Call it
 * on every telemetry frame with the latest value; `value` only changes that often, always to
 * whatever was latest at the moment it's allowed to change — nothing is averaged away, only the
 * redraw pace is capped.
 */
export class ThrottledReadout {
  value: number;
  private lastMs: number | null = null;

  constructor(initial: number) {
    this.value = initial;
  }

  update(latest: number, atMs: number): void {
    if (this.lastMs === null || atMs - this.lastMs >= READOUT_INTERVAL_MS) {
      this.value = latest;
      this.lastMs = atMs;
    }
  }

  reset(value: number): void {
    this.value = value;
    this.lastMs = null;
  }
}
