import { sidecarViewSetSpectral } from "../ipc/commands";
import { registerAction } from "../shortcuts";
import type { ColormapName } from "../spectrogram/colormap";
import type { FreqScale } from "../spectrum/freqAxis";
import { saveSettings } from "./settings.svelte";

/**
 * Spectral pane store (T-207, SPEC-007 §2.1/§2.5/§2.6): visibility, the waveform/spectral split
 * ratio, and the display settings that are shader-only (no refetch — floor/ceiling/colormap/
 * frequency scale, AC-8) plus the FFT size (which does trigger a new `spectro_request`, since it
 * changes the tile grid). Registers the Shift+D toggle (SPEC-007 §2.1).
 *
 * **T-306 (SPEC-018 §2.6.5):** per-document persistence. Every mutator debounces a
 * `sidecar_view_set_spectral` call (the Rust side merges it into the open document's sidecar
 * `view` for the next Save — never marks the document modified, SPEC-018 §2.4); restoring on
 * open is `document.svelte.ts`'s job (it owns `document_changed`'s `spectral_view`).
 *
 * **H-12 (A-014):** the *display* settings — colormap, frequency scale, floor/ceiling, FFT size,
 * not visibility/split ratio — also debounce a `settings_set` write to `Settings.spectral_defaults`
 * (`saveSettings`, `settings.svelte.ts`), so they become the app-wide default a document with no
 * sidecar view starts from. `App.svelte` applies the loaded settings once via
 * {@link applySpectralDefaults}; a document's own sidecar `spectral_view`, if any, is applied
 * afterwards by `document.svelte.ts` and wins.
 */

/** Debounce for pushing a settings change to the backend (matches the digest-baseline debounce
 * SPEC-018 §4.3 uses on the Rust side; not itself normative, just a sane UI-side default). */
const PERSIST_DEBOUNCE_MS = 250;
let persistTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * `persistDefaults`: also write the display settings to `Settings.spectral_defaults` (A-014) —
 * `true` for the display-setting mutators (colormap/scale/floor/ceiling/FFT size), `false` for
 * visibility/split-ratio (per-document only, SPEC-018 §2.6.5).
 */
function schedulePersist(persistDefaults: boolean): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
  }
  persistTimer = setTimeout(() => {
    persistTimer = null;
    void sidecarViewSetSpectral({
      visible: state.visible,
      split_ratio: state.splitRatio,
      fft_size: state.fftSize,
      freq_scale: state.freqScale,
      display_floor_db: state.floorDb,
      display_ceil_db: state.ceilDb,
      colormap: state.colormap,
    }).catch(() => {
      // Fire-and-forget: a view-only change never blocks the UI or shows a notice on failure
      // (there is no document open, or the IPC call itself failed) — the next Save just won't
      // carry this particular tweak.
    });
    if (persistDefaults) {
      // `saveSettings` already reports/swallows its own errors (settings.svelte.ts) — nothing
      // further to catch here.
      void saveSettings({
        spectral_defaults: {
          freq_scale: state.freqScale,
          colormap: state.colormap,
          display_floor_db: state.floorDb,
          display_ceil_db: state.ceilDb,
          fft_size: state.fftSize,
        },
      });
    }
  }, PERSIST_DEBOUNCE_MS);
}

export const FLOOR_RANGE_DB: readonly [number, number] = [-150, -30];
export const CEIL_RANGE_DB: readonly [number, number] = [-60, 6];
export const MIN_SPAN_DB = 20;

export interface SpectralSnapshot {
  visible: boolean;
  /** The waveform pane's share of the split, 0-100 (SPEC-007 §2.1). */
  splitRatio: number;
  freqScale: FreqScale;
  colormap: ColormapName;
  floorDb: number;
  ceilDb: number;
  /** `null` = Auto (SPEC-007 §2.6). */
  fftSize: number | null;
}

const DEFAULTS: SpectralSnapshot = {
  visible: false,
  splitRatio: 50,
  freqScale: "log",
  colormap: "inferno",
  floorDb: -120,
  ceilDb: 0,
  fftSize: null,
};

let state = $state<SpectralSnapshot>({ ...DEFAULTS });

/** Clamps `(floorDb, ceilDb)` into their ranges, preserving `ceilDb − floorDb ≥ MIN_SPAN_DB`
 * (SPEC-007 §3) by adjusting the ceiling first, then the floor if the ceiling was itself clamped. */
function clampFloorCeil(floorDb: number, ceilDb: number): { floorDb: number; ceilDb: number } {
  let floor = Math.min(Math.max(floorDb, FLOOR_RANGE_DB[0]), FLOOR_RANGE_DB[1]);
  let ceil = Math.min(Math.max(ceilDb, CEIL_RANGE_DB[0]), CEIL_RANGE_DB[1]);
  if (ceil - floor < MIN_SPAN_DB) {
    ceil = Math.min(CEIL_RANGE_DB[1], floor + MIN_SPAN_DB);
    floor = Math.max(FLOOR_RANGE_DB[0], ceil - MIN_SPAN_DB);
  }
  return { floorDb: floor, ceilDb: ceil };
}

export interface SpectralStateApi extends SpectralSnapshot {
  toggle(): void;
  setVisible(visible: boolean): void;
  setSplitRatio(pct: number): void;
  setFreqScale(scale: FreqScale): void;
  setColormap(name: ColormapName): void;
  setFloorDb(db: number): void;
  setCeilDb(db: number): void;
  setFftSize(size: number | null): void;
}

/** Read-only-shaped accessor with mutators (components read fields directly, e.g.
 * `spectralState().visible`, and call the setters — mirrors `settingsState()`'s pattern). */
export function spectralState(): SpectralStateApi {
  return {
    get visible() {
      return state.visible;
    },
    get splitRatio() {
      return state.splitRatio;
    },
    get freqScale() {
      return state.freqScale;
    },
    get colormap() {
      return state.colormap;
    },
    get floorDb() {
      return state.floorDb;
    },
    get ceilDb() {
      return state.ceilDb;
    },
    get fftSize() {
      return state.fftSize;
    },
    toggle() {
      state = { ...state, visible: !state.visible };
      schedulePersist(false);
    },
    setVisible(visible: boolean) {
      state = { ...state, visible };
      schedulePersist(false);
    },
    setSplitRatio(pct: number) {
      state = { ...state, splitRatio: Math.min(100, Math.max(0, pct)) };
      schedulePersist(false);
    },
    setFreqScale(scale: FreqScale) {
      state = { ...state, freqScale: scale };
      schedulePersist(true);
    },
    setColormap(name: ColormapName) {
      state = { ...state, colormap: name };
      schedulePersist(true);
    },
    setFloorDb(db: number) {
      state = { ...state, ...clampFloorCeil(db, state.ceilDb) };
      schedulePersist(true);
    },
    setCeilDb(db: number) {
      state = { ...state, ...clampFloorCeil(state.floorDb, db) };
      schedulePersist(true);
    },
    setFftSize(size: number | null) {
      state = { ...state, fftSize: size };
      schedulePersist(true);
    },
  };
}

/** Wires the Shift+D toggle (SPEC-007 §2.1). Returns the teardown. */
export function initSpectral(): () => void {
  return registerAction("spectral.toggle", () => spectralState().toggle());
}

/**
 * T-306: applies a sidecar's restored spectral settings (`document.svelte.ts`, right after a
 * successful open) — sets the state directly, without re-scheduling a persist (there is nothing
 * new to write back; it's exactly what was just read). Invalid enum values are ignored rather
 * than defaulted, so an unrecognized future value doesn't clobber the current setting.
 */
export function applyRestoredSpectralView(view: {
  visible: boolean;
  split_ratio: number;
  fft_size: number | null;
  freq_scale: string;
  display_floor_db: number;
  display_ceil_db: number;
  colormap: string;
}): void {
  const freqScale: FreqScale | null = view.freq_scale === "log" || view.freq_scale === "linear" ? view.freq_scale : null;
  const colormap: ColormapName | null =
    view.colormap === "inferno" || view.colormap === "viridis" || view.colormap === "gray"
      ? view.colormap
      : null;
  const { floorDb, ceilDb } = clampFloorCeil(view.display_floor_db, view.display_ceil_db);
  state = {
    visible: view.visible,
    splitRatio: Math.min(100, Math.max(0, view.split_ratio)),
    freqScale: freqScale ?? state.freqScale,
    colormap: colormap ?? state.colormap,
    floorDb,
    ceilDb,
    fftSize: view.fft_size,
  };
}

/**
 * H-12 (A-014): applies `Settings.spectral_defaults`, loaded once at startup (`App.svelte`, after
 * `loadSettings`) — sets the *display* settings only (colormap/scale/floor/ceiling/FFT size), not
 * visibility/split ratio (SPEC-018 §2.6.5 keeps those per document only, with no app-wide
 * default). Never re-schedules a persist, same reasoning as {@link applyRestoredSpectralView}.
 * Invalid enum values are ignored, same tolerance as the restored-view path. A document's own
 * sidecar `spectral_view`, applied afterwards by `document.svelte.ts`, overrides this.
 */
export function applySpectralDefaults(defaults: {
  freq_scale: string;
  colormap: string;
  display_floor_db: number;
  display_ceil_db: number;
  fft_size: number | null;
}): void {
  const freqScale: FreqScale | null =
    defaults.freq_scale === "log" || defaults.freq_scale === "linear" ? defaults.freq_scale : null;
  const colormap: ColormapName | null =
    defaults.colormap === "inferno" || defaults.colormap === "viridis" || defaults.colormap === "gray"
      ? defaults.colormap
      : null;
  const { floorDb, ceilDb } = clampFloorCeil(defaults.display_floor_db, defaults.display_ceil_db);
  state = {
    ...state,
    freqScale: freqScale ?? state.freqScale,
    colormap: colormap ?? state.colormap,
    floorDb,
    ceilDb,
    fftSize: defaults.fft_size,
  };
}

/** Test/teardown helper. */
export function resetSpectralForTest(): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
    persistTimer = null;
  }
  state = { ...DEFAULTS };
}
