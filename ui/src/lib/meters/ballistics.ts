/**
 * Meter ballistics shared by every level meter in the app (H-41, moved here from
 * `record/ballistics.ts` so the output meter — and, later, any other meter — can reuse the same
 * maths instead of re-deriving it): the input meter (SPEC-002 §2.1/§3/§4.5 — the engine sends
 * per-frame max-hold peaks and a properly windowed RMS, the UI only applies ballistics) and the
 * output meter (no output-specific spec section exists; ADR-003's `VXTM` table and this ticket
 * both reuse SPEC-002's fixed values verbatim rather than inventing new ones).
 *
 * Everything here is pure/canvas-free and driven by whatever calls `update()` — there is
 * deliberately no `requestAnimationFrame`/`setInterval` loop in this module (the stores add one on
 * top, `state/transport.svelte.ts`/`state/record.svelte.ts`, H-43/H-123, so the bar keeps
 * animating at display rate between and after real telemetry frames instead of only snapping on
 * arrival).
 *
 * **H-123 (owner: "they are very leggy and slow, why is that? can i setup the speed?").** Two
 * separate causes, found by tracing an owner-reported "laggy" bar back through this file and
 * `meters/VerticalMeter.svelte`:
 *  1. `VerticalMeter.svelte`'s `.fill`/`.hold` elements had their own CSS `transition: height/
 *     bottom 100ms linear`, *on top of* the ballistics below already computing a continuous,
 *     analytically-correct value every ~16.7 ms (a real telemetry frame or an animation frame).
 *     Two layers of smoothing in series: every new JS value re-triggered a fresh 100 ms CSS ramp
 *     from wherever the previous ramp had gotten to, so the bar perpetually chased the true value
 *     ~50-100 ms behind it, and an "instant attack" visibly took ~100 ms to arrive. Removed — the
 *     bar/hold now render exactly the value computed below, the same convention every canvas
 *     renderer in this app already follows (draw what was computed, never re-smooth it).
 *  2. The release rate itself (20 dB/s) was fixed, with no way to make it feel snappier. Now
 *     selectable — see {@link MeterSpeed}/{@link METER_SPEED_PROFILES} — while the attack stays
 *     instant at every speed (real meters vary their *return* time, not how fast they catch a
 *     peak).
 */

/** One meter speed choice (Settings → `meter_speed`, mirrors the Rust `MeterSpeedPref`/generated
 * `MeterSpeedPref` binding structurally — a plain string union needs no import/cast either way). */
export type MeterSpeed = "fast" | "medium" | "slow";

/** A speed's ballistics: how fast the peak bar/hold fall back down, and how much extra smoothing
 * the numeric RMS readout gets (the RMS *bar* itself always tracks the engine's fixed 300 ms
 * window, `SPEC-002 §3 meter_rms_window_ms` — not selectable, an actual measurement window rather
 * than a UI easing knob). Attack is always instant at every speed. */
export interface MeterSpeedProfile {
  /** Peak bar release, dB/s. */
  peakReleaseDbPerS: number;
  /** Peak-hold tick duration before it starts falling too, ms. */
  peakHoldMs: number;
  /** Time constant for the RMS *readout*'s extra smoothing, ms. */
  readoutSmoothingTauMs: number;
}

/**
 * H-123: Fast halves Medium's numbers, Slow doubles them — Medium keeps the pre-H-123 SPEC-002 §3
 * values (`meter_peak_release_db_per_s: 20`, `meter_peak_hold_s: 1.5`) exactly, so choosing Medium
 * changes nothing about the ballistics themselves, only removes the CSS double-smoothing above.
 * Default: Medium (`DEFAULT_METER_SPEED`) — Fast is snappier still, for a user who wants the bar
 * to read as instantaneous as possible; Slow reads closer to a classic PPM's deliberately damped
 * motion for someone who wants to judge an average level rather than chase every transient.
 */
export const METER_SPEED_PROFILES: Readonly<Record<MeterSpeed, MeterSpeedProfile>> = {
  fast: { peakReleaseDbPerS: 40, peakHoldMs: 750, readoutSmoothingTauMs: 150 },
  medium: { peakReleaseDbPerS: 20, peakHoldMs: 1500, readoutSmoothingTauMs: 300 },
  slow: { peakReleaseDbPerS: 10, peakHoldMs: 2500, readoutSmoothingTauMs: 600 },
};

export const DEFAULT_METER_SPEED: MeterSpeed = "medium";

/** Resolves a possibly-absent/unknown speed (settings not loaded yet, or an unrecognized value)
 * to its profile, defaulting to {@link DEFAULT_METER_SPEED} — pure, so it's trivially testable
 * without touching a store. */
export function meterSpeedProfile(speed: MeterSpeed | null | undefined): MeterSpeedProfile {
  return (speed && METER_SPEED_PROFILES[speed]) || METER_SPEED_PROFILES[DEFAULT_METER_SPEED];
}

/** Peak bar release (SPEC-002 §3 `meter_peak_release_db_per_s`) — the Medium/factory-default
 * speed's value; `PeakBallistics.update`'s own default, kept for callers/tests that don't care
 * about speed at all. */
export const PEAK_RELEASE_DB_PER_S = METER_SPEED_PROFILES.medium.peakReleaseDbPerS;
/** Peak-hold tick duration (SPEC-002 §3 `meter_peak_hold_s`) — Medium's value; see
 * {@link PEAK_RELEASE_DB_PER_S}. */
export const PEAK_HOLD_MS = METER_SPEED_PROFILES.medium.peakHoldMs;
/**
 * How often a meter's *numeric* readout may visibly change (H-41: "numeric readouts refreshed at
 * about 4-5 Hz"). The bar and hold tick still update on every telemetry frame for the lively
 * motion the owner likes; redrawing digits that fast reads as flicker rather than information.
 * 220 ms ≈ 4.5 Hz, the middle of the ticket's "4-5 Hz". Fixed — not part of a speed profile: it
 * paces legibility of the *digits*, unrelated to how fast the bar itself moves.
 */
export const READOUT_INTERVAL_MS = 220;
/**
 * Time constant for a readout's extra smoothing (H-41: "the RMS readout is smoothed") — Medium's
 * value; see {@link PEAK_RELEASE_DB_PER_S}. The RMS *bar* already tracks the engine's proper
 * 300 ms window and stays lively; only the text gets an additional light low-pass so its last
 * digit doesn't hunt between two throttled samples.
 */
export const READOUT_SMOOTHING_TAU_MS = METER_SPEED_PROFILES.medium.readoutSmoothingTauMs;

/** With no telemetry frame for this long, a meter's animation-frame fallback takes over (H-43/
 * H-123): a bit more than two frame periods at the 30 Hz `telemetry_rate_hz` setting, so a single
 * skipped tick never triggers it. Shared by every meter's animation-frame client. */
export const METER_STALE_MS = 100;

/** A level at or below this counts as the engine reporting silence — its own idle-rest floor
 * (`TELEMETRY_REST_DBFS` in `crates/engine/src/telemetry.rs`), used to decide when a meter has
 * genuinely nothing left to show. Deliberately *not* whatever floor the UI happens to display
 * (H-112: the input meter's floor is selectable, −60/−80/−120) — a meter reading −90 dBFS is very
 * much still live audio on a −120 floor, even though it would be off the bottom of a −60 one. */
export const SILENT_SOURCE_DBFS = -120;

/** Whether a peak/RMS pair is fully at rest (H-43/H-123: shared by every meter's animation-frame
 * fallback to decide when to stop animating and snap to silence). */
export function meterSourceAtRest(peakDbfs: number, rmsDbfs: number): boolean {
  return peakDbfs <= SILENT_SOURCE_DBFS && rmsDbfs <= SILENT_SOURCE_DBFS;
}

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
 * `releaseDbPerS` (default {@link PEAK_RELEASE_DB_PER_S}, Medium speed); a hold tick stays at the
 * peak for `holdMs` (default {@link PEAK_HOLD_MS}), then falls at the same rate (never below the
 * bar itself). `releaseDbPerS`/`holdMs` are per-call, not fixed at construction (H-123: a speed
 * change takes effect on the very next update, with no discontinuity — the instance carries no
 * state that depends on which speed produced it).
 */
export class PeakBallistics {
  bar = Number.NEGATIVE_INFINITY;
  hold = Number.NEGATIVE_INFINITY;
  private holdSinceMs = 0;
  private lastMs: number | null = null;

  update(
    peakDbfs: number,
    atMs: number,
    releaseDbPerS: number = PEAK_RELEASE_DB_PER_S,
    holdMs: number = PEAK_HOLD_MS,
  ): void {
    const dt = this.lastMs === null ? 0 : Math.max(0, atMs - this.lastMs) / 1000;
    this.lastMs = atMs;
    const fall = releaseDbPerS * dt;
    this.bar = Math.max(peakDbfs, decay(this.bar, fall));
    if (peakDbfs >= this.hold) {
      this.hold = peakDbfs;
      this.holdSinceMs = atMs;
    } else if (atMs - this.holdSinceMs > holdMs) {
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
 * with time constant `tauMs` (default: the constructor's, H-123: overridable per call so a speed
 * change applies to the very next update). `-Infinity` (digital silence) jumps instantly in
 * either direction — there's nothing meaningful to smooth into or out of silence, and hanging at
 * some quiet-but-finite number while the signal is actually gone would misreport it.
 */
export class SmoothedDb {
  value: number;
  private lastMs: number | null = null;

  constructor(
    initial: number,
    private readonly defaultTauMs: number,
  ) {
    this.value = initial;
  }

  update(target: number, atMs: number, tauMs: number = this.defaultTauMs): void {
    if (this.lastMs === null || !Number.isFinite(this.value) || !Number.isFinite(target)) {
      this.value = target;
    } else {
      const dt = Math.max(0, atMs - this.lastMs);
      const alpha = dt <= 0 ? 0 : 1 - Math.exp(-dt / tauMs);
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
