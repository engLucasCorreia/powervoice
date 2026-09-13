import { registerAction } from "../keymap";
import type { ColormapName } from "../spectrogram/colormap";
import type { FreqScale } from "../spectrum/freqAxis";

/**
 * Spectral pane store (T-207, SPEC-007 §2.1/§2.5/§2.6): visibility, the waveform/spectral split
 * ratio, and the display settings that are shader-only (no refetch — floor/ceiling/colormap/
 * frequency scale, AC-8) plus the FFT size (which does trigger a new `spectro_request`, since it
 * changes the tile grid). Registers the Shift+D toggle (SPEC-007 §2.1).
 *
 * **Deferred (lean first version):** SPEC-007 §2.1/§2.12/AC-12 says visibility, ratio and display
 * settings "survive a restart" — this ticket keeps them in memory only (like the waveform's own
 * zoom, which also isn't persisted yet). Persisting them needs new fields on the Rust `Settings`
 * DTO (`src-tauri/src/settings.rs`) plus `just gen-types`; out of this UI-only ticket's scope —
 * left as a follow-up for a hardening ticket.
 */

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
    },
    setVisible(visible: boolean) {
      state = { ...state, visible };
    },
    setSplitRatio(pct: number) {
      state = { ...state, splitRatio: Math.min(100, Math.max(0, pct)) };
    },
    setFreqScale(scale: FreqScale) {
      state = { ...state, freqScale: scale };
    },
    setColormap(name: ColormapName) {
      state = { ...state, colormap: name };
    },
    setFloorDb(db: number) {
      state = { ...state, ...clampFloorCeil(db, state.ceilDb) };
    },
    setCeilDb(db: number) {
      state = { ...state, ...clampFloorCeil(state.floorDb, db) };
    },
    setFftSize(size: number | null) {
      state = { ...state, fftSize: size };
    },
  };
}

/** Wires the Shift+D toggle (SPEC-007 §2.1). Returns the teardown. */
export function initSpectral(): () => void {
  return registerAction("spectral.toggle", () => spectralState().toggle());
}

/** Test/teardown helper. */
export function resetSpectralForTest(): void {
  state = { ...DEFAULTS };
}
