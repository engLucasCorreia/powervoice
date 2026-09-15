/**
 * Musical note naming for the analyzer's peak labels and F0 readouts (H-42, SPEC-007 §8.2):
 * equal temperament, A4 = 440 Hz, scientific pitch notation (C4 = middle C = 261.63 Hz), sharps,
 * cents to the nearest note in −50 … +50.
 */
import { t } from "../i18n";
import { formatNumber } from "../ui/units";

export const A4_HZ = 440;
const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;

export interface NoteInfo {
  /** "A", "C#", … */
  name: string;
  octave: number;
  /** −50 … +50, rounded. */
  cents: number;
  /** MIDI note number (A4 = 69). */
  midi: number;
}

/** The nearest note to `freqHz`, or `null` for a non-positive / non-finite frequency. */
export function noteForFreq(freqHz: number): NoteInfo | null {
  if (!(freqHz > 0) || !Number.isFinite(freqHz)) {
    return null;
  }
  const exact = 69 + 12 * Math.log2(freqHz / A4_HZ);
  let midi = Math.round(exact);
  let cents = Math.round((exact - midi) * 100);
  if (cents === 50 && exact < midi) {
    // Math.round(-0.5) rounds up; keep −50 … +50 symmetric.
    cents = -50;
  }
  if (cents > 50) {
    midi += 1;
    cents -= 100;
  }
  const name = NAMES[((midi % 12) + 12) % 12] ?? "C";
  return { name, octave: Math.floor(midi / 12) - 1, cents, midi };
}

/** "A3", "C#5". */
export function noteName(note: NoteInfo): string {
  return `${note.name}${note.octave}`;
}

/** "+12", "−7" (U+2212), "±0". */
export function formatCents(cents: number): string {
  if (cents === 0) {
    return "±0";
  }
  const text = formatNumber(cents, 0);
  return cents > 0 ? `+${text}` : text;
}

/** "A3 +12¢" — through i18n (`analyzer.note_cents`). Empty for no note. */
export function formatNote(freqHz: number): string {
  const note = noteForFreq(freqHz);
  if (!note) {
    return "";
  }
  return t("analyzer.note_cents", { note: noteName(note), cents: formatCents(note.cents) });
}
