/**
 * WCAG 2.2 contrast checking for the design tokens (H-25). Pure and DOM-free: the test parses
 * `design-tokens.css` as text (jsdom doesn't compute custom properties from stylesheets reliably),
 * resolves `var(--x)` references inside one theme, and checks every pair below.
 *
 * Text pairs need 4.5:1 (AA, normal text — every PowerVoice UI string is < 18.66 px bold/24 px);
 * non-text indicators (focus ring, selected border, slider fill, record lamp, meter boundaries)
 * need 3:1 (WCAG 1.4.11). Disabled text is exempt and deliberately absent.
 */

export type Rgb = [number, number, number];
export type ThemeTokens = Record<string, string>;

export interface ContrastPair {
  fg: string;
  bg: string;
  min: number;
  use: string;
}

const AA_TEXT = 4.5;
const AA_NON_TEXT = 3;

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
  for (const match of withoutComments.matchAll(/(--pv-[\w-]+)\s*:\s*([^;]+);/g)) {
    tokens[match[1]!] = match[2]!.trim();
  }
  return tokens;
}

/**
 * Splits `design-tokens.css` into `{ dark, light }` role maps: the dark block is the one whose
 * selector contains `[data-theme="dark"]`, the light block `[data-theme="light"]`. Theme-
 * independent scales and media-query blocks are ignored.
 */
export function parseThemes(css: string): Record<string, ThemeTokens> {
  const themes: Record<string, ThemeTokens> = {};
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const match of clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = match[1]!;
    const theme = /\[data-theme="(\w+)"\]/.exec(selector);
    if (theme) {
      themes[theme[1]!] = parseDeclarations(match[2]!);
    }
  }
  return themes;
}

/** Resolves a role (following `var(--x)` chains) to an opaque hex colour, or `null`. */
export function resolveColor(theme: ThemeTokens, role: string, depth = 0): string | null {
  const value = theme[role];
  if (value === undefined || depth > 8) {
    return null;
  }
  const ref = /^var\((--[\w-]+)\)$/.exec(value);
  if (ref) {
    return resolveColor(theme, ref[1]!, depth + 1);
  }
  return parseHex(value) ? value : null;
}
