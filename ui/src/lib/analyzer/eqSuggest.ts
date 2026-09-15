/**
 * "Add EQ band here" (H-42, SPEC-007 §8.10): turns a diagnostic's EQ move into Parametric EQ
 * parameter changes, sent through the normal rack commands (`rack_add` when there is no EQ yet,
 * then `param_set_plain`), so the change shows in the rack and is undone the rack's way.
 *
 * - high-pass → the EQ's HP band (SPEC-015: ids 10/11), switched on at the move's frequency;
 * - notch / cut / boost → a **free** peak band (gain 0 dB, i.e. neutral — SPEC-015 ids
 *   30–73), the one whose current frequency is closest to the target so the graph stays tidy.
 *   No free band → nothing changes and the caller shows a notice.
 * The first Parametric EQ slot in the rack is used; without one, an EQ is added at the end.
 */
import type { RackSlotDto, RackStateDto } from "../ipc/bindings";
import type { EqAction } from "./diagnosticsHints";

export const EQ_MODULE_ID = "org.powervoice.parametric-eq";

/** SPEC-015 §3 parameter ids. */
export const EQ_IDS = {
  hpOn: 10,
  hpFreq: 11,
  peaks: [30, 40, 50, 60, 70].map((base) => ({ on: base, freq: base + 1, gain: base + 2, q: base + 3 })),
} as const;

/** A peak band counts as free when its gain is within this of 0 dB. */
const FREE_GAIN_DB = 0.05;

export interface ParamSet {
  id: number;
  value: number;
}

export interface EqPlan {
  /** `"hp"` or the peak band number 1…5. */
  band: "hp" | 1 | 2 | 3 | 4 | 5;
  sets: ParamSet[];
}

/** Index of the first Parametric EQ slot, or `null`. */
export function eqSlotIndex(state: RackStateDto): number | null {
  const i = state.slots.findIndex((s) => s.module_id === EQ_MODULE_ID);
  return i < 0 ? null : i;
}

function value(slot: RackSlotDto, id: number): number | undefined {
  return slot.values.find((v) => v.id === id)?.value;
}

/** The parameter changes for `action` on `slot`, or `null` when no peak band is free. */
export function planEqAction(slot: RackSlotDto, action: EqAction): EqPlan | null {
  if (action.kind === "high_pass") {
    return {
      band: "hp",
      sets: [
        { id: EQ_IDS.hpFreq, value: action.freqHz },
        { id: EQ_IDS.hpOn, value: 1 },
      ],
    };
  }
  let best: { band: number; distance: number } | null = null;
  EQ_IDS.peaks.forEach((ids, i) => {
    const gain = value(slot, ids.gain) ?? 0;
    if (Math.abs(gain) > FREE_GAIN_DB) {
      return;
    }
    const freq = value(slot, ids.freq) ?? action.freqHz;
    const distance = Math.abs(Math.log2(Math.max(1, freq) / Math.max(1, action.freqHz)));
    if (!best || distance < best.distance) {
      best = { band: i, distance };
    }
  });
  const chosen = best as { band: number; distance: number } | null;
  if (!chosen) {
    return null;
  }
  const ids = EQ_IDS.peaks[chosen.band]!;
  return {
    band: (chosen.band + 1) as 1 | 2 | 3 | 4 | 5,
    sets: [
      { id: ids.freq, value: action.freqHz },
      { id: ids.q, value: action.q },
      { id: ids.gain, value: action.gainDb },
      { id: ids.on, value: 1 },
    ],
  };
}
