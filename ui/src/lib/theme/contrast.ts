/**
 * WCAG 2.2 contrast checking for the design tokens (H-25, T-708). Pure and DOM-free: the test
 * parses `design-tokens.css` as text (jsdom doesn't compute custom properties from stylesheets
 * reliably), resolves `var(--x)` references inside one theme, and checks every pair below in
 * every theme block. `themeColors.ts` reuses the parser as its fallback source.
 *
 * Text pairs need 4.5:1 (AA, normal text — every PowerVoice UI string is < 18.66 px bold/24 px);
 * non-text indicators (focus ring, selected border, slider fill, record lamp, meter boundaries,
 * waveform/playhead/marker lines) need 3:1 (WCAG 1.4.11). High Contrast raises both: 7:1 for text
 * (AAA) and 4.5:1 for indicators (`minRatio`). Disabled text is exempt and deliberately absent.
 */

export type Rgb = [number, number, number];
export type ThemeTokens = Record<string, string>;

export interface ContrastPair {
  fg: string;
  bg: string;
  min: number;
  use: string;
  /** H-79: when `fg` (or `bg`) resolves to a translucent colour (an `rgba()` wash, e.g. a
   * selection fill), composite it over this token's opaque colour first — the ratio then
   * reflects what a viewer actually sees, not the wash's colour in isolation. Ignored for an
   * opaque `fg`/`bg`. */
  fgOver?: string;
  bgOver?: string;
}

const AA_TEXT = 4.5;
const AA_NON_TEXT = 3;
const AAA_TEXT = 7;
const HC_NON_TEXT = 4.5;

/** The ratio `pair` must reach in `theme`: AA everywhere; AAA text (and 4.5:1 indicators) in
 * High Contrast. */
export function minRatio(pair: ContrastPair, theme: string): number {
  if (theme === "high-contrast") {
    return pair.min >= AA_TEXT ? AAA_TEXT : HC_NON_TEXT;
  }
  return pair.min;
}

const TEXT_ROLES = ["--pv-text-primary", "--pv-text-secondary", "--pv-text-tertiary"] as const;
const SURFACES = [
  "--pv-bg-inset",
  "--pv-bg-app",
  "--pv-bg-panel",
  "--pv-bg-raised",
  "--pv-bg-overlay",
] as const;

function textOnSurfaces(): ContrastPair[] {
  const pairs: ContrastPair[] = [];
  for (const fg of TEXT_ROLES) {
    for (const bg of SURFACES) {
      pairs.push({ fg, bg, min: AA_TEXT, use: "body/label text on every surface" });
    }
  }
  return pairs;
}

export const CONTRAST_PAIRS: readonly ContrastPair[] = [
  ...textOnSurfaces(),
  // Text on control fills (buttons, segments, selected pills).
  { fg: "--pv-text-primary", bg: "--pv-control-bg", min: AA_TEXT, use: "button label" },
  { fg: "--pv-text-primary", bg: "--pv-control-bg-hover", min: AA_TEXT, use: "hovered button" },
  { fg: "--pv-text-primary", bg: "--pv-control-bg-active", min: AA_TEXT, use: "pressed button" },
  { fg: "--pv-text-primary", bg: "--pv-control-bg-selected", min: AA_TEXT, use: "selected segment" },
  { fg: "--pv-text-secondary", bg: "--pv-control-bg", min: AA_TEXT, use: "unselected segment" },
  { fg: "--pv-text-secondary", bg: "--pv-control-track", min: AA_TEXT, use: "segment on track" },
  { fg: "--pv-text-primary", bg: "--pv-field-bg", min: AA_TEXT, use: "typed value" },
  { fg: "--pv-text-tertiary", bg: "--pv-field-bg", min: AA_TEXT, use: "unit suffix in a field" },
  // Accent.
  { fg: "--pv-text-on-accent", bg: "--pv-accent-fill", min: AA_TEXT, use: "primary button" },
  { fg: "--pv-text-on-accent", bg: "--pv-accent-fill-hover", min: AA_TEXT, use: "primary hover" },
  { fg: "--pv-text-on-accent", bg: "--pv-accent-fill-active", min: AA_TEXT, use: "primary pressed" },
  { fg: "--pv-accent-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "accent text on panel" },
  { fg: "--pv-accent-text", bg: "--pv-bg-overlay", min: AA_TEXT, use: "accent text in menus" },
  { fg: "--pv-accent-text", bg: "--pv-accent-soft", min: AA_TEXT, use: "pressed toggle label" },
  // Semantic text and fills.
  { fg: "--pv-text-on-record", bg: "--pv-record-fill", min: AA_TEXT, use: "Record while recording" },
  { fg: "--pv-text-on-record", bg: "--pv-record-fill-hover", min: AA_TEXT, use: "Record hover" },
  { fg: "--pv-record-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "recording time" },
  { fg: "--pv-record-text", bg: "--pv-record-soft", min: AA_TEXT, use: "record badge" },
  { fg: "--pv-text-on-danger", bg: "--pv-danger-fill", min: AA_TEXT, use: "destructive button" },
  { fg: "--pv-text-on-danger", bg: "--pv-danger-fill-hover", min: AA_TEXT, use: "destructive hover" },
  { fg: "--pv-danger-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "error text" },
  { fg: "--pv-danger-text", bg: "--pv-bg-overlay", min: AA_TEXT, use: "error text in dialogs" },
  { fg: "--pv-danger-text", bg: "--pv-danger-soft", min: AA_TEXT, use: "danger badge" },
  { fg: "--pv-text-on-warning", bg: "--pv-warning-fill", min: AA_TEXT, use: "warning pill" },
  { fg: "--pv-warning-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "warning text" },
  { fg: "--pv-warning-text", bg: "--pv-warning-soft", min: AA_TEXT, use: "warning badge" },
  { fg: "--pv-text-on-success", bg: "--pv-success-fill", min: AA_TEXT, use: "success pill" },
  { fg: "--pv-success-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "pass text (ACX)" },
  { fg: "--pv-success-text", bg: "--pv-success-soft", min: AA_TEXT, use: "success badge" },
  // Non-text indicators (3:1).
  { fg: "--pv-focus-ring", bg: "--pv-bg-app", min: AA_NON_TEXT, use: "focus ring" },
  { fg: "--pv-focus-ring", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "focus ring" },
  { fg: "--pv-focus-ring", bg: "--pv-bg-overlay", min: AA_NON_TEXT, use: "focus ring in dialogs" },
  { fg: "--pv-focus-ring", bg: "--pv-bg-inset", min: AA_NON_TEXT, use: "focus ring on wells" },
  { fg: "--pv-accent", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "selected/slider indicator" },
  { fg: "--pv-accent", bg: "--pv-control-track", min: AA_NON_TEXT, use: "switch on / slider fill" },
  { fg: "--pv-border-control", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "text field boundary" },
  { fg: "--pv-border-control", bg: "--pv-bg-overlay", min: AA_NON_TEXT, use: "field in dialogs" },
  { fg: "--pv-record", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "record lamp / tally" },
  { fg: "--pv-warning", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "warning dot" },
  { fg: "--pv-success", bg: "--pv-bg-panel", min: AA_NON_TEXT, use: "ok dot" },
  { fg: "--pv-text-secondary", bg: "--pv-control-track", min: AA_NON_TEXT, use: "switch-off knob" },
  // Audio content (T-708): every line or bar that carries information holds 3:1 on its well.
  { fg: "--wave-fill", bg: "--wave-bg", min: AA_NON_TEXT, use: "waveform" },
  { fg: "--wave-playhead", bg: "--wave-bg", min: AA_NON_TEXT, use: "playhead" },
  { fg: "--wave-marker", bg: "--wave-bg", min: AA_NON_TEXT, use: "marker line/flag" },
  { fg: "--wave-record", bg: "--wave-bg", min: AA_NON_TEXT, use: "take being recorded" },
  { fg: "--wave-record-head", bg: "--wave-bg", min: AA_NON_TEXT, use: "record head" },
  { fg: "--wave-selection-handle", bg: "--wave-bg", min: AA_NON_TEXT, use: "selection edge" },
  // H-79 (SPEC-006 §2.12 Amendment 2): the selection must read as a different colour from the
  // wave, and the wave must stay visible against the selection's own wash. Both washes are
  // translucent, so they're composited over `--wave-bg` (what the well shows behind them) before
  // comparing — see `fgOver`/`bgOver` and `compositeOver` below.
  {
    fg: "--wave-selection-fill",
    fgOver: "--wave-bg",
    bg: "--wave-fill",
    min: AA_NON_TEXT,
    use: "selection wash vs. the waveform's own colour",
  },
  {
    fg: "--wave-fill-selected",
    bg: "--wave-selection-fill",
    bgOver: "--wave-bg",
    min: AA_NON_TEXT,
    use: "the wave drawn inside a selection, against the selection wash",
  },
  { fg: "--wave-ruler-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "amplitude/time ruler labels" },
  { fg: "--wave-playhead", bg: "--spec-bg", min: AA_NON_TEXT, use: "playhead over the spectrogram" },
  { fg: "--wave-marker", bg: "--spec-bg", min: AA_NON_TEXT, use: "marker over the spectrogram" },
  { fg: "--spec-ruler-text", bg: "--pv-bg-panel", min: AA_TEXT, use: "frequency ruler labels" },
  { fg: "--spec-scrim-text", bg: "--spec-bg", min: AA_TEXT, use: "frozen notice over the spectrogram" },
  { fg: "--analyzer-peak", bg: "--analyzer-bg", min: AA_NON_TEXT, use: "analyzer peak hold" },
  { fg: "--pv-meter-safe", bg: "--pv-meter-track", min: AA_NON_TEXT, use: "meter level bar" },
  { fg: "--pv-meter-caution", bg: "--pv-meter-track", min: AA_NON_TEXT, use: "meter peak hold / GR" },
  { fg: "--pv-meter-over", bg: "--pv-meter-track", min: AA_NON_TEXT, use: "meter clip" },
  { fg: "--eq-curve", bg: "--pv-bg-inset", min: AA_NON_TEXT, use: "EQ total response" },
  ...["hp", "ls", "1", "2", "3", "4", "5", "hs", "lp"].map((band) => ({
    fg: `--eq-band-${band}`,
    bg: "--pv-bg-inset",
    min: AA_NON_TEXT,
    use: "EQ band node",
  })),
];

/** `#rgb` / `#rrggbb` → `[r, g, b]` (0–255), or `null` for anything else (rgba, names…). */
export function parseHex(value: string): Rgb | null {
  const v = value.trim();
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(v);
  if (short) {
    return [short[1]!, short[2]!, short[3]!].map((c) => parseInt(c + c, 16)) as Rgb;
  }
  const long = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(v);
  if (long) {
    return [long[1]!, long[2]!, long[3]!].map((c) => parseInt(c, 16)) as Rgb;
  }
  return null;
}

/** `#rgb`/`#rrggbb` or `rgb(a)(...)` -> `{ rgb, alpha }` (alpha defaults to 1), or `null` for
 * anything else. H-79: the compositing counterpart to {@link parseHex}, which only accepts opaque
 * hex — this also accepts the `rgba()` shape our translucent wash tokens use. */
export function parseCssColor(value: string): { rgb: Rgb; alpha: number } | null {
  const hex = parseHex(value);
  if (hex) {
    return { rgb: hex, alpha: 1 };
  }
  const m = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(value.trim());
  if (!m) {
    return null;
  }
  return {
    rgb: [Number(m[1]), Number(m[2]), Number(m[3])],
    alpha: m[4] !== undefined ? Number(m[4]) : 1,
  };
}

/** Alpha-composites `fg` (hex or `rgba()`) over the opaque hex `bg`, returning an opaque hex —
 * i.e. what a viewer actually sees when a translucent wash sits over a surface (H-79). An already
 * opaque `fg` normalizes straight through. */
export function compositeOver(fg: string, bg: string): string {
  const f = parseCssColor(fg);
  const b = parseHex(bg);
  if (!f || !b) {
    throw new Error(`compositeOver needs a resolvable colour and an opaque hex bg, got ${fg} / ${bg}`);
  }
  const [r, g, bl] = f.rgb.map((c, i) => Math.round(c * f.alpha + b[i]! * (1 - f.alpha)));
  return `#${[r, g, bl].map((c) => c!.toString(16).padStart(2, "0")).join("")}`;
}

function channel(c: number): number {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

export function relativeLuminance([r, g, b]: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG contrast ratio between two opaque hex colours (order-independent). */
export function contrastRatio(a: string, b: string): number {
  const ra = parseHex(a);
  const rb = parseHex(b);
  if (!ra || !rb) {
    throw new Error(`contrastRatio needs opaque hex colours, got ${a} / ${b}`);
  }
  const la = relativeLuminance(ra);
  const lb = relativeLuminance(rb);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

function parseDeclarations(body: string): ThemeTokens {
  const tokens: ThemeTokens = {};
  const withoutComments = body.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const match of withoutComments.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    tokens[match[1]!] = match[2]!.trim();
  }
  return tokens;
}

/**
 * Splits `design-tokens.css` into one token map per theme, keyed by the `[data-theme="…"]` name
 * in the block's selector (`dark`, `light`, `high-contrast`). Theme-independent scales and
 * media-query blocks are ignored.
 */
export function parseThemes(css: string): Record<string, ThemeTokens> {
  const themes: Record<string, ThemeTokens> = {};
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const match of clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = match[1]!;
    const theme = /\[data-theme="([\w-]+)"\]/.exec(selector);
    if (theme) {
      themes[theme[1]!] = parseDeclarations(match[2]!);
    }
  }
  return themes;
}

/** A token's value with `var(--x)` chains followed, or `null` if it (or a link) is undefined. */
export function resolveValue(theme: ThemeTokens, name: string, depth = 0): string | null {
  const value = theme[name];
  if (value === undefined || depth > 8) {
    return null;
  }
  const ref = /^var\((--[\w-]+)\)$/.exec(value);
  return ref ? resolveValue(theme, ref[1]!, depth + 1) : value;
}

/** Resolves a role (following `var(--x)` chains) to an opaque hex colour, or `null`. */
export function resolveColor(theme: ThemeTokens, role: string): string | null {
  const value = resolveValue(theme, role);
  return value !== null && parseHex(value) ? value : null;
}
