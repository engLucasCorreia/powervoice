/**
 * Canvas-free math for the NR profile graph (H-85, SPEC-014 §2.8 item 2, §4.10): the "reduced
 * to" line, the graph's frequency range, and its auto-fitting dB axis. Kept out of
 * `NoiseProfileGraph.svelte` so it's testable under jsdom (no canvas, MEMORY.md) — mirrors
 * `eq/gainAxis.ts`/`transfer/levelAxis.ts`'s split between pure math and drawing.
 *
 * The curve itself is never computed here: it always comes from Rust's `NoiseProfile::describe()`
 * (`noise_profile_curve`), the same "the UI never evaluates DSP" rule the EQ and transfer graphs
 * follow (SPEC-015/016's AC-17-style guarantee).
 */

import type { ParamInfoDto, RackSlotDto } from "../ipc/bindings";

/** Log axis bottom (SPEC-014 §2.8: "Log frequency axis 20 Hz → min(Nyquist, 24 kHz)"), shared
 * with the analyzer/spectral ruler's own floor (`spectrum/freqAxis.ts::LOG_MIN_HZ`). */
export const NR_GRAPH_MIN_HZ = 20;
/** Log axis top cap. */
export const NR_GRAPH_MAX_HZ = 24_000;

/** dB axis default range (SPEC-014 §2.8: "−120 … 0 dBFS by default"). */
export const NR_GRAPH_DEFAULT_MIN_DB = -120;
export const NR_GRAPH_DEFAULT_MAX_DB = 0;
/** Auto-fit margin around the print's own range (SPEC-014 §2.8: "auto-fits the print's range
 * ±10 dB"). */
const AUTO_FIT_MARGIN_DB = 10;
/** `describe_points`' sentinel for a zero-density band (`EMPTY_BAND_DB` in `vox_dsp::nr`):
 * excluded from the auto-fit so an empty high band never drags the range down to nothing. */
const EMPTY_BAND_SENTINEL_DB = -140;

/** The graph's frequency range for a rack running at `rateHz` (SPEC-014 §2.8: 20 Hz to the
 * lesser of the Nyquist rate and 24 kHz). Falls back to the full cap without a known rate. */
export function noiseProfileFreqRange(rateHz: number): [number, number] {
  const nyquist = rateHz > 0 ? rateHz / 2 : NR_GRAPH_MAX_HZ;
  return [NR_GRAPH_MIN_HZ, Math.max(NR_GRAPH_MIN_HZ + 1, Math.min(nyquist, NR_GRAPH_MAX_HZ))];
}

/**
 * The dB axis range: the SPEC-014 §2.8 default, narrowed to the print's own levels ± a 10 dB
 * margin when that's tighter (never wider than the default, never past it). Levels at or below
 * {@link EMPTY_BAND_SENTINEL_DB} (silent/empty bands) don't influence the fit. Empty input keeps
 * the default.
 */
export function profileDbRange(levelsDbfs: readonly number[]): [number, number] {
  const finite = levelsDbfs.filter((d) => Number.isFinite(d) && d > EMPTY_BAND_SENTINEL_DB);
  if (finite.length === 0) {
    return [NR_GRAPH_DEFAULT_MIN_DB, NR_GRAPH_DEFAULT_MAX_DB];
  }
  const min = Math.min(...finite);
  const max = Math.max(...finite);
  const lo = Math.max(NR_GRAPH_DEFAULT_MIN_DB, min - AUTO_FIT_MARGIN_DB);
  const hi = Math.min(NR_GRAPH_DEFAULT_MAX_DB, max + AUTO_FIT_MARGIN_DB);
  return hi > lo ? [lo, hi] : [NR_GRAPH_DEFAULT_MIN_DB, NR_GRAPH_DEFAULT_MAX_DB];
}

/**
 * The "reduced to" line (SPEC-014 §2.8 item 2): the print minus `reduction_db × amount_pct /
 * 100` at every band — where the noise should end up once the module's dry/wet share is
 * applied. Pure dB-domain arithmetic (never the module's actual DSP, which is decision-directed
 * and signal-dependent — this is the panel's guide line, not a prediction).
 */
export function reducedToLevelsDb(
  printLevelsDbfs: readonly number[],
  reductionDb: number,
  amountPct: number,
): number[] {
  const applied = reductionDb * (amountPct / 100);
  return printLevelsDbfs.map((db) => db - applied);
}

/** A rack slot's current plain value of the parameter keyed `key` (SPEC-012 §2.6's schema key,
 * e.g. `reduction_db`), or `fallback` when the slot has no such parameter. */
export function paramValueByKey(rackSlot: RackSlotDto, key: string, fallback = 0): number {
  const info: ParamInfoDto | undefined = rackSlot.params.find((p) => p.key === key);
  if (!info) {
    return fallback;
  }
  return rackSlot.values.find((v) => v.id === info.id)?.value ?? info.default;
}
