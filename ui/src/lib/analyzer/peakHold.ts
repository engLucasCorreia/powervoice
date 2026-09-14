/**
 * Peak-hold ballistics for the live analyzer panel (SPEC-007 §2.9/§4.8 step 7, AC-17): pure UI
 * ballistics computed per animation frame from the received `VXSA` band levels, like the meters
 * (ADR-003 §1). A band's peak holds for {@link PEAK_HOLD_S}, then falls at
 * {@link PEAK_FALL_DB_PER_S} until the live level catches up (which re-arms the hold) or it
 * bottoms out.
 */

/** SPEC-007 §2.9: peak hold time. */
export const PEAK_HOLD_S = 2.0;
/** SPEC-007 §2.9: fall rate after the hold expires. */
export const PEAK_FALL_DB_PER_S = 12.0;

export interface PeakHoldBand {
  /** Current held peak level, dB (`-Infinity` = never held anything). */
  value: number;
  /** Seconds remaining before this band starts falling. */
  holdRemainingS: number;
}

/** A fresh peak-hold state for `bandCount` bands, all at `-Infinity`. */
export function createPeakHold(bandCount: number): PeakHoldBand[] {
  return Array.from({ length: bandCount }, () => ({ value: -Infinity, holdRemainingS: 0 }));
}

/** Clears every band's hold (a click on the panel, or a `RESET` frame, SPEC-007 AC-17). */
export function resetPeakHold(state: readonly PeakHoldBand[]): void {
  for (const band of state) {
    band.value = -Infinity;
    band.holdRemainingS = 0;
  }
}

/**
 * Advances the ballistics by `dtS` seconds given the current instantaneous `levelsDb`. Mutates
 * `state` in place (sized to `levelsDb.length`, or resized if it does not already match).
 */
export function updatePeakHold(
  state: PeakHoldBand[],
  levelsDb: readonly number[],
  dtS: number,
): PeakHoldBand[] {
  if (state.length !== levelsDb.length) {
    state = createPeakHold(levelsDb.length);
  }
  const dt = Math.max(0, dtS);
  for (let i = 0; i < state.length; i++) {
    const level = levelsDb[i] ?? Number.NEGATIVE_INFINITY;
    const band = state[i];
    if (!band) {
      continue;
    }
    if (level >= band.value) {
      band.value = level;
      band.holdRemainingS = PEAK_HOLD_S;
    } else if (band.holdRemainingS > 0) {
      band.holdRemainingS = Math.max(0, band.holdRemainingS - dt);
    } else {
      band.value = Math.max(level, band.value - PEAK_FALL_DB_PER_S * dt);
    }
  }
  return state;
}
