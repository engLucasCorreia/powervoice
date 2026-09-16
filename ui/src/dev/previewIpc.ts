/**
 * Dev-only preview (H-25, H-26): mounts the real App against mocked IPC so the shell can be
 * opened — and screenshotted — in a plain browser, without a Tauri backend or audio devices.
 * `main.ts` loads this only for `?preview` when `import.meta.env.DEV` (never in a build):
 *
 *   npm --prefix ui run dev → http://localhost:1420/?preview
 *   &theme=dark | light | system | high_contrast   (T-708; default dark)
 *
 * H-26 adds fixture content, driven by `previewScenes.ts` after the App mounts:
 *   &scene=document | spectral | recording | rack | loudness   (combine: scene=rack,loudness)
 *          | plugins | plugins-scanning | plugins-folders   (T-809: the plugin manager, every
 *            status; with `rack`, a flagged CLAP slot) | plugin-flag   (with `rack`: the flagged
 *            slot, manager closed)
 *   &dialog=about | preferences | export | new-recording | calibration | normalize |
 *           normalize-lufs | recovery | recovery-storage | save-as | unsaved | recent-missing |
 *           audio-devices | confirm | channel-choice | clip | low-disk |
 *           plugin-install | plugin-collision | plugin-failed   (T-809 "Install module…") |
 *           bake   (T-602, with `scene=rack`: the noise-only confirm or the progress dialog)
 *   &menu=file | edit | view | effects | help | normalize | add-module | rack-slot
 *        | theme   (T-708: View → Theme ▸ open)
 *   &scene=tour&step=n[&tour=welcome|rack|noise|loudness|punch|plugins]   (T-709)
 *   &renderer=auto|webgl2|canvas2d   (T-704: the renderer Setting; default canvas2d, which keeps
 *                 screenshots deterministic — the app's own default is auto, i.e. WebGL2 first)
 *   &doc=60min   (T-704: a 60-minute document, zoomed to fit, whose peaks come from a full
 *                 precomputed pyramid — the frame-time sweep `scripts/bench/ui_frames.mjs` uses it)
 *   &dialog=tour-offer   (T-709: the first-run Welcome tour offer)
 * The audio is synthetic (a narrator's phrases with breaths), generated here as the same binary
 * frames the backend sends (VXPK peaks, VXST spectrogram tiles, VXTM telemetry).
 *
 * H-31: a command this file has no case for is a bug (H-32's EQ-graph-goes-blank root cause was
 * exactly this — a silent `null` where a store expected a real DTO, corrupting its state). The
 * default case now says so loudly: a `console.error` always, and — since a preview screenshot run
 * must never depend on interactively noticing that error — a thrown `Error` under Vitest
 * (`import.meta.env.MODE === "test"`) so a missing case fails the test that exercises it, not
 * just this file's own dev console.
 */
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import {
  PREVIEW_AVERAGE_REPORTS,
  PREVIEW_VOICE_REPORT,
  roomToneBins,
  voiceBands,
  voiceBinsCached,
  vxisFrame,
  vxltFrame,
  vxsaFrame,
} from "./previewSpectrum";
import { docDto, rackSlotDto, rackStateDto, recordStateDto, settingsFixture, transportStateDto } from "../lib/test/fixtures";
import type {
  AcxCheckReportDto,
  DevicesDto,
  DocumentDto,
  DocumentProbeDto,
  IpcError,
  MarkerDto,
  ModuleDescriptorDto,
  ParamInfoDto,
  ParamValueDto,
  PluginEntryDto,
  PluginFoldersDto,
  PluginInstallResultDto,
  RackSlotDto,
  RackStateDto,
  RecordOffsetDto,
  RecordStateDto,
  RecoverableSessionDto,
  ResponseCurveDto,
  Settings,
  StorageInfoDto,
  ThemePref,
  TransportStateDto,
  UnitDto,
} from "../lib/ipc/bindings";
import { formatWithUnit } from "../lib/ui/units";

export const PREVIEW_RATE_HZ = 48_000;
export const PREVIEW_LEN_SAMPLES = 95 * PREVIEW_RATE_HZ;
export const PREVIEW_PATH = "/home/narrator/audiobook/chapter-03.wav";
/** T-704 (`&doc=60min`): the long preview document, PROMPT §2's 60-min 48 kHz performance case. */
export const PREVIEW_LONG_LEN_SAMPLES = 3600 * PREVIEW_RATE_HZ;
/** ADR-004 §5 pyramid levels (samples per bucket). */
const PYRAMID_LEVELS_SPP = [64, 256, 1024, 4096, 16_384, 65_536] as const;
/** Samples already recorded in the recording scene when the page opens. */
const RECORDED_SAMPLES = 754_000;
const LIVE_PEAKS_SPB = 256;
const TILE_FRAMES = 256;

export interface PreviewOptions {
  theme: ThemePref;
  scenes: string[];
  dialog: string | null;
  /** T-704 `&doc=60min`: open a 60-minute document instead of the 95 s chapter. */
  longDocument?: boolean;
  /** T-704 `&renderer=`: the waveform/spectral renderer Setting (default `canvas2d`). */
  renderer?: Settings["renderer_preference"];
}

// --- Synthetic narration --------------------------------------------------------------------

/** Linear amplitude envelope of a narrator: ~2.6 s phrases, ~0.6 s breaths, ~4 syllables/s. */
export function narrationLevel(t: number): number {
  if (t < 0.5) {
    return 0.003;
  }
  const p = (t - 0.5) % 3.2;
  if (p > 2.6) {
    return 0.003 + 0.002 * Math.abs(Math.sin(t * 50));
  }
  const phrase = Math.sin((Math.PI * p) / 2.6) ** 0.4;
  const syllable = 0.3 + 0.7 * Math.abs(Math.sin(Math.PI * 4.3 * t + 1.7 * Math.sin(0.9 * t)));
  const colour = 0.6 + 0.4 * Math.sin(0.37 * t) ** 2;
  return 0.72 * phrase * syllable * colour;
}

function bucketPeaks(startSample: number, spp: number, count: number): Float32Array {
  const out = new Float32Array(count * 2);
  for (let i = 0; i < count; i++) {
    const s0 = startSample + i * spp;
    let amp = 0;
    for (let k = 0; k < 4; k++) {
      amp = Math.max(amp, narrationLevel((s0 + (k * spp) / 4) / PREVIEW_RATE_HZ));
    }
    out[i * 2] = -amp * 0.93;
    out[i * 2 + 1] = amp;
  }
  return out;
}

/**
 * T-704: the long preview document's whole min/max pyramid, built once per level on first use:
 * level 64 from the narration envelope, each coarser level from its four children — the shape of
 * ADR-004 §5's per-chunk pyramids. `peaks_get` is then a slice copy, like the backend's pyramid
 * read, instead of a per-request synthesis on the UI thread (which the real app never pays, and
 * which would otherwise dominate a frame-time sweep over a 60-min document).
 */
export class SyntheticPyramid {
  private readonly levels = new Map<number, Float32Array>();

  constructor(private readonly lenSamples: number) {}

  private level(spp: number): Float32Array {
    const cached = this.levels.get(spp);
    if (cached) {
      return cached;
    }
    const n = Math.ceil(this.lenSamples / spp);
    const data = new Float32Array(n * 2);
    if (spp === PYRAMID_LEVELS_SPP[0]) {
      for (let i = 0; i < n; i++) {
        const amp = narrationLevel((i * spp + spp / 2) / PREVIEW_RATE_HZ);
        data[i * 2] = -amp * 0.93;
        data[i * 2 + 1] = amp;
      }
    } else {
      const child = this.level(spp / 4);
      const childCount = child.length / 2;
      for (let i = 0; i < n; i++) {
        let min = Infinity;
        let max = -Infinity;
        for (let c = i * 4; c < Math.min(i * 4 + 4, childCount); c++) {
          min = Math.min(min, child[c * 2] ?? 0);
          max = Math.max(max, child[c * 2 + 1] ?? 0);
        }
        data[i * 2] = min;
        data[i * 2 + 1] = max;
      }
    }
    this.levels.set(spp, data);
    return data;
  }

  /** `count` `(min, max)` buckets of level `spp` from bucket `floor(startSample / spp)`; buckets
   * past the document end read `(0, 0)` (the backend's convention). */
  peaks(spp: number, startSample: number, count: number): Float32Array {
    const data = this.level(spp);
    const out = new Float32Array(count * 2);
    const first = Math.floor(startSample / spp);
    const available = Math.max(0, Math.min(count, data.length / 2 - first));
    if (available > 0) {
      out.set(data.subarray(first * 2, (first + available) * 2));
    }
    return out;
  }
}

/** T-704: the long document's raw samples repeat this many samples of the narration (a sweep over a
 * 60-min document asks for up to ~100 k raw samples per frame at close zoom; synthesizing them with
 * `Math.sin` on the UI thread is a preview-only cost the real backend never has). */
const LONG_RAW_PERIOD = 1 << 16;
let longRawBlock: Float32Array | null = null;

/** Raw samples of the long document: `rawSamples` over one block, repeated. */
function longRawSamples(startSample: number, count: number): Float32Array {
  longRawBlock ??= rawSamples(10 * PREVIEW_RATE_HZ, LONG_RAW_PERIOD);
  const out = new Float32Array(count);
  for (let i = 0; i < count; i++) {
    out[i] = longRawBlock[(startSample + i) % LONG_RAW_PERIOD]!;
  }
  return out;
}

function rawSamples(startSample: number, count: number): Float32Array {
  const out = new Float32Array(count);
  for (let i = 0; i < count; i++) {
    const t = (startSample + i) / PREVIEW_RATE_HZ;
    const voice = Math.sin(2 * Math.PI * 140 * t) + 0.5 * Math.sin(2 * Math.PI * 280 * t) + 0.3 * Math.sin(2 * Math.PI * 420 * t);
    out[i] = (narrationLevel(t) * voice) / 1.8;
  }
  return out;
}

// --- Binary frames ---------------------------------------------------------------------------

function writeMagic(view: DataView, magic: string): void {
  for (let i = 0; i < 4; i++) {
    view.setUint8(i, magic.charCodeAt(i));
  }
}

function vxpk(
  requestId: number,
  audioRev: number,
  startSample: number,
  spp: number,
  count: number,
  raw: boolean,
  values: Float32Array,
): ArrayBuffer {
  const buf = new ArrayBuffer(48 + values.byteLength);
  const view = new DataView(buf);
  writeMagic(view, "VXPK");
  view.setUint16(4, 1, true);
  view.setUint16(6, 48, true);
  view.setUint32(8, requestId, true);
  view.setUint32(12, raw ? 1 : 0, true);
  view.setBigUint64(16, BigInt(audioRev), true);
  view.setBigUint64(24, BigInt(startSample), true);
  view.setUint32(32, spp, true);
  view.setUint32(36, count, true);
  view.setUint32(40, PREVIEW_RATE_HZ, true);
  new Float32Array(buf, 48).set(values);
  return buf;
}

/** One spectrogram tile: harmonics of a gliding f0 under three formants, a tilt, breaths as
 * room tone and a few sibilants. */
function vxst(requestId: number, audioRev: number, fft: number, hop: number, tile: number, last: boolean): ArrayBuffer {
  const bins = fft / 2 + 1;
  const buf = new ArrayBuffer(64 + TILE_FRAMES * bins);
  const view = new DataView(buf);
  writeMagic(view, "VXST");
  view.setUint16(4, 1, true);
  view.setUint16(6, 64, true);
  view.setUint32(8, requestId, true);
  view.setUint32(12, last ? 1 : 0, true);
  view.setBigUint64(16, BigInt(audioRev), true);
  view.setBigUint64(24, BigInt(tile * TILE_FRAMES * hop), true);
  view.setUint32(32, hop, true);
  view.setUint32(36, fft, true);
  view.setUint32(40, TILE_FRAMES, true);
  view.setUint32(44, bins, true);
  view.setFloat32(48, -150, true);
  view.setFloat32(52, 6, true);
  view.setUint32(56, tile, true);
  view.setUint32(60, 0, true);
  const codes = new Uint8Array(buf, 64);
  for (let f = 0; f < TILE_FRAMES; f++) {
    const t = ((tile * TILE_FRAMES + f) * hop) / PREVIEW_RATE_HZ;
    const level = narrationLevel(t);
    const voiced = level > 0.02;
    const sibilant = voiced && (t * 1.7) % 1 < 0.06;
    const f0 = 125 + 22 * Math.sin(0.8 * t) + 10 * Math.sin(5.1 * t);
    const base = 20 * Math.log10(Math.max(level, 1e-5));
    for (let b = 0; b < bins; b++) {
      const hz = (b * PREVIEW_RATE_HZ) / fft;
      let db: number;
      if (!voiced) {
        db = -108 + 6 * Math.sin(b * 12.9898 + f * 78.233) ** 2 - hz / 4000;
      } else {
        const h = hz / f0;
        const dist = Math.abs(h - Math.round(h));
        const harmonic = h >= 0.5 ? Math.exp(-((dist * 5) ** 2)) * 26 - 26 : -30;
        const formants =
          10 * Math.exp(-(((hz - 600) / 260) ** 2)) +
          8 * Math.exp(-(((hz - 1650) / 380) ** 2)) +
          5 * Math.exp(-(((hz - 2750) / 450) ** 2));
        const tilt = -9 * Math.log2(Math.max(hz, 150) / 200);
        db = base - 6 + harmonic + formants + tilt;
        if (sibilant && hz > 4500) {
          db = Math.max(db, base - 18 - (hz - 7000) / 2500);
        }
        db = Math.max(db, -106 + 4 * Math.sin(b * 3.7 + f) ** 2);
      }
      codes[f * bins + b] = Math.max(0, Math.min(255, Math.round(((db + 150) / 156) * 255)));
    }
  }
  return buf;
}

/** T-704: generated tiles, keyed by FFT size/hop/index (a tile's content depends on nothing else),
 * so a re-request — e.g. the frame-time sweep's measured pass after its warm-up pass — costs a copy,
 * not a ~10 ms synthesis on the UI thread that the real app (Rust tile workers) never pays. */
const TILE_MEMO_CAP = 512;
const tileMemo = new Map<string, ArrayBuffer>();
/** T-704: the long document's tiles reuse the content of `tile % LONG_TILE_TEMPLATES` (headers are
 * still the requested tile's), so the frame-time sweep measures the renderer — tile decode and
 * upload — rather than ~10 ms of preview-only synthesis per new tile on the UI thread (the real
 * app computes tiles on Rust workers). */
const LONG_TILE_TEMPLATES = 16;
/**
 * H-47: the templated tiles are synthesized at **this** hop whatever hop was requested, so the
 * whole zoom sweep shares one set of {@link LONG_TILE_TEMPLATES} templates per FFT size instead of
 * re-synthesizing 16 of them at every new hop. Without it the sweep's own mock cost 77–97 % of the
 * self time inside every frame over 50 ms (H-47's CDP attribution): each Ctrl+wheel step picks a
 * new hop, and the 16 fresh ~10 ms syntheses landed on the UI thread in `setTimeout(0)` tasks —
 * a measurement artifact of the preview, not a renderer cost (the real app's tiles come from Rust
 * workers over IPC). The header still carries the requested hop, so geometry is unchanged; only
 * the fake content no longer varies with zoom.
 */
const LONG_TEMPLATE_HOP_DIV = 4;

function memoVxst(
  requestId: number,
  audioRev: number,
  fft: number,
  hop: number,
  tile: number,
  last: boolean,
  templated = false,
): ArrayBuffer {
  const source = templated ? tile % LONG_TILE_TEMPLATES : tile;
  const sourceHop = templated ? fft / LONG_TEMPLATE_HOP_DIV : hop;
  const key = `${fft}:${sourceHop}:${source}`;
  let base = tileMemo.get(key);
  if (!base) {
    base = vxst(0, 0, fft, sourceHop, source, false);
    if (tileMemo.size >= TILE_MEMO_CAP) {
      const oldest = tileMemo.keys().next().value;
      if (oldest !== undefined) {
        tileMemo.delete(oldest);
      }
    }
    tileMemo.set(key, base);
  }
  const buf = base.slice(0);
  const view = new DataView(buf);
  view.setUint32(8, requestId, true);
  view.setUint32(12, last ? 1 : 0, true);
  view.setBigUint64(16, BigInt(audioRev), true);
  view.setBigUint64(24, BigInt(tile * TILE_FRAMES * hop), true);
  view.setUint32(32, hop, true);
  view.setUint32(56, tile, true);
  return buf;
}

function vxtm(
  seq: number,
  flags: number,
  playhead: number,
  levels: [number, number, number, number],
  rate: number = PREVIEW_RATE_HZ,
): ArrayBuffer {
  const buf = new ArrayBuffer(72);
  const view = new DataView(buf);
  writeMagic(view, "VXTM");
  view.setUint16(4, 1, true);
  view.setUint16(6, 72, true);
  view.setUint32(8, seq, true);
  view.setUint32(12, flags, true);
  view.setBigUint64(16, BigInt(Math.round(playhead)), true);
  view.setBigUint64(24, BigInt(Math.round(performance.now() * 1e6)), true);
  view.setFloat64(32, rate, true);
  view.setFloat32(40, levels[0], true);
  view.setFloat32(44, levels[1], true);
  view.setFloat32(48, levels[2], true);
  view.setFloat32(52, levels[3], true);
  view.setBigUint64(56, 1n, true);
  view.setUint32(64, 0, true);
  return buf;
}

// --- Fixtures ----------------------------------------------------------------------------------

function documentFixture(options: PreviewOptions): DocumentDto {
  const recording = options.scenes.includes("recording");
  const long = options.longDocument === true && !recording;
  return docDto({
    name: recording ? null : "chapter-03.wav",
    path: recording ? null : PREVIEW_PATH,
    sample_rate_hz: PREVIEW_RATE_HZ,
    len_samples: recording ? 0 : long ? PREVIEW_LONG_LEN_SAMPLES : PREVIEW_LEN_SAMPLES,
    dirty: options.dialog === "unsaved",
    // T-704: the long document opens zoomed to fit (no stored view), like a first open.
    waveform_view: recording || long
      ? null
      : {
          start_sample: 0,
          samples_per_pixel: 4800,
          selection: { start_sample: 19 * PREVIEW_RATE_HZ, end_sample: 25.4 * PREVIEW_RATE_HZ },
          cursor_samples: 19 * PREVIEW_RATE_HZ,
          time_ruler_format: "timecode",
          vertical_zoom: 1,
        },
  });
}

function recordFixture(options: PreviewOptions): RecordStateDto {
  const recording = options.scenes.includes("recording");
  return recordStateDto({
    input_device: "Scarlett Solo USB",
    armed: recording,
    input_open: recording,
    input_rate_hz: PREVIEW_RATE_HZ,
    recording,
    monitor: recording ? "dry" : "off",
    monitoring: recording,
    monitor_latency_us: recording ? 9_800 : null,
    disk_remaining_s: options.dialog === "low-disk" ? 6 * 60 : 4 * 3600 + 12 * 60,
  });
}

const MARKERS: MarkerDto[] = [
  { id: 1, pos_samples: 4 * PREVIEW_RATE_HZ, len_samples: 0, name: "Chapter 3 — The Lighthouse", kind: "user" },
  { id: 2, pos_samples: 31 * PREVIEW_RATE_HZ, len_samples: 3 * PREVIEW_RATE_HZ, name: "Retake: breath", kind: "user" },
  { id: 3, pos_samples: 62 * PREVIEW_RATE_HZ, len_samples: 0, name: "Scene break", kind: "user" },
  { id: 4, pos_samples: 45 * PREVIEW_RATE_HZ, len_samples: 0, name: "Dropout 8 ms", kind: "dropout" },
];

const text = (value: string) => ({ text: value, key: null });

const FLAGS = { automatable: true, stepped: false, boolean: false, read_only: false, hidden: false, bypass: false };

function unitLabel(unit: UnitDto): string {
  switch (unit.kind) {
    case "hz":
      return "Hz";
    case "db":
      return "dB";
    case "dbtp":
      return "dBTP";
    case "ms":
      return "ms";
    case "ratio":
      return ":1";
    default:
      return "";
  }
}

function param(
  id: number,
  key: string,
  name: string,
  unit: UnitDto,
  min: number,
  max: number,
  value: number,
  decimals: number,
  log = false,
): { info: ParamInfoDto; value: ParamValueDto } {
  const normalized = log ? Math.log(value / min) / Math.log(max / min) : (value - min) / (max - min);
  const label = unitLabel(unit);
  return {
    info: {
      id,
      key,
      name: text(name),
      group: null,
      unit,
      min,
      max,
      default: value,
      taper: log ? { kind: "log" } : { kind: "linear" },
      step: null,
      enum_labels: [],
      decimals,
      smoothing_ms: 20,
      flags: FLAGS,
    },
    value: {
      id,
      value,
      normalized,
      text: label === ":1" ? `${formatWithUnit(value, "", decimals)}:1` : formatWithUnit(value, label, decimals),
    },
  };
}

function slot(
  uid: number,
  moduleId: string,
  name: string,
  params: { info: ParamInfoDto; value: ParamValueDto }[],
  extra: Partial<RackSlotDto> = {},
): RackSlotDto {
  return rackSlotDto({
    uid,
    module: `${moduleId}@1.0.0`,
    module_id: moduleId,
    name,
    params: params.map((p) => p.info),
    values: params.map((p) => p.value),
    ...extra,
  });
}

const GR = (max = 0, min = -24) => ({
  id: 0,
  key: "gain_reduction",
  name: text("Gain reduction"),
  unit: { kind: "db" } as UnitDto,
  min,
  max,
  kind: "gain_reduction" as const,
  group: null,
});

const EQ_BANDS = { hp: 80, b1f: 250, b1g: -3, b1q: 1.2, b2f: 3200, b2g: 2.5, b2q: 1, hsf: 10_000, hsg: 1.5 };

function rackFixture(withPlugin: boolean): RackStateDto {
  const hz = { kind: "hz" } as UnitDto;
  const db = { kind: "db" } as UnitDto;
  const ms = { kind: "ms" } as UnitDto;
  const none = { kind: "none" } as UnitDto;
  const eq = slot(
    1,
    "org.powervoice.parametric-eq",
    "Parametric EQ",
    [
      param(0, "hp_freq_hz", "High-pass", hz, 20, 1000, EQ_BANDS.hp, 0, true),
      param(1, "b1_freq_hz", "Band 1 frequency", hz, 20, 20_000, EQ_BANDS.b1f, 0, true),
      param(2, "b1_gain_db", "Band 1 gain", db, -24, 24, EQ_BANDS.b1g, 1),
      param(3, "b1_q", "Band 1 Q", none, 0.1, 10, EQ_BANDS.b1q, 2, true),
      param(4, "b2_freq_hz", "Band 2 frequency", hz, 20, 20_000, EQ_BANDS.b2f, 0, true),
      param(5, "b2_gain_db", "Band 2 gain", db, -24, 24, EQ_BANDS.b2g, 1),
      param(6, "b2_q", "Band 2 Q", none, 0.1, 10, EQ_BANDS.b2q, 2, true),
      param(7, "hs_freq_hz", "High shelf", hz, 1000, 20_000, EQ_BANDS.hsf, 0, true),
      param(8, "hs_gain_db", "High-shelf gain", db, -24, 24, EQ_BANDS.hsg, 1),
    ],
    {
      curve_handles: [
        { component: 0, freq: 0, gain: null, q: null, enable: null },
        { component: 1, freq: 1, gain: 2, q: 3, enable: null },
        { component: 2, freq: 4, gain: 5, q: 6, enable: null },
        { component: 3, freq: 7, gain: 8, q: null, enable: null },
      ],
    },
  );
  const gate = slot(2, "org.powervoice.noise-gate", "Noise gate", [
    param(0, "threshold_db", "Threshold", db, -80, 0, -52, 1),
    param(1, "range_db", "Range", db, -80, 0, -18, 1),
    param(2, "release_ms", "Release", ms, 5, 1000, 120, 0, true),
  ], { telemetry: [GR(0, -80)] });
  const comp = slot(3, "org.powervoice.dynamics", "Dynamics", [
    param(0, "threshold_db", "Threshold", db, -60, 0, -22, 1),
    param(1, "ratio", "Ratio", { kind: "ratio" }, 1, 20, 3, 1, true),
    param(2, "attack_ms", "Attack", ms, 0.1, 100, 8, 1, true),
    param(3, "release_ms", "Release", ms, 10, 1000, 140, 0, true),
    param(4, "makeup_db", "Make-up gain", db, 0, 24, 4, 1),
  ], { telemetry: [GR()] });
  const limiter = slot(4, "org.powervoice.true-peak-limiter", "True-peak limiter", [
    param(0, "ceiling_dbtp", "Ceiling", { kind: "dbtp" }, -12, 0, -3, 1),
    param(1, "release_ms", "Release", ms, 1, 500, 50, 0, true),
  ], { telemetry: [GR()], latency_samples: 64 });
  // T-809: with the plugins scene, a sandboxed CLAP effect that has crashed before (flagged).
  const breath = slot(5, PREVIEW_FLAGGED_PLUGIN, "Breath Control", [
    param(0, "p0", "Reduction", db, -30, 0, -12, 1),
    param(1, "p1", "Sensitivity", none, 0, 100, 60, 0),
  ], { sandboxed: true, has_editor: true, latency_samples: 256 });
  const slots = withPlugin ? [eq, gate, comp, limiter, breath] : [eq, gate, comp, limiter];
  return rackStateDto(slots, false, withPlugin ? 320 : 64);
}

// --- Plugins (T-809) ---------------------------------------------------------------------------

const HOME = "/home/narrator";
export const PREVIEW_FLAGGED_PLUGIN = "clap:com.vocalift.breath-control";
export const PREVIEW_INSTALL_SOURCE = `${HOME}/Downloads/acme-deesser.clap`;

function plugin(
  id: string,
  name: string,
  vendor: string,
  version: string,
  format: string,
  path: string,
  status: PluginEntryDto["status"],
  ports: [number, number] | null,
  params: number,
): PluginEntryDto {
  return {
    id,
    name,
    vendor,
    version,
    format,
    path,
    status,
    ports: ports ? { input_channels: ports[0], output_channels: ports[1] } : null,
    param_count: params,
  };
}

/** One of every status (and every format badge the backends will report). */
const PLUGINS: PluginEntryDto[] = [
  plugin("clap:com.acme.deesser", "De-esser", "Acme Audio", "2.1.0", "clap", `${HOME}/.clap/acme-deesser.clap`, { kind: "ok" }, [1, 1], 12),
  plugin(PREVIEW_FLAGGED_PLUGIN, "Breath Control", "Vocalift", "1.4.2", "clap", `${HOME}/.clap/vocalift/breath-control.clap`, { kind: "flagged", crash_count: 3 }, [2, 2], 8),
  plugin("clap:org.studio.voice-eq", "Voice EQ", "Studio Tools", "0.9.0", "clap", "/usr/lib/clap/studio-tools.clap", { kind: "ok" }, null, 0),
  plugin("vst3:northwind.tape", "Tape Saturator", "Northwind DSP", "3.0.1", "vst3", "/usr/lib/vst3/Tape Saturator.vst3", { kind: "disabled" }, [2, 2], 24),
  plugin("lv2:urn:studio:room", "Small Room", "Studio Tools", "1.0.0", "lv2", "/usr/lib/lv2/small-room.lv2", { kind: "ok" }, [1, 2], 9),
  plugin("jsfx:loudness-rider", "Loudness Rider", "JSFX Community", "1.2.0", "jsfx", `${HOME}/.config/powervoice/jsfx/loudness-rider.jsfx`, { kind: "ok" }, [2, 2], 6),
  plugin("clap:com.acme.hum", "Hum Remover", "Acme Audio", "1.0.3", "clap", "/media/plugins/voice-tools/hum-remover.clap", { kind: "blocklisted", reason: "blocked by the user", cause: "manual" }, [1, 1], 5),
  plugin("", "glitchy-comp", "", "", "clap", `${HOME}/Downloads/glitchy-comp.clap`, { kind: "blocklisted", reason: "crashed while being scanned", cause: "crashed" }, null, 0),
  plugin("", "slow-limiter", "", "", "clap", "/media/plugins/voice-tools/slow-limiter.clap", { kind: "blocklisted", reason: "timed out while being scanned", cause: "timed_out" }, null, 0),
];

const PLUGIN_FOLDERS: PluginFoldersDto = {
  install: `${HOME}/.clap`,
  install_folders: [`${HOME}/.clap`, `${HOME}/.vst3`],
  standard: [`${HOME}/.clap`, "/usr/lib/clap", `${HOME}/.vst3`, "/usr/lib/vst3"],
  custom: ["/media/plugins/voice-tools"],
  modules: `${HOME}/.local/share/app.powervoice.editor/modules`,
};

function installResult(dialog: string | null, replace: boolean): PluginInstallResultDto {
  if (dialog === "plugin-collision" && !replace) {
    return { kind: "collision", path: `${HOME}/.clap/acme-deesser.clap`, installed_version: null, new_version: null };
  }
  if (dialog === "plugin-failed") {
    return {
      kind: "failed",
      code: "scan_crashed",
      detail: "crashed while being scanned (signal 11)",
      blocklisted: true,
      cause: "crashed",
    };
  }
  return {
    kind: "installed",
    path: `${HOME}/.clap/acme-deesser.clap`,
    replaced: replace,
    effects: [
      { id: "clap:com.acme.deesser", name: "De-esser" },
      { id: "clap:com.acme.deesser-stereo", name: "De-esser (stereo)" },
    ],
  };
}

const MODULES: ModuleDescriptorDto[] = [
  ["org.powervoice.parametric-eq", "Parametric EQ", ["equalizer"]],
  ["org.powervoice.noise-reduction", "Noise reduction", ["restoration"]],
  ["org.powervoice.noise-gate", "Noise gate", ["gate"]],
  ["org.powervoice.dynamics", "Dynamics", ["compressor"]],
  ["org.powervoice.true-peak-limiter", "True-peak limiter", ["limiter", "mastering"]],
  ["org.powervoice.gain", "Gain", ["utility"]],
  ["clap:com.example.deesser", "De-esser", ["audio-effect", "restoration"]],
].map(([id, name, features]) => ({
  id: id as string,
  name: text(name as string),
  vendor: "PowerVoice",
  description: text(""),
  features: features as string[],
}));

function responseCurve(points: number[]): ResponseCurveDto {
  const bell = (f: number, fc: number, gain: number, q: number) =>
    gain * Math.exp(-(Math.log2(f / fc) ** 2) / (2 * (0.9 / q) ** 2));
  const components = [
    points.map((f) => -10 * Math.log10(1 + (EQ_BANDS.hp / f) ** 4)),
    points.map((f) => bell(f, EQ_BANDS.b1f, EQ_BANDS.b1g, EQ_BANDS.b1q)),
    points.map((f) => bell(f, EQ_BANDS.b2f, EQ_BANDS.b2g, EQ_BANDS.b2q)),
    points.map((f) => EQ_BANDS.hsg / (1 + (EQ_BANDS.hsf / f) ** 2)),
  ];
  return {
    freqs_hz: points,
    sample_rate_hz: PREVIEW_RATE_HZ,
    total_db: points.map((_, i) => components.reduce((sum, c) => sum + c[i]!, 0)),
    components_db: components,
  };
}

const ACX: AcxCheckReportDto = {
  passes: false,
  rms: { measured_db: -20.4, status: "pass" },
  peak: { measured_db: -3.1, status: "pass" },
  noise_floor: { measured_db: -57.2, status: "too_high" },
};

const SESSION: RecoverableSessionDto = {
  id: "s-1",
  name: null,
  path: null,
  last_modified_unix_ms: Date.UTC(2026, 8, 14, 22, 21, 56),
  unsaved_changes: 2,
  recording_samples: 38 * PREVIEW_RATE_HZ,
  sample_rate_hz: PREVIEW_RATE_HZ,
  source_changed: false,
  source_missing: false,
  damaged: false,
  size_bytes: 70 * 1024 * 1024,
};

const STORAGE: StorageInfoDto = {
  session_bytes: 212 * 1024 * 1024,
  history_bytes: 38 * 1024 * 1024,
  sessions: [SESSION],
  recovery_bytes: 70 * 1024 * 1024,
};

const OFFSET: RecordOffsetDto = {
  available: true,
  host: "pipewire",
  input_device: "Scarlett Solo USB",
  output_device: "Scarlett Solo USB",
  device_rate_hz: PREVIEW_RATE_HZ,
  offset_ms: -1.25,
  source: "calibrated",
  updated_unix_ms: Date.UTC(2026, 8, 2),
  confidence: 0.98,
  buffer_frames: 256,
  current_buffer_frames: 256,
};

const DEVICES: DevicesDto = {
  hosts: ["pipewire", "alsa"],
  host: "pipewire",
  devices: [
    {
      name: "Scarlett Solo USB",
      base_name: "Scarlett Solo USB",
      input: true,
      output: true,
      input_channels: 2,
      system_default: true,
      input_is_monitor: false,
      output_rates_hz: [44_100, 48_000, 96_000],
      output_buffer_sizes: [64, 128, 256, 512],
    },
    {
      name: "Built-in Audio",
      base_name: "Built-in Audio",
      input: true,
      output: true,
      input_channels: 2,
      system_default: false,
      input_is_monitor: false,
      output_rates_hz: [44_100, 48_000],
      output_buffer_sizes: [256, 512, 1024],
    },
  ],
  default_input: "Scarlett Solo USB",
  default_output: "Scarlett Solo USB",
  prefs: {
    host: "pipewire",
    input_device: "Scarlett Solo USB",
    input_channel: 1,
    output_device: "Scarlett Solo USB",
    sample_rate_hz: PREVIEW_RATE_HZ,
    buffer_size_frames: 256,
  },
  output_device: "Scarlett Solo USB",
  output_rate_hz: PREVIEW_RATE_HZ,
  output_buffer_frames: 256,
  output_status: "healthy",
  input_device: "Scarlett Solo USB",
  input_status: "healthy",
};

const PROBE: DocumentProbeDto = {
  container: "WAV",
  codec: "PCM 24-bit",
  sample_rate_hz: PREVIEW_RATE_HZ,
  channels: [
    { label: "Left", is_lfe: false },
    { label: "Right", is_lfe: false },
  ],
  len_samples: PREVIEW_LEN_SAMPLES,
  channel_peaks_dbfs: [-3.1, -4.2],
  identical_channels: false,
  suggested_channel: 0,
};

function needsConfirmation(key: string, params: Record<string, string>): IpcError {
  return { code: "needs_confirmation", key, params };
}

type Sink = { onmessage: (message: ArrayBuffer) => void };

export function installPreviewIpc(options: PreviewOptions): void {
  const { theme, scenes, dialog } = options;
  const hasScene = (name: string) => scenes.includes(name);
  const analyzerScene = scenes.some((s) => s.startsWith("analyzer"));
  const inspectorConfig = { fft: 16_384, window: 0, response: 1 };
  const settings: Settings = settingsFixture({
    device: {
      host: "pipewire",
      input_device: "Scarlett Solo USB",
      input_channel: 1,
      output_device: "Scarlett Solo USB",
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    monitor_hint_shown: true,
    renderer_preference: options.renderer ?? "canvas2d",
    layout: {
      markers_width_px: 240,
      rack_width_px: 300,
      dock_height_px: analyzerScene ? 300 : 240,
      markers_collapsed: false,
      rack_collapsed: false,
      dock_tab: hasScene("loudness") ? "loudness" : "meters",
    },
    theme,
    // H-42: `&scene=analyzer` streams a synthetic voice into the analyzer; `analyzer-diag` shows
    // the diagnostics panel; the dock gets a little taller so the details read.
    analyzer_diagnostics: {
      ...settingsFixture().analyzer_diagnostics,
      panel_visible: hasScene("analyzer-diag"),
    },
    // T-709: the Welcome offer only in `&dialog=tour-offer`; everywhere else it's been answered.
    tours: { progress: dialog === "tour-offer" ? [] : [{ id: "welcome", version: 1, outcome: "dismissed" }] },
  });

  const doc = documentFixture(options);
  const pyramid = options.longDocument === true ? new SyntheticPyramid(doc.len_samples) : null;
  const pluginsScene = ["plugins", "plugins-scanning", "plugins-folders", "plugin-flag"].some(hasScene);
  let rack = hasScene("rack") ? rackFixture(pluginsScene) : rackStateDto();
  const spectro = new Map<number, Sink>();
  let documentOpens = 0;
  let lastSpectrumSources: Array<"source" | "processed"> = ["processed"];
  let seq = 0;

  // H-43: the telemetry stream behaves like the engine's (`IdleTelemetryGate`): while the
  // transport is stopped it sends the current state once — and again after every transport
  // command — with silent meters and a still playhead (rate 0); while playing, or in the recording
  // scene, it streams at the 60 Hz telemetry rate.
  const recordingScene = hasScene("recording");
  let telemetrySink: Sink | null = null;
  let streamTimer: ReturnType<typeof setInterval> | null = null;
  let mockPlaying = false;
  let mockPlayhead = 23.4 * PREVIEW_RATE_HZ;
  let mockPlayedAtMs = 0;
  const mockPosition = (): number =>
    mockPlaying ? mockPlayhead + ((performance.now() - mockPlayedAtMs) / 1000) * PREVIEW_RATE_HZ : mockPlayhead;
  const telemetryFrame = (): ArrayBuffer => {
    seq += 1;
    const wobble = Math.sin(seq / 9) * 2;
    if (recordingScene) {
      return vxtm(seq, 2 | 4, RECORDED_SAMPLES + seq * 800, [-18 + wobble, -27 + wobble, -9 + wobble, -21 + wobble]);
    }
    if (mockPlaying) {
      return vxtm(seq, 1, mockPosition(), [-14 + wobble, -23 + wobble, -Infinity, -Infinity]);
    }
    return vxtm(seq, 0, mockPlayhead, [-Infinity, -Infinity, -Infinity, -Infinity], 0);
  };
  const pushTelemetry = (): void => telemetrySink?.onmessage(telemetryFrame());
  const syncTelemetry = (): void => {
    const stream = recordingScene || mockPlaying;
    if (stream && streamTimer === null) {
      streamTimer = setInterval(pushTelemetry, 1000 / 60);
    } else if (!stream && streamTimer !== null) {
      clearInterval(streamTimer);
      streamTimer = null;
    }
    pushTelemetry();
  };
  const moveTransport = (playing: boolean, playhead: number = mockPosition()): void => {
    mockPlayhead = playhead;
    mockPlaying = playing;
    mockPlayedAtMs = performance.now();
    syncTelemetry();
  };

  // H-37: the Loop toggle and the synced selection (the loop region while loop is on).
  let loopEnabled = false;
  let selectionRange: [number, number] | null = null;

  // H-31: `transport_get` and the six transport command cases below (`transport_play`,
  // `transport_pause`, `transport_stop`, `transport_play_from_start`, `transport_return_to_start`,
  // `transport_seek`) all answer with a `TransportStateDto` built from this one snapshot — none of
  // them fell through to `default`'s `null` before, which is exactly the shape of bug H-32 found
  // (a `null` transport reply corrupting `transport.svelte.ts`'s store).
  const transportSnapshot = (overrides: Partial<TransportStateDto> = {}): TransportStateDto => {
    const open = scenes.some((s) => s !== "rack") || dialog !== null;
    return transportStateDto({
      playhead_samples: open ? 23.4 * PREVIEW_RATE_HZ : 0,
      doc_len_samples: open ? doc.len_samples : 0,
      doc_rate_hz: open ? PREVIEW_RATE_HZ : 0,
      can_play: true,
      loop_enabled: loopEnabled,
      loop_range: loopEnabled ? selectionRange : null,
      ...overrides,
    });
  };

  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "app_info":
          return { name: "PowerVoice", version: "0.1.0" };
        case "settings_get":
          return settings;
        case "edit_bake_start":
          // T-708: `&scene=rack&dialog=bake` shows T-602's bake progress (or its noise-only confirm).
          return { job_id: 9 };
        case "settings_set":
          return (args as { settings: Settings }).settings;
        case "settings_startup_notice_take":
          // T-703: the preview never simulates a corrupt settings file.
          return null;
        case "settings_defaults":
          return settingsFixture();
        case "transport_get":
          return transportSnapshot();
        case "transport_play":
          moveTransport(true);
          return transportSnapshot({ playing: true, playhead_samples: Math.round(mockPlayhead) });
        case "transport_pause":
          moveTransport(false);
          return transportSnapshot({ playing: false, playhead_samples: Math.round(mockPlayhead) });
        case "transport_stop":
          moveTransport(false, 0);
          return transportSnapshot({ playing: false, playhead_samples: 0, play_start_samples: 0 });
        case "transport_play_from_start":
          moveTransport(true, 0);
          return transportSnapshot({ playing: true, playhead_samples: 0, play_start_samples: 0 });
        case "transport_return_to_start":
          moveTransport(false, 0);
          return transportSnapshot({ playing: false, playhead_samples: 0, play_start_samples: 0 });
        case "transport_seek": {
          const at = a.positionSamples as number;
          moveTransport(mockPlaying, at);
          return transportSnapshot({ playing: mockPlaying, playhead_samples: at, play_start_samples: at });
        }
        case "transport_set_loop":
          loopEnabled = a.enabled === true;
          return transportSnapshot();
        case "transport_set_selection":
          selectionRange = (a.selection as [number, number] | null) ?? null;
          return transportSnapshot();
        case "clock_now_ns":
          return 0;
        case "record_get":
          return recordFixture(options);
        case "telemetry_subscribe": {
          telemetrySink = a.channel as Sink;
          // Start once the stores have their initial state (the real engine only streams
          // telemetry after the transport exists).
          setTimeout(syncTelemetry, 600);
          return null;
        }
        case "recent_files_get":
          return [
            { path: PREVIEW_PATH, name: "chapter-03.wav", folder: "/home/narrator/audiobook", exists: true },
            { path: "/home/narrator/audiobook/chapter-02.wav", name: "chapter-02.wav", folder: "/home/narrator/audiobook", exists: true },
            { path: "/media/usb/old-take.wav", name: "old-take.wav", folder: "/media/usb", exists: false },
          ];
        case "document_open": {
          documentOpens += 1;
          if (dialog === "channel-choice" && documentOpens === 1) {
            throw needsConfirmation("dialog.channel_choice", { probe: JSON.stringify(PROBE) });
          }
          if (dialog === "confirm" && documentOpens === 2) {
            throw needsConfirmation("dialog.already_open", { name: "chapter-03.wav" });
          }
          return doc;
        }
        case "document_save":
          if (dialog === "clip") {
            throw needsConfirmation("dialog.overs", { count: "12", peak_dbfs: "1.8" });
          }
          return doc;
        case "peaks_get": {
          const r = a.request as { request_id: number; audio_rev: number; spp: number; start_sample: number; count: number };
          const count = Math.max(0, Math.min(r.count, Math.ceil((doc.len_samples - r.start_sample) / r.spp)));
          const raw = r.spp === 1;
          const values = raw
            ? pyramid
              ? longRawSamples(r.start_sample, count)
              : rawSamples(r.start_sample, count)
            : pyramid
              ? pyramid.peaks(r.spp, r.start_sample, count)
              : bucketPeaks(r.start_sample, r.spp, count);
          return vxpk(r.request_id, r.audio_rev, r.start_sample, r.spp, count, raw, values);
        }
        case "record_peaks_get": {
          const start = a.startBucket as number;
          const available = Math.floor((RECORDED_SAMPLES + seq * 2400) / LIVE_PEAKS_SPB);
          const count = Math.max(0, Math.min(a.count as number, available - start));
          return vxpk(0, 0, start * LIVE_PEAKS_SPB, LIVE_PEAKS_SPB, count, false, bucketPeaks(start * LIVE_PEAKS_SPB, LIVE_PEAKS_SPB, count));
        }
        // T-704: fire-and-forget subscriptions and view persistence the frame-time sweep hits
        // (the default case would log an error for each). `analyzer_subscribe` is with the H-42
        // analyzer cases below (it only streams in `&scene=analyzer…`).
        case "module_telemetry_subscribe":
        case "sidecar_view_set_waveform":
        case "sidecar_view_set_spectral":
          return null;
        case "spectro_attach":
          spectro.set(a.viewId as number, a.channel as Sink);
          return null;
        case "spectro_request": {
          const sink = spectro.get(a.viewId as number);
          const r = a.request as { request_id: number; audio_rev: number; fft_size: number; hop: number; tiles: number[] };
          if (sink) {
            r.tiles.forEach((tile, i) => {
              const last = i === r.tiles.length - 1;
              setTimeout(() => sink.onmessage(memoVxst(r.request_id, r.audio_rev, r.fft_size, r.hop, tile, last, pyramid !== null)), 0);
            });
          }
          return null;
        }
        case "markers_get":
          return hasScene("recording") ? [] : MARKERS;
        case "rack_list_modules":
          return MODULES;
        case "rack_get":
          return rack;
        // T-901: the preview has no sandbox; the slot's window state just toggles.
        case "rack_editor_open":
        case "rack_editor_close":
        case "rack_editor_close_all": {
          const open = cmd === "rack_editor_open";
          rack = {
            ...rack,
            slots: rack.slots.map((s, i) =>
              cmd === "rack_editor_close_all" || i === a.slot ? { ...s, editor_open: open && s.has_editor } : s,
            ),
          };
          return rack;
        }
        case "rack_response_curve":
          return responseCurve(a.points as number[]);
        case "module_presets_list":
        case "rack_presets_list":
          return [{ key: "voice_warmth", name: text("Voice warmth"), is_factory: true }, { key: "My booth", name: text("My booth"), is_factory: false }];
        case "module_preset_load":
        case "rack_preset_load":
        case "module_reset_default":
          return rack;
        case "module_preset_save":
        case "module_preset_rename":
        case "rack_preset_save":
        case "rack_preset_rename":
        case "rack_preset_import":
          return { key: "My booth", name: text("My booth"), is_factory: false };
        case "module_preset_import":
          return { module_id: "org.powervoice.gain", entry: { key: "My booth", name: text("My booth"), is_factory: false } };
        case "module_preset_delete":
        case "rack_preset_delete":
        case "module_preset_export":
        case "rack_preset_export":
          return null;
        case "loudness_analyze_start":
          return { job_id: 7 };
        // H-42: the analyzer streams (synthetic voice) and the long-term average job.
        case "analyzer_subscribe": {
          if (analyzerScene) {
            const sink = a.channel as Sink;
            let n = 0;
            setTimeout(() => setInterval(() => sink.onmessage(vxsaFrame(++n, PREVIEW_RATE_HZ, voiceBands(PREVIEW_RATE_HZ, n))), 100), 400);
          }
          return 1;
        }
        case "analyzer_set_response":
        case "analyzer_unsubscribe":
        case "analyzer_inspector_configure":
        case "spectrum_analyze_cancel":
          if (cmd === "analyzer_inspector_configure") {
            const c = a.config as { fft_size: number; response: string };
            inspectorConfig.fft = c.fft_size;
            inspectorConfig.response = ["fast", "medium", "slow"].indexOf(c.response);
          }
          return null;
        case "analyzer_voice_subscribe": {
          if (analyzerScene) {
            const sink = a.channel as unknown as { onmessage: (m: unknown) => void };
            setTimeout(() => sink.onmessage(PREVIEW_VOICE_REPORT), 500);
          }
          return 2;
        }
        case "analyzer_inspector_subscribe": {
          if (analyzerScene) {
            const sink = a.channel as Sink;
            let n = 0;
            const send = () => {
              const levels = voiceBinsCached(inspectorConfig.fft, PREVIEW_RATE_HZ);
              sink.onmessage(vxisFrame(++n, PREVIEW_RATE_HZ, inspectorConfig.fft, inspectorConfig.window, inspectorConfig.response, levels));
            };
            setTimeout(send, 300);
            setTimeout(() => setInterval(send, 250), 400);
          }
          return 3;
        }
        case "spectrum_analyze_start": {
          const request = a.request as { sources: Array<"source" | "processed">; fft_size: number; window: string };
          const jobId = 11;
          const fft = request.fft_size;
          const progress = (fraction: number, state: string) =>
            void emit("job_progress", { job_id: jobId, kind: "spectrum_analyze", state, fraction });
          setTimeout(() => progress(0.35, "running"), 150);
          setTimeout(() => {
            progress(1, "done");
            void emit("spectrum_report", {
              job_id: jobId,
              sample_rate_hz: PREVIEW_RATE_HZ,
              fft_size: fft,
              window: request.window,
              start_sample: 0,
              end_sample: doc.len_samples,
              results: request.sources.map((source) => ({
                source,
                frames: 560,
                has_noise: true,
                report: PREVIEW_AVERAGE_REPORTS[source],
              })),
            });
          }, 400);
          lastSpectrumSources = request.sources;
          return { job_id: jobId };
        }
        case "spectrum_analyze_curve": {
          const index = a.index as number;
          const source = lastSpectrumSources[index] ?? "processed";
          const fft = 16_384;
          return vxltFrame(11, index, PREVIEW_RATE_HZ, fft, voiceBinsCached(fft, PREVIEW_RATE_HZ, source), roomToneBins(fft, PREVIEW_RATE_HZ));
        }
        case "spectrum_export_csv":
          return a.path as string;
        case "acx_check":
          return ACX;
        case "export_formats":
          return { mp3_available: true };
        case "record_offset_get":
          return OFFSET;
        case "storage_info":
          return STORAGE;
        case "recovery_list":
          return dialog === "recovery" ? [SESSION] : [];
        case "devices_list":
          return DEVICES;
        case "plugins_list":
          return PLUGINS;
        case "plugins_folders":
          return PLUGIN_FOLDERS;
        case "plugins_install":
          return installResult(dialog, (a.replace as boolean) ?? false);
        case "plugins_rescan":
          return PLUGINS.length;
        default: {
          // H-31: see the file doc comment — a missing case must never quietly answer `null`.
          const message = `previewIpc: unhandled command "${cmd}"`;
          console.error(`[previewIpc] ${message}`);
          if (import.meta.env.MODE === "test") {
            throw new Error(message);
          }
          return null;
        }
      }
    },
    { shouldMockEvents: true },
  );

  // H-42: `&scene=analyzer-average|analyzer-compare` switch the analyzer's mode (and run the
  // average job / freeze A and B); `&dialog=inspector` opens the Spectrum Inspector.
  if (analyzerScene || dialog === "inspector") {
    void import("../lib/analyzer/diagnostics.svelte").then(async (d) => {
      await new Promise((resolve) => setTimeout(resolve, 900));
      if (dialog === "inspector") {
        d.setInspectorOpen(true);
      }
      if (hasScene("analyzer-average")) {
        d.setAnalyzerMode("average");
        await d.startAverage();
      }
      if (hasScene("analyzer-compare")) {
        d.setAnalyzerMode("compare");
        await d.startSourceVsProcessed();
      }
    });
  }
}
