/**
 * Synthetic voice spectra and reports for the *Explain My Voice* tests (H-91).
 *
 * Kept out of the test files themselves so the harmonic tests, the findings tests and the
 * snapshot tests all reason about the *same* signals — a harmonic comb built two slightly
 * different ways would let one of them pass on a curve the others never see.
 *
 * Nothing here is a measurement: these are inputs, deliberately built from stated levels so a
 * test can assert what the engine makes of them.
 */
import type { VoiceReportDto } from "../../ipc/bindings";

export interface CombSpec {
  /** Fundamental of the comb (Hz). */
  f0Hz: number;
  /** Peak level of H1, H2, … (dB). `-Infinity` leaves that harmonic out entirely. */
  harmonicsDb: number[];
  /** Broadband level under the comb (dB). */
  floorDb?: number;
  /** Half-width of each line (Hz) — how far the speaker's pitch wandered. */
  lineWidthHz?: number;
  /** Extra non-harmonic resonances: `[frequency, level, half-width]`. */
  resonances?: [number, number, number][];
  binHz?: number;
  maxHz?: number;
}

export interface SyntheticCurve {
  freqsHz: Float64Array;
  levelsDb: Float32Array;
}

/**
 * A harmonic comb on a broadband floor, in the shape of an FFT-bin curve: Gaussian lines at
 * `n · f0Hz` with the given peak levels, summed in **power**, like any real spectrum.
 */
export function combCurve(spec: CombSpec): SyntheticCurve {
  const binHz = spec.binHz ?? 48_000 / 16_384;
  const maxHz = spec.maxHz ?? 6000;
  const width = spec.lineWidthHz ?? 3;
  const floor = 10 ** ((spec.floorDb ?? -95) / 10);
  const n = Math.floor(maxHz / binHz) + 1;
  const freqsHz = new Float64Array(n);
  const levelsDb = new Float32Array(n);
  const lines: [number, number, number][] = spec.harmonicsDb.map((db, i) => [
    (i + 1) * spec.f0Hz,
    db,
    width,
  ]);
  lines.push(...(spec.resonances ?? []));
  for (let i = 0; i < n; i++) {
    const f = i * binHz;
    freqsHz[i] = f;
    let power = floor;
    for (const [centreHz, peakDb, halfWidthHz] of lines) {
      if (!Number.isFinite(peakDb)) {
        continue;
      }
      const x = (f - centreHz) / halfWidthHz;
      power += 10 ** (peakDb / 10) * Math.exp(-0.5 * x * x);
    }
    levelsDb[i] = 10 * Math.log10(power);
  }
  return { freqsHz, levelsDb };
}

/** A `VoiceReportDto` whose every measurement sits in the healthy zone; override what a test
 * is about. */
export function balancedReport(overrides: Partial<VoiceReportDto> = {}): VoiceReportDto {
  return {
    f0: {
      current_hz: null,
      median_hz: 120,
      low_hz: 112,
      high_hz: 129,
      voiced_fraction: 0.62,
      confidence: 0.91,
      octave_corrected: 0,
    },
    tone: { mud_db: 2.5, presence_db: -7, air_db: -22 },
    sibilance: { ratio_db: -24, centre_hz: 6300 },
    hum: null,
    rumble_db: -32,
    noise_floor_dbfs: -68,
    active_level_dbfs: -20,
    snr_db: 48,
    span_s: 12.5,
    ...overrides,
  };
}
