import type { ThemePref } from "../ipc/bindings";

/**
 * H-25 theme switching (design-system §15): the resolved theme is stamped as `data-theme` on
 * <html>; `design-tokens.css` + `theme-bridge.css` do the rest. Dark is the default (a missing
 * attribute also renders dark). "Match system" follows `prefers-color-scheme` live. Canvas
 * renderers read their colour tokens on every draw, so they pick the new theme up on their next
 * frame (the analyzer immediately; waveform/spectrogram/EQ on their next redraw).
 */
export type ResolvedTheme = "dark" | "light";

export function resolveTheme(pref: ThemePref, prefersLight: boolean): ResolvedTheme {
  if (pref === "light") {
    return "light";
  }
  if (pref === "system") {
    return prefersLight ? "light" : "dark";
  }
  return "dark";
}

let current = $state<ThemePref>("dark");
let media: MediaQueryList | null = null;

function osPrefersLight(): boolean {
  return media?.matches ?? false;
}

function stamp(): void {
  document.documentElement.dataset.theme = resolveTheme(current, osPrefersLight());
}

function onOsChange(): void {
  stamp();
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
  stamp();
}

export function themeState(): { readonly pref: ThemePref } {
  return {
    get pref() {
      return current;
    },
  };
}

export function resetThemeForTest(): void {
  applyThemePref("dark");
  delete document.documentElement.dataset.theme;
}
