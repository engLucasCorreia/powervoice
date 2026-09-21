import { cssColorToRgba, type Rgba } from "../render/quads";
import { parseThemes, resolveValue, type ThemeTokens } from "./contrast";
import { RESOLVED_THEMES, themeState, type ResolvedTheme } from "./theme.svelte";

/**
 * T-708: the colours and line widths the canvas/WebGL renderers draw with, read from the theme's
 * CSS custom properties — typed, cached, and invalidated when the resolved theme changes, so a
 * theme switch repaints every renderer on its next frame (no reload, no hard-coded fallbacks).
 *
 * Source: `getComputedStyle(<html>)` (the real cascade). A token that comes back empty (jsdom in
 * tests; a stylesheet not loaded yet) falls back to the same token parsed from
 * `design-tokens.css` for the resolved theme — the token file stays the only place a colour is
 * written down.
 *
 * Every renderer (waveform, spectral, analyzer, EQ graph — H-32) draws on its own perpetual
 * animation-frame loop rather than a reactive `$effect`, so a theme switch is simply picked up on
 * the next frame with no explicit subscription needed.
 */

/** One colour in both shapes the renderers need. */
export interface ThemeColor {
  /** For Canvas2D `fillStyle`/`strokeStyle`. */
  readonly css: string;
  /** For WebGL vertex colours, `0..1`. */
  readonly rgba: Rgba;
}

export interface ThemeColors {
  readonly theme: ResolvedTheme;
  /** Content lines: playhead, record head, markers, the sample polyline (CSS px). */
  readonly strokePx: number;
  /** The EQ graph's total-response curve (CSS px). */
  readonly emphasisStrokePx: number;
  readonly wave: {
    readonly bg: ThemeColor;
    readonly fill: ThemeColor;
    /** H-79: the wave's colour where it passes through the current selection — a distinct shade
     * from `fill`, drawn on top so the wave stays fully legible there (SPEC-006 §2.12 Amendment 2). */
    readonly fillSelected: ThemeColor;
    readonly pending: ThemeColor;
    readonly playhead: ThemeColor;
    readonly selectionFill: ThemeColor;
    /** H-79: the selection's boundary lines (`--wave-selection-handle`). */
    readonly selectionBorder: ThemeColor;
    readonly marker: ThemeColor;
    readonly markerRegion: ThemeColor;
    readonly record: ThemeColor;
    readonly recordHead: ThemeColor;
    readonly punchRegion: ThemeColor;
    /** H-37: the loop region's brace (top strip, ruler bar) and boundary lines. */
    readonly loop: ThemeColor;
  };
  readonly spec: {
    readonly bg: ThemeColor;
    readonly pending: ThemeColor;
  };
  readonly analyzer: {
    readonly bg: ThemeColor;
    readonly fill: ThemeColor;
    readonly peak: ThemeColor;
    readonly grid: ThemeColor;
    /** H-42: frozen A/B snapshots, the room-tone curve, the hover crosshair, peak markers. */
    readonly compareA: ThemeColor;
    readonly compareB: ThemeColor;
    readonly noise: ThemeColor;
    readonly crosshair: ThemeColor;
    readonly marker: ThemeColor;
    /** H-92: the "Explain My Voice" graph's dominant smoothed envelope and its voice-region
     * tint. The raw curve, F0 line and harmonic ticks reuse `noise`/`marker`/`compareB` above. */
    readonly explainEnvelope: ThemeColor;
    readonly explainBand: ThemeColor;
  };
  readonly eq: {
    readonly grid: ThemeColor;
    readonly curve: ThemeColor;
    readonly fill: ThemeColor;
    readonly labelPatch: ThemeColor;
    readonly labelText: ThemeColor;
    readonly selectedRing: ThemeColor;
    /** Node colour by band key (`hp`, `ls`, `1`…`5`, `hs`, `lp`); other keys get the curve colour. */
    readonly bands: Readonly<Record<string, ThemeColor>>;
  };
  /** H-63: the dynamics transfer graph. It shares the EQ graph's curve/grid tokens so the two
   * module graphs read as one family; only the 1:1 diagonal and the threshold handles are its
   * own roles. */
  readonly transfer: {
    readonly grid: ThemeColor;
    readonly unity: ThemeColor;
    readonly curve: ThemeColor;
    readonly fill: ThemeColor;
    readonly handle: ThemeColor;
    /** H-77: one colour per section (component), for the threshold handles and the per-section
     * curve overlays; it wraps for a module with more sections than colours. */
    readonly components: readonly ThemeColor[];
    readonly labelPatch: ThemeColor;
    readonly labelText: ThemeColor;
  };
}

const EQ_BANDS = ["hp", "ls", "1", "2", "3", "4", "5", "hs", "lp"] as const;

// The token file's text, for the fallback (Vite `?raw`; vitest.config lets this CSS through).
const files = import.meta.glob("./design-tokens.css", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

let parsed: Record<string, ThemeTokens> | null = null;

function tokenFile(theme: ResolvedTheme): ThemeTokens {
  parsed ??= parseThemes(files["./design-tokens.css"] ?? "");
  return parsed[theme] ?? parsed.dark ?? {};
}

function currentTheme(): ResolvedTheme {
  const stamped = document.documentElement.dataset.theme;
  return RESOLVED_THEMES.find((name) => name === stamped) ?? "dark";
}

/** Builds a fresh snapshot for `theme` (exported for tests; renderers use {@link themeColors}). */
export function readThemeColors(theme: ResolvedTheme): ThemeColors {
  const style = getComputedStyle(document.documentElement);
  const fallback = tokenFile(theme);
  const raw = (name: string): string =>
    style.getPropertyValue(name).trim() || resolveValue(fallback, name) || "";
  const color = (name: string): ThemeColor => {
    const css = raw(name);
    return { css, rgba: cssColorToRgba(css) };
  };
  const px = (name: string): number => {
    const value = Number.parseFloat(raw(name));
    return Number.isFinite(value) && value > 0 ? value : 1;
  };
  const curve = color("--eq-curve");
  const bands: Record<string, ThemeColor> = {};
  for (const band of EQ_BANDS) {
    bands[band] = color(`--eq-band-${band}`);
  }
  return {
    theme,
    strokePx: px("--pv-stroke-content"),
    emphasisStrokePx: px("--pv-stroke-emphasis"),
    wave: {
      bg: color("--wave-bg"),
      fill: color("--wave-fill"),
      fillSelected: color("--wave-fill-selected"),
      pending: color("--wave-pending"),
      playhead: color("--wave-playhead"),
      selectionFill: color("--wave-selection-fill"),
      selectionBorder: color("--wave-selection-handle"),
      marker: color("--wave-marker"),
      markerRegion: color("--wave-marker-region"),
      record: color("--wave-record"),
      recordHead: color("--wave-record-head"),
      punchRegion: color("--wave-punch-region"),
      loop: color("--wave-loop"),
    },
    spec: {
      bg: color("--spec-bg"),
      pending: color("--spec-pending"),
    },
    analyzer: {
      bg: color("--analyzer-bg"),
      fill: color("--analyzer-fill"),
      peak: color("--analyzer-peak"),
      grid: color("--analyzer-grid"),
      compareA: color("--analyzer-compare-a"),
      compareB: color("--analyzer-compare-b"),
      noise: color("--analyzer-noise"),
      crosshair: color("--analyzer-crosshair"),
      marker: color("--analyzer-marker"),
      explainEnvelope: color("--analyzer-explain-envelope"),
      explainBand: color("--analyzer-explain-band"),
    },
    eq: {
      grid: color("--eq-grid"),
      curve,
      fill: color("--eq-fill"),
      labelPatch: color("--pv-bg-inset"),
      labelText: color("--pv-text-tertiary"),
      selectedRing: color("--pv-focus-ring"),
      bands,
    },
    transfer: {
      grid: color("--eq-grid"),
      unity: color("--eq-grid"),
      curve,
      fill: color("--eq-fill"),
      handle: color("--pv-accent"),
      components: [
        color("--eq-band-1"),
        color("--eq-band-2"),
        color("--eq-band-3"),
        color("--eq-band-4"),
      ],
      labelPatch: color("--pv-bg-inset"),
      labelText: color("--pv-text-tertiary"),
    },
  };
}

let cache: { key: string; colors: ThemeColors } | null = null;

/**
 * The current theme's renderer colours. Cached: the same object comes back until the resolved
 * theme changes (a reactive read of `themeState().revision`, plus the stamped `data-theme` so a
 * stamp from outside `applyThemePref` also invalidates).
 */
export function themeColors(): ThemeColors {
  const revision = themeState().revision;
  const theme = currentTheme();
  const key = `${revision}:${theme}`;
  if (cache?.key !== key) {
    cache = { key, colors: readThemeColors(theme) };
  }
  return cache.colors;
}

/** EQ node colour for a band key (falls back to the curve colour for unknown keys). */
export function eqBandColor(colors: ThemeColors, key: string): ThemeColor {
  return colors.eq.bands[key] ?? colors.eq.curve;
}

/** Offset that puts a `widthPx` line centred on a pixel boundary crisply (odd widths need ½ px). */
export function crispOffset(widthPx: number): number {
  return Math.round(widthPx) % 2 === 1 ? 0.5 : 0;
}

export function resetThemeColorsForTest(): void {
  cache = null;
}
