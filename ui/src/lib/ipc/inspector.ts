/**
 * H-42 binary spectrum frames (SPEC-007 §8.9), little-endian:
 * - `VXIS` — one Spectrum Inspector frame: `"VXIS"`, u16 version 1, u16 header_len (40), u32 seq,
 *   u32 flags (bit0 RESET, bit1 SILENT), u32 sample_rate_hz, u32 fft_size, u32 window, u32
 *   bin_count, u32 response, u32 reserved, then `f32[bin_count]` averaged levels (dB).
 * - `VXLT` — one long-term average curve: `"VXLT"`, u16 version 1, u16 header_len (36), u32
 *   job_id, u32 index, u32 sample_rate_hz, u32 fft_size, u32 window, u32 bin_count, u32 flags
 *   (bit0 HAS_NOISE), then `f32[bin_count]` levels and, with HAS_NOISE, `f32[bin_count]`
 *   room-tone levels.
 * Bin `k` sits at `k · sample_rate_hz / fft_size` Hz. Readers accept a larger header_len and
 * reject other magics/versions and truncated frames (the `VXSA` convention).
 */
import type { AnalyzerResponseDto, SpectrumWindowDto } from "./bindings";

export const SPECTRUM_WINDOW_CODES: readonly SpectrumWindowDto[] = [
  "hann",
  "blackman_harris",
  "flat_top",
  "rectangular",
];
const RESPONSE_CODES: readonly AnalyzerResponseDto[] = ["fast", "medium", "slow"];

export interface InspectorFrame {
  seq: number;
  reset: boolean;
  silent: boolean;
  sampleRateHz: number;
  fftSize: number;
  window: SpectrumWindowDto;
  response: AnalyzerResponseDto;
  levelsDb: Float32Array;
}

export interface LtasCurveFrame {
  jobId: number;
  index: number;
  sampleRateHz: number;
  fftSize: number;
  window: SpectrumWindowDto;
  levelsDb: Float32Array;
  /** The room-tone (quiet frames) spectrum, when there were quiet frames. */
  noiseDb: Float32Array | null;
}

function magicOf(dv: DataView): string {
  return String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
}

function readF32(dv: DataView, offset: number, n: number): Float32Array {
  const out = new Float32Array(n);
  for (let k = 0; k < n; k++) {
    out[k] = dv.getFloat32(offset + 4 * k, true);
  }
  return out;
}

function header(buf: ArrayBuffer, magic: string, minLen: number): DataView | null {
  if (buf.byteLength < minLen) {
    return null;
  }
  const dv = new DataView(buf);
  const headerLen = dv.getUint16(6, true);
  if (magicOf(dv) !== magic || dv.getUint16(4, true) !== 1 || headerLen < minLen || headerLen > buf.byteLength) {
    return null;
  }
  return dv;
}

/** Decodes a `VXIS` frame, or `null` if `buf` isn't a complete one. */
export function decodeVxis(buf: ArrayBuffer): InspectorFrame | null {
  const dv = header(buf, "VXIS", 40);
  if (!dv) {
    return null;
  }
  const headerLen = dv.getUint16(6, true);
  const bins = dv.getUint32(28, true);
  if (headerLen + 4 * bins > buf.byteLength) {
    return null;
  }
  const flags = dv.getUint32(12, true);
  return {
    seq: dv.getUint32(8, true),
    reset: (flags & 1) !== 0,
    silent: (flags & 2) !== 0,
    sampleRateHz: dv.getUint32(16, true),
    fftSize: dv.getUint32(20, true),
    window: SPECTRUM_WINDOW_CODES[dv.getUint32(24, true)] ?? "hann",
    response: RESPONSE_CODES[dv.getUint32(32, true)] ?? "medium",
    levelsDb: readF32(dv, headerLen, bins),
  };
}

/** Decodes a `VXLT` curve, or `null` if `buf` isn't a complete one. */
export function decodeVxlt(buf: ArrayBuffer): LtasCurveFrame | null {
  const dv = header(buf, "VXLT", 36);
  if (!dv) {
    return null;
  }
  const headerLen = dv.getUint16(6, true);
  const bins = dv.getUint32(28, true);
  const hasNoise = (dv.getUint32(32, true) & 1) !== 0;
  if (headerLen + 4 * bins * (hasNoise ? 2 : 1) > buf.byteLength) {
    return null;
  }
  return {
    jobId: dv.getUint32(8, true),
    index: dv.getUint32(12, true),
    sampleRateHz: dv.getUint32(16, true),
    fftSize: dv.getUint32(20, true),
    window: SPECTRUM_WINDOW_CODES[dv.getUint32(24, true)] ?? "hann",
    levelsDb: readF32(dv, headerLen, bins),
    noiseDb: hasNoise ? readF32(dv, headerLen + 4 * bins, bins) : null,
  };
}

/** Bin centre frequencies `k · fs / N` for `bins` bins. */
export function binFrequencies(bins: number, sampleRateHz: number, fftSize: number): Float64Array {
  const out = new Float64Array(bins);
  const binHz = sampleRateHz / Math.max(1, fftSize);
  for (let k = 0; k < bins; k++) {
    out[k] = k * binHz;
  }
  return out;
}
