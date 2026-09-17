/**
 * Keyboard node-editing math (H-84, SPEC-015 §2.6.5 / "Graph constants" `key_freq_step` /
 * `key_gain_step`): pure step functions for the arrow/PageUp/PageDown keys on a focused EQ node,
 * so they're testable without a DOM. Every function clamps to the caller's `[min, max]` — the
 * keyboard must never send an out-of-range value (unlike a drag, which Rust itself clamps).
 */

/** `2^(±1/12)` per arrow key (Shift: `2^(±1/48)`), SPEC-015 §2.6.5 "← / →". */
export function keyFreqFactor(direction: 1 | -1, fine: boolean): number {
  const exponent = fine ? 1 / 48 : 1 / 12;
  return 2 ** (direction * exponent);
}

/** The next frequency for an arrow key, clamped to `[minHz, maxHz]`. */
export function keyFreqHz(
  currentHz: number,
  direction: 1 | -1,
  fine: boolean,
  minHz: number,
  maxHz: number,
): number {
  const next = currentHz * keyFreqFactor(direction, fine);
  return Math.min(Math.max(next, minHz), maxHz);
}

/** SPEC-015 §2.6 "Graph constants" `key_gain_step`. */
export const EQ_KEY_GAIN_STEP_DB = 0.5;
export const EQ_KEY_GAIN_STEP_FINE_DB = 0.1;

/** The next gain for an arrow key (↑ = `direction` 1), clamped to `[minDb, maxDb]`. */
export function keyGainDb(
  currentDb: number,
  direction: 1 | -1,
  fine: boolean,
  minDb: number,
  maxDb: number,
): number {
  const step = fine ? EQ_KEY_GAIN_STEP_FINE_DB : EQ_KEY_GAIN_STEP_DB;
  const next = currentDb + direction * step;
  return Math.min(Math.max(next, minDb), maxDb);
}

/** `2^(±1/6)` per PageUp/PageDown (SPEC-015 §2.6.5) — the same coarse factor as the wheel's. */
export function keyQFactor(direction: 1 | -1): number {
  return 2 ** (direction / 6);
}

/** The next Q for PageUp (`direction` 1) / PageDown (`direction` -1), clamped to `[minQ, maxQ]`. */
export function keyQValue(currentQ: number, direction: 1 | -1, minQ: number, maxQ: number): number {
  return Math.min(Math.max(currentQ * keyQFactor(direction), minQ), maxQ);
}

/** One slope step steeper (↑, `direction` 1) or shallower (↓, `direction` -1), clamped to the
 * enum's index range `[0, maxIndex]` (SPEC-015 §3: `hp_slope`/`lp_slope`, 8 steps 6…48 dB/oct). */
export function stepSlopeIndex(currentIndex: number, direction: 1 | -1, maxIndex: number): number {
  return Math.min(Math.max(currentIndex + direction, 0), maxIndex);
}
