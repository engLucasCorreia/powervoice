import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyThemePref,
  isThemePref,
  resetThemeForTest,
  resolveTheme,
  THEME_STORAGE_KEY,
  themeState,
} from "./theme.svelte";

const originalMatchMedia = window.matchMedia;

function stubMatchMedia(prefersLight: boolean): { fire: (light: boolean) => void } {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  let matches = prefersLight;
  window.matchMedia = vi.fn().mockImplementation(() => ({
    get matches() {
      return matches;
    },
    addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => listeners.add(cb),
    removeEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => listeners.delete(cb),
  }));
  return {
    fire: (light: boolean) => {
      matches = light;
      listeners.forEach((cb) => cb({ matches: light } as MediaQueryListEvent));
    },
  };
}

afterEach(() => {
  resetThemeForTest();
  window.matchMedia = originalMatchMedia;
});

describe("resolveTheme", () => {
  it("dark, light and high contrast are explicit; system follows the OS", () => {
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("high_contrast", true)).toBe("high-contrast");
    expect(resolveTheme("high_contrast", false)).toBe("high-contrast");
  });

  it("recognises exactly the offered preferences", () => {
    expect(isThemePref("high_contrast")).toBe(true);
    expect(isThemePref("system")).toBe(true);
    expect(isThemePref("high-contrast")).toBe(false);
    expect(isThemePref("sepia")).toBe(false);
    expect(isThemePref(null)).toBe(false);
  });
});

describe("applyThemePref", () => {
  it("stamps data-theme on <html> and remembers the preference", () => {
    applyThemePref("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(themeState().pref).toBe("light");
    applyThemePref("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("Match system follows OS changes, and stops following once another choice is made", () => {
    const os = stubMatchMedia(true);
    applyThemePref("system");
    expect(document.documentElement.dataset.theme).toBe("light");
    os.fire(false);
    expect(document.documentElement.dataset.theme).toBe("dark");
    applyThemePref("light");
    os.fire(false);
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("mirrors the preference for the pre-mount script and hands color-scheme back to the tokens", () => {
    document.documentElement.style.colorScheme = "light";
    applyThemePref("high_contrast");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("high_contrast");
    expect(document.documentElement.dataset.theme).toBe("high-contrast");
    expect(document.documentElement.style.colorScheme).toBe("");
    applyThemePref("system");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("system");
  });

  it("bumps the revision only when the resolved theme changes", () => {
    const os = stubMatchMedia(false);
    applyThemePref("dark");
    const start = themeState().revision;
    applyThemePref("dark");
    expect(themeState().revision).toBe(start);
    applyThemePref("system"); // dark OS → still dark
    expect(themeState().revision).toBe(start);
    os.fire(true); // OS goes light, live
    expect(themeState().resolved).toBe("light");
    expect(themeState().revision).toBe(start + 1);
    applyThemePref("high_contrast");
    expect(themeState().revision).toBe(start + 2);
  });

  it("works without matchMedia (treated as a dark OS)", () => {
    // @ts-expect-error — simulate an environment without matchMedia
    window.matchMedia = undefined;
    applyThemePref("system");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
