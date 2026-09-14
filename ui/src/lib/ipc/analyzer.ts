/**
 * `VXSA` live output analyzer frame decoder (SPEC-007 §4.9, T-208): little-endian, a 48-byte
 * header (`"VXSA"`, u16 version = 1, u16 header_len, u32 seq, u32 flags, u64 frame_time_ns,
 * u32 sample_rate_hz, u32 fft_size, f32 f0_hz, u32 bands_per_octave, u32 band_count,
 * u32 response), then `f32[band_count]` averaged band levels (dB, `-Infinity` allowed, never
 * NaN). Band centre frequencies are derived deterministically (`f_k = f0Hz * 2^(k/bandsPerOctave)`
 * — see `bandCenterHz`), not transmitted. Like `decodeVxtm`, readers accept a larger `header_len`
 * and reject other magics/versions; truncated frames are rejected too.
 */

export const VXSA_FLAGS = {
  /** History/averaging restarted (device reopen or rate change). */
  RESET: 1 << 0,
  /** Tap samples dropped since the previous frame. */
  DROPPED: 1 << 1,
  /** The analysis window is digital silence. */
  SILENT: 1 << 2,
} as const;

/** `analyzer_subscribe`/`analyzer_set_response`'s averaging response. */
export type AnalyzerResponse = "fast" | "medium" | "slow";

/** Wire code (0 fast, 1 medium, 2 slow) ↔ [`AnalyzerResponse`]. */
export const ANALYZER_RESPONSE_CODES: readonly AnalyzerResponse[] = ["fast", "medium", "slow"];

export function analyzerResponseFromCode(code: number): AnalyzerResponse {
  return ANALYZER_RESPONSE_CODES[code] ?? "medium";
}

export interface AnalyzerFrame {
  seq: number;
  /** History/averaging restarted (device reopen or rate change). */
  reset: boolean;
  /** Tap samples dropped since the previous frame. */
  dropped: boolean;
  /** The analysis window is digital silence. */
  silent: boolean;
  /** App-clock time (ns) of the computation. */
  frameTimeNs: number;
  sampleRateHz: number;
  fftSize: number;
  /** Band-centre base frequency (Hz), always 20. */
  f0Hz: number;
  /** Bands per octave, always 24. */
  bandsPerOctave: number;
  bandCount: number;
  /** Wire code: 0 fast, 1 medium, 2 slow. */
  response: number;
  /** `bandCount` averaged band levels, dB (`-Infinity` allowed, never NaN). */
  levelsDb: number[];
}

const VXSA_V1_HEADER_LEN = 48;

/** u64 → number (exact below 2^53, ADR-003 §4). */
function u64(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true) + dv.getUint32(offset + 4, true) * 2 ** 32;
}

/** Decodes a `VXSA` frame, or `null` if `buf` is not a complete one. */
export function decodeVxsa(buf: ArrayBuffer): AnalyzerFrame | null {
  if (buf.byteLength < VXSA_V1_HEADER_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  const headerLen = dv.getUint16(6, true);
  if (
    magic !== "VXSA" ||
    dv.getUint16(4, true) !== 1 ||
    headerLen < VXSA_V1_HEADER_LEN ||
    headerLen > buf.byteLength
  ) {
    return null;
  }
  const flags = dv.getUint32(12, true);
  const bandCount = dv.getUint32(40, true);
  if (headerLen + 4 * bandCount > buf.byteLength) {
    return null;
  }
  const levelsDb: number[] = [];
  for (let k = 0; k < bandCount; k++) {
    levelsDb.push(dv.getFloat32(headerLen + 4 * k, true));
  }
  return {
    seq: dv.getUint32(8, true),
    reset: (flags & VXSA_FLAGS.RESET) !== 0,
    dropped: (flags & VXSA_FLAGS.DROPPED) !== 0,
    silent: (flags & VXSA_FLAGS.SILENT) !== 0,
    frameTimeNs: u64(dv, 16),
    sampleRateHz: dv.getUint32(24, true),
    fftSize: dv.getUint32(28, true),
    f0Hz: dv.getFloat32(32, true),
    bandsPerOctave: dv.getUint32(36, true),
    bandCount,
    response: dv.getUint32(44, true),
    levelsDb,
  };
}

/** Analyzer band-centre frequency `f_k = f0Hz * 2^(k / bandsPerOctave)` Hz (SPEC-007 §4.8.3),
 * shared by the analyzer panel and (M4) the EQ graph via `freqAxis.ts`. */
export function bandCenterHz(k: number, f0Hz = 20, bandsPerOctave = 24): number {
  return f0Hz * 2 ** (k / bandsPerOctave);
}
