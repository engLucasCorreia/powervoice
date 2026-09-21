/**
 * The named frequency regions the "Explain My Voice" graph shades (H-92, ticket scope item 3):
 * rumble / fundamental / low-mids / midrange / presence / sibilance / air.
 *
 * Every boundary that already has an owner keeps it: `rumble`, `low_mids` (H-91's `body`,
 * SPEC-007 §8.7's "mud" band), `presence`, `sibilance` and `air` are exactly
 * {@link RUMBLE_BAND_HZ} / {@link BODY_BAND_HZ} / {@link PRESENCE_BAND_HZ} /
 * {@link SIBILANCE_BAND_HZ} / {@link AIR_BAND_HZ} from `findings.ts` — one definition, not a
 * second copy that could drift from the thresholds a card's anchor is built from.
 *
 * Two bands have no existing owner because nothing before this ticket needed them as a *named
 * region* (only as thresholds): `midrange` fills the un-named gap between low-mids and presence
 * (500 Hz–2 kHz, where a voice's higher formants and consonant energy sit) and `fundamental` is
 * never a fixed range — it is the take's own measured F0 10th–90th percentile band
 * ({@link PitchProfile.lowHz}/`highHz`), so a reader with a very different pitch sees a
 * different band, exactly as H-91's "nothing is invented" rule expects. With no pitch profile
 * (silent or unvoiced take) the fundamental band is omitted rather than guessed.
 *
 * Bands legitimately overlap (presence 2–5 kHz and sibilance 4–10 kHz share 4–5 kHz, sibilance
 * and air share 10 kHz) — the ticket asks for exactly that ("overlapping where acoustically
 * right"), so {@link regionsAt} can return more than one id for a frequency.
 */
import type { PitchProfile } from "./snapshot";
import { AIR_BAND_HZ, BODY_BAND_HZ, PRESENCE_BAND_HZ, RUMBLE_BAND_HZ, SIBILANCE_BAND_HZ } from "./findings";

export type VoiceBandId = "rumble" | "fundamental" | "low_mids" | "midrange" | "presence" | "sibilance" | "air";

export interface VoiceBand {
  id: VoiceBandId;
  lowHz: number;
  highHz: number;
}

/** The gap between low-mids and presence: higher formants and consonant energy, un-named by any
 * existing SPEC-007 §8.7 threshold. */
export const MIDRANGE_BAND_HZ: [number, number] = [500, 2000];

const STATIC_BANDS: readonly [Exclude<VoiceBandId, "fundamental">, readonly [number, number]][] = [
  ["rumble", RUMBLE_BAND_HZ],
  ["low_mids", BODY_BAND_HZ],
  ["midrange", MIDRANGE_BAND_HZ],
  ["presence", PRESENCE_BAND_HZ],
  ["sibilance", SIBILANCE_BAND_HZ],
  ["air", AIR_BAND_HZ],
];

/** Every band this take supports, ascending by `lowHz`. `pitch` is `null` when nothing voiced
 * was measured — the fundamental band is then omitted, not guessed. */
export function voiceBands(pitch: Pick<PitchProfile, "lowHz" | "highHz"> | null): VoiceBand[] {
  const bands: VoiceBand[] = STATIC_BANDS.map(([id, [lowHz, highHz]]) => ({ id, lowHz, highHz }));
  if (pitch && pitch.highHz > pitch.lowHz && pitch.lowHz > 0) {
    bands.push({ id: "fundamental", lowHz: pitch.lowHz, highHz: pitch.highHz });
  }
  return bands.sort((a, b) => a.lowHz - b.lowHz);
}

/** Every band `freqHz` falls inside (possibly more than one — bands overlap by design). */
export function regionsAt(freqHz: number, bands: readonly VoiceBand[]): VoiceBandId[] {
  return bands.filter((b) => freqHz >= b.lowHz && freqHz <= b.highHz).map((b) => b.id);
}
