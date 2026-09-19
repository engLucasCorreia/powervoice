/**
 * Synthetic analyzer data for the `?preview&scene=analyzer…` pages (H-42): a narrator's voice
 * spectrum (a 131 Hz harmonic series shaped by /a/-like formants, breath and sibilance noise, a
 * pink room tone and 50 Hz mains hum), encoded in the same `VXSA` / `VXIS` / `VXLT` frames the
 * engine sends, plus a matching voice report. Dev-only (never bundled into the app's code path).
 */
import type { VoiceReportDto } from "../lib/ipc/bindings";

const F0_HZ = 130.8;
const FORMANTS: Array<[number, number, number]> = [
  [720, 110, 14],
  [1240, 140, 11],
  [2650, 260, 8],
  [3500, 320, 4],
];

function db(p: number): number {
  return p > 0 ? 10 * Math.log10(p) : -Infinity;
}

/** Power spectrum on an FFT grid: bins `k · fs / fft`. `tilt` shapes the voice for A/B. */
export function voiceBins(fftSize: number, fs: number, options: { mud?: number; air?: number; hum?: boolean } = {}): Float32Array {
  const bins = fftSize / 2 + 1;
  const binHz = fs / fftSize;
  const p = new Float64Array(bins);
  const mud = options.mud ?? 0;
  const air = options.air ?? 0;
  // Room tone (pink-ish) + breath/sibilance noise.
  for (let k = 1; k < bins; k++) {
    const f = k * binHz;
    const oct = Math.log2(Math.max(f, 20) / 1000);
    p[k] = (p[k] ?? 0) + 10 ** ((-96 - 3 * oct) / 10);
    const sib = -62 + 13 * Math.exp(-((Math.log2(f / 6300)) ** 2) / (2 * 0.28 ** 2)) + air * Math.max(0, Math.log2(f / 8000));
    if (f > 1800) {
      p[k] = (p[k] ?? 0) + 10 ** (sib / 10);
    }
  }
  // Harmonics, smeared by intonation over time (σ ≈ 1.5 %).
  for (let h = 1; h * F0_HZ < fs / 2 - 200; h++) {
    const fh = h * F0_HZ;
    let level = -17 - 11 * Math.log2(h);
    for (const [fc, bw, gain] of FORMANTS) {
      level += gain * Math.exp(-(((fh - fc) / bw) ** 2));
    }
    level += mud * Math.exp(-((Math.log2(fh / 320)) ** 2) / (2 * 0.6 ** 2));
    const sigma = 0.015 * fh + 1.5;
    const amp = 10 ** (level / 10);
    const lo = Math.max(1, Math.floor((fh - 5 * sigma) / binHz));
    const hi = Math.min(bins - 1, Math.ceil((fh + 5 * sigma) / binHz));
    for (let k = lo; k <= hi; k++) {
      const d = k * binHz - fh;
      p[k] = (p[k] ?? 0) + amp * Math.exp(-(d * d) / (2 * sigma * sigma)) * Math.min(1, binHz / (sigma * 2.5));
    }
  }
  if (options.hum ?? true) {
    for (const [fh, level] of [
      [50, -71],
      [100, -77],
      [150, -74],
    ] as const) {
      const k = Math.round(fh / binHz);
      for (let d = -2; d <= 2; d++) {
        if (k + d > 0 && k + d < bins) {
          p[k + d] = (p[k + d] ?? 0) + 10 ** ((level - 6 * d * d) / 10);
        }
      }
    }
  }
  return Float32Array.from(p, db);
}

/** Room tone alone (the quiet-frame spectrum) on the same grid. */
export function roomToneBins(fftSize: number, fs: number): Float32Array {
  const bins = fftSize / 2 + 1;
  const binHz = fs / fftSize;
  const out = new Float32Array(bins);
  for (let k = 0; k < bins; k++) {
    const f = Math.max(k * binHz, 20);
    let p = 10 ** ((-98 - 3 * Math.log2(f / 1000)) / 10);
    for (const [fh, level] of [
      [50, -71],
      [100, -77],
      [150, -74],
    ] as const) {
      p += 10 ** ((level - 6 * ((k * binHz - fh) / binHz) ** 2) / 10);
    }
    out[k] = db(p);
  }
  return out;
}

/** The analyzer's 1/24-octave bands (max of the bins in each band, SPEC-007 §4.8.3). */
export function voiceBands(fs: number, jitterSeed: number): Float32Array {
  const fft = 8192;
  const bins = voiceBinsCached(fft, fs);
  const binHz = fs / fft;
  const fMax = Math.min(fs / 2, 24_000);
  const out: number[] = [];
  let rng = jitterSeed * 9301 + 49297;
  for (let k = 0; ; k++) {
    const fc = 20 * 2 ** (k / 24);
    if (fc > fMax) {
      break;
    }
    const lo = Math.ceil((fc * 2 ** (-1 / 48)) / binHz);
    const hi = Math.floor((fc * 2 ** (1 / 48)) / binHz);
    let v = -Infinity;
    for (let b = lo; b <= hi; b++) {
      v = Math.max(v, bins[b] ?? -Infinity);
    }
    if (!Number.isFinite(v)) {
      v = bins[Math.round(fc / binHz)] ?? -120;
    }
    rng = (rng * 9301 + 49297) % 233_280;
    out.push(v + (rng / 233_280 - 0.5) * 1.6);
  }
  return Float32Array.from(out);
}

const cache = new Map<string, Float32Array>();
export function voiceBinsCached(fft: number, fs: number, variant: "live" | "source" | "processed" = "live"): Float32Array {
  const key = `${fft}:${fs}:${variant}`;
  let v = cache.get(key);
  if (!v) {
    v = voiceBins(fft, fs, variant === "source" ? { mud: 6, air: -4 } : variant === "processed" ? { mud: -1, air: 2, hum: false } : {});
    cache.set(key, v);
  }
  return v;
}

function header(magic: string, len: number): { buf: ArrayBuffer; dv: DataView } {
  const buf = new ArrayBuffer(len);
  const dv = new DataView(buf);
  for (let i = 0; i < 4; i++) {
    dv.setUint8(i, magic.charCodeAt(i));
  }
  dv.setUint16(4, 1, true);
  return { buf, dv };
}

export function vxsaFrame(seq: number, fs: number, levels: Float32Array): ArrayBuffer {
  const { buf, dv } = header("VXSA", 48 + 4 * levels.length);
  dv.setUint16(6, 48, true);
  dv.setUint32(8, seq, true);
  dv.setUint32(12, 0, true);
  dv.setUint32(24, fs, true);
  dv.setUint32(28, 8192, true);
  dv.setFloat32(32, 20, true);
  dv.setUint32(36, 24, true);
  dv.setUint32(40, levels.length, true);
  dv.setUint32(44, 1, true);
  levels.forEach((v, k) => dv.setFloat32(48 + 4 * k, v, true));
  return buf;
}

export function vxisFrame(seq: number, fs: number, fft: number, window: number, response: number, levels: Float32Array): ArrayBuffer {
  const { buf, dv } = header("VXIS", 40 + 4 * levels.length);
  dv.setUint16(6, 40, true);
  dv.setUint32(8, seq, true);
  dv.setUint32(16, fs, true);
  dv.setUint32(20, fft, true);
  dv.setUint32(24, window, true);
  dv.setUint32(28, levels.length, true);
  dv.setUint32(32, response, true);
  levels.forEach((v, k) => dv.setFloat32(40 + 4 * k, v, true));
  return buf;
}

export function vxltFrame(jobId: number, index: number, fs: number, fft: number, levels: Float32Array, noise: Float32Array | null): ArrayBuffer {
  const n = levels.length;
  const { buf, dv } = header("VXLT", 36 + 4 * n * (noise ? 2 : 1));
  dv.setUint16(6, 36, true);
  dv.setUint32(8, jobId, true);
  dv.setUint32(12, index, true);
  dv.setUint32(16, fs, true);
  dv.setUint32(20, fft, true);
  dv.setUint32(28, n, true);
  dv.setUint32(32, noise ? 1 : 0, true);
  levels.forEach((v, k) => dv.setFloat32(36 + 4 * k, v, true));
  noise?.forEach((v, k) => dv.setFloat32(36 + 4 * (n + k), v, true));
  return buf;
}

export const PREVIEW_VOICE_REPORT: VoiceReportDto = {
  f0: { current_hz: 131.4, median_hz: 128.9, low_hz: 112.3, high_hz: 151.7, voiced_fraction: 0.64, confidence: 0.9, octave_corrected: 0 },
  tone: { mud_db: 7.2, presence_db: -9.1, air_db: -24.5 },
  sibilance: { ratio_db: -14.8, centre_hz: 6310 },
  hum: { mains_hz: 50, harmonics: [1, 2, 3], strongest_hz: 100.1, prominence_db: 21.4, level_db: -71.2 },
  rumble_db: -31.5,
  noise_floor_dbfs: -66.4,
  active_level_dbfs: -20.8,
  snr_db: 45.6,
  span_s: 9.7,
};

export const PREVIEW_AVERAGE_REPORTS: Record<"source" | "processed", VoiceReportDto> = {
  source: {
    ...PREVIEW_VOICE_REPORT,
    f0: { current_hz: null, median_hz: 129.6, low_hz: 110.8, high_hz: 154.2, voiced_fraction: 0.61, confidence: 0.9, octave_corrected: 0 },
    tone: { mud_db: 10.4, presence_db: -11.2, air_db: -27.9 },
    span_s: 94.6,
  },
  processed: {
    ...PREVIEW_VOICE_REPORT,
    f0: { current_hz: null, median_hz: 129.6, low_hz: 110.8, high_hz: 154.2, voiced_fraction: 0.61, confidence: 0.9, octave_corrected: 0 },
    tone: { mud_db: 3.1, presence_db: -6.4, air_db: -19.8 },
    hum: null,
    noise_floor_dbfs: -71.9,
    snr_db: 51.1,
    span_s: 94.6,
  },
};
