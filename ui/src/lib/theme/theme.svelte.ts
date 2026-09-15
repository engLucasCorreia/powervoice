import type { ThemePref } from "../ipc/bindings";

/**
 * Theme switching (H-25, T-708; design-system §15). The preference (`Settings.theme`) resolves to
 * one of the token blocks in `design-tokens.css`, stamped as `data-theme` on <html>. Dark is the
 * default (A-018); "Match system" follows `prefers-color-scheme` live.
 *
 * Data-driven: `THEMES` lists every choice the UI offers (Preferences → Appearance, View → Theme);
 * `RESOLVED_THEMES` every token block. A new theme = a block in `design-tokens.css`, an entry in
 * both lists, and a Rust `ThemePref` variant (tests fail if these drift apart).
 *
 * No flash at start-up: the preference is mirrored into `localStorage` (`THEME_STORAGE_KEY`) and
 * index.html's inline script stamps it before the stylesheet or the app loads; `Settings` stays
 * the source of truth and is re-applied once it loads.
 *
 * Canvas/WebGL renderers read colours through `themeColors()`, which is keyed on `revision`
 * (bumped whenever the resolved theme changes), so they repaint in the new theme on their next
 * frame without a reload.
 */
export type ResolvedTheme = "dark" | "light" | "high-contrast";

/** Every token block in `design-tokens.css`, by its `data-theme` name. */
export const RESOLVED_THEMES: readonly ResolvedTheme[] = ["dark", "light", "high-contrast"];

export interface ThemeChoice {
  pref: ThemePref;
  /** Preferences → Appearance (sentence case). */
  labelKey: string;
  /** One-line caption under the swatch. */
  captionKey: string;
  /** View → Theme ▸ (menu title case). */
  menuLabelKey: string;
}

/** The choices the UI offers, in display order. */
export const THEMES: readonly ThemeChoice[] = [
  {
    pref: "dark",
    labelKey: "preferences.theme.dark",
    captionKey: "preferences.theme.dark.caption",
    menuLabelKey: "menu.view.theme_dark",
  },
  {
    pref: "light",
    labelKey: "preferences.theme.light",
    captionKey: "preferences.theme.light.caption",
    menuLabelKey: "menu.view.theme_light",
  },
  {
    pref: "system",
    labelKey: "preferences.theme.system",
    captionKey: "preferences.theme.system.caption",
    menuLabelKey: "menu.view.theme_system",
  },
  {
    pref: "high_contrast",
    labelKey: "preferences.theme.high_contrast",
    captionKey: "preferences.theme.high_contrast.caption",
    menuLabelKey: "menu.view.theme_high_contrast",
  },
];

/** Where the preference is mirrored for index.html's pre-mount script. */
export const THEME_STORAGE_KEY = "powervoice.theme";

export function isThemePref(value: unknown): value is ThemePref {
  return THEMES.some((choice) => choice.pref === value);
}

export function resolveTheme(pref: ThemePref, prefersLight: boolean): ResolvedTheme {
  switch (pref) {
    case "light":
      return "light";
    case "high_contrast":
      return "high-contrast";
    case "system":
      return prefersLight ? "light" : "dark";
    default:
      return "dark";
  }
}

let current = $state<ThemePref>("dark");
let resolved = $state<ResolvedTheme>("dark");
let revision = $state(0);
let media: MediaQueryList | null = null;

function osPrefersLight(): boolean {
  return media?.matches ?? false;
}

function stamp(): void {
  const next = resolveTheme(current, osPrefersLight());
  const root = document.documentElement;
  root.dataset.theme = next;
  // index.html sets an inline `color-scheme` for the unstyled first paint; from here on the
  // token blocks own it (an inline value would outlive the next switch).
  root.style.removeProperty("color-scheme");
  if (next !== resolved) {
    resolved = next;
    revision += 1;
  }
}

function onOsChange(): void {
  stamp();
}

function remember(pref: ThemePref): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, pref);
  } catch {
    // Storage can be unavailable (private mode, tests); only the start-up flash is affected.
  }
}

export function applyThemePref(pref: ThemePref): void {
  current = pref;
  if (pref === "system" && media === null && typeof window.matchMedia === "function") {
    media = window.matchMedia("(prefers-color-scheme: light)");
    media.addEventListener("change", onOsChange);
  } else if (pref !== "system" && media !== null) {
    media.removeEventListener("change", onOsChange);
    media = null;
  }
  remember(pref);
  stamp();
}

/** Reactive view: the preference, what it resolved to, and a counter bumped on every change of
 * the resolved theme (renderers key their colour caches on it). */
export function themeState(): {
  readonly pref: ThemePref;
  readonly resolved: ResolvedTheme;
  readonly revision: number;
} {
  return {
    get pref() {
      return current;
    },
    get resolved() {
      return resolved;
    },
    get revision() {
      return revision;
    },
  };
}

export function resetThemeForTest(): void {
  applyThemePref("dark");
  delete document.documentElement.dataset.theme;
  try {
    localStorage.removeItem(THEME_STORAGE_KEY);
  } catch {
    // ignore
  }
}
