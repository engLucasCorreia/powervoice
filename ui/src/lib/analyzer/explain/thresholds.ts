/**
 * Every threshold *Explain My Voice* uses, in one place (H-91 §5: "thresholds live in one
 * documented place, not scattered across the UI").
 *
 * The measurement thresholds SPEC-007 §8.10 already fixed for the diagnostics panel are **not
 * restated here** — they stay in `analyzer/diagnosticsHints.ts`, which owns them, and are
 * re-exported below so a reader has one import to follow and no chance of two different values
 * for the same idea. What this module adds is only what the report needs beyond §8.10:
 *
 * - a **fourth severity step**. §8.10 has three (ok / info / warn); the report needs to separate
 *   "a threshold was crossed" from "crossed by a lot", so `significant` is defined once, as a
 *   fixed margin past the `attention` threshold rather than as a second table of invented
 *   numbers: crossing by more than {@link SIGNIFICANT_MARGIN_DB} is `significant`.
 * - the **harmonic** thresholds (how far a line may sit from where the pitch range puts it, and
 *   how far it must stand above the valleys beside it to count as measured at all);
 * - the **priority** each finding carries when space runs out (H-93's solver drops the lowest).
 *
 * Nothing here is a standard. They are voice-over engineering defaults, like §8.10's.
 */
export {
  ACX_NOISE_FLOOR_DBFS,
  LOW_PITCH_CONFIDENCE,
  NEAR_THRESHOLD_DB,
  NOTABLE_OCTAVE_CORRECTION,
  RUMBLE_WARN_DB,
  SIBILANCE_MODERATE_DB,
  SIBILANCE_STRONG_DB,
  SNR_FAIR_DB,
  SNR_GOOD_DB,
  TONE_ZONES,
} from "../diagnosticsHints";

/**
 * How far past an `attention` threshold a measurement has to be before it is `significant`
 * (dB, or dB-equivalent for SNR). One rule instead of a second threshold table: the escalation
 * is then always explainable as "the threshold, crossed by more than 6 dB".
 */
export const SIGNIFICANT_MARGIN_DB = 6;

/*
 * `NEAR_THRESHOLD_DB` (H-94: "`attention` is not one thing" — a reading 0.7 dB past its
 * threshold and a reading 5 dB past it are both "crossed", but only the second is evidence of
 * anything a listener would hear) moved to `../diagnosticsHints` in H-99, because the panel's
 * own "boomy"/"harsh" wording needed the identical margin: one number, re-exported above, so the
 * panel and this report can never split the same hair of margin into two different verdicts.
 */

/**
 * A broadband RMS noise floor below this (dBFS) is reported as *probably processed* rather than
 * as an achievement (H-94 §5). A microphone's own self-noise, the preamp behind it and any real
 * room put the quietest 500 ms of a take well above this; a file that reads lower has almost
 * always been gated or noise-reduced, and then the floor — and the SNR measured against it —
 * describes the file rather than the recording space. The feature must not congratulate a user
 * for a number that is an artefact of processing.
 */
export const IMPLAUSIBLE_NOISE_FLOOR_DBFS = -80;

/**
 * The largest static tone-shaping move the prose will ever suggest (dB, H-94 §3). A measurement
 * of a voice does not justify more: a bigger move is a production decision, not a correction,
 * and the measurement cannot tell the two apart. Corrective filters aimed at something that is
 * not the voice — a mains notch, a high-pass under 80 Hz — are not tone shaping and are not
 * capped by this.
 */
export const MAX_SUGGESTED_TONE_GAIN_DB = 3;

/** Harmonics H1…H{@link HARMONIC_COUNT} are looked for (H-91 §2). */
export const HARMONIC_COUNT = 6;

/**
 * How far outside the measured pitch range (`low_hz`…`high_hz`) a spectral line may still be
 * counted as that harmonic, in cents. The range itself does the work — a harmonic of a voice
 * that moved between 87 and 129 Hz is smeared over `n·87 … n·129` Hz, not parked at `n·median` —
 * so this is only slack for the difference between a *time* median (what the tracker reports)
 * and an *energy* average (what the spectrum shows).
 */
export const HARMONIC_RANGE_MARGIN_CENTS = 50;

/**
 * A pitch range narrower than this is widened to it before the harmonic bands are built, so a
 * synthetic or very steady report (`low_hz == high_hz`) still gets a band with width to search
 * (cents, half-width).
 */
export const MIN_PITCH_HALF_RANGE_CENTS = 50;

/**
 * How far a harmonic's peak must stand above the valleys on either side of its band before the
 * data is said to support it (dB). Same figure as SPEC-007 §8.2's peak prominence: below it,
 * what is being measured is the shoulder of a neighbour, not a line of its own.
 */
export const HARMONIC_MIN_PROMINENCE_DB = 6;

/**
 * Harmonics are only separable while their bands don't touch: `(n+1)·low_hz > n·high_hz`. Past
 * that the report says `unresolved` rather than pretending to measure — see
 * `harmonics.ts`. This is the largest harmonic number ever *reported* as unresolved rather than
 * simply omitted.
 */
export const MAX_REPORTED_HARMONIC = HARMONIC_COUNT;

/** The strongest peak is related to harmonics up to this number; beyond it "a harmonic of F0"
 * stops being a useful description of a formant-region peak. */
export const MAX_PEAK_HARMONIC = 12;

/**
 * Sub-harmonic (octave-down) test, `chooseFundamental`: the odd harmonics of F0/2 — the lines at
 * 1.5·F0, 2.5·F0, 3.5·F0 that would exist *only* if the tracker had locked onto H2 — must be
 * measurable, and at least this many of them supported, before the report moves the fundamental
 * down an octave. Anything less and the tracker's F0 already explains every line in the
 * spectrum.
 */
export const SUB_HARMONIC_TESTS = [1.5, 2.5, 3.5] as const;
export const MIN_SUPPORTED_SUB_HARMONICS = 2;

/*
 * `LOW_PITCH_CONFIDENCE` and `NOTABLE_OCTAVE_CORRECTION` moved to `../diagnosticsHints` in H-97,
 * which added the same low-confidence/octave-correction signal to the diagnostics panel's F0
 * readout — one number for each idea, re-exported above, so the panel and this report can never
 * disagree about when a pitch reading stopped being firm.
 */

/**
 * Hum is `attention` as soon as SPEC-007 §8.7 detects it, and `significant` once its strongest
 * line is this prominent — §8.7's own "one line this strong is hum on its own" figure, reused
 * rather than a new number.
 */
export const HUM_SIGNIFICANT_PROMINENCE_DB = 20;

/** A voiced fraction below this is reported: there was little voiced audio to measure. */
export const LOW_VOICED_FRACTION = 0.25;

/**
 * Findings are ordered for the annotation layout (H-93) by `priority`, highest first:
 * `priority = BASE_PRIORITY[id] + SEVERITY_PRIORITY[severity]`. The base ranks *what the finding
 * is about* (which harmonic carries the voice beats how much air there is), the severity term
 * lets something that actually crossed a threshold overtake a neighbour that didn't.
 */
export const BASE_PRIORITY = {
  strongest_peak: 90,
  f0: 85,
  hum: 80,
  sibilance: 70,
  snr: 65,
  noise_floor: 60,
  body: 55,
  presence: 50,
  rumble: 45,
  harmonics: 40,
  air: 30,
} as const;

export const SEVERITY_PRIORITY = {
  good: -10,
  info: 0,
  attention: 15,
  significant: 30,
} as const;
