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

const LIGHT_QUERY = "(prefers-color-scheme: light)";
const CONTRAST_QUERY = "(prefers-contrast: more)";

/** Stubs `window.matchMedia` for both queries `theme.svelte.ts` listens to (A-019: the OS
 * light/dark preference and the OS "more contrast" preference), each with its own listener set so
 * firing one never fires the other. */
function stubMatchMedia(
  prefersLight: boolean,
  prefersContrastMore = false,
): { fire: (light: boolean) => void; fireContrast: (contrastMore: boolean) => void } {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  const contrastListeners = new Set<(e: MediaQueryListEvent) => void>();
  let matches = prefersLight;
  let contrastMatches = prefersContrastMore;
  window.matchMedia = vi.fn().mockImplementation((query: string) => {
    if (query === CONTRAST_QUERY) {
      return {
        get matches() {
          return contrastMatches;
        },
        addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => contrastListeners.add(cb),
        removeEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => contrastListeners.delete(cb),
      };
    }
    return {
      get matches() {
        return matches;
      },
      addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => listeners.add(cb),
      removeEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => listeners.delete(cb),
    };
  });
  return {
    fire: (light: boolean) => {
      matches = light;
      listeners.forEach((cb) => cb({ matches: light } as MediaQueryListEvent));
    },
    fireContrast: (contrastMore: boolean) => {
      contrastMatches = contrastMore;
      contrastListeners.forEach((cb) => cb({ matches: contrastMore } as MediaQueryListEvent));
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

  it("system resolves to High Contrast when the OS asks for more contrast (A-019)", () => {
    expect(resolveTheme("system", true, true)).toBe("high-contrast");
    expect(resolveTheme("system", false, true)).toBe("high-contrast");
    expect(resolveTheme("system", true, false)).toBe("light");
    expect(resolveTheme("system", false, false)).toBe("dark");
  });

  it("an explicit choice ignores prefers-contrast: more (A-019)", () => {
    expect(resolveTheme("dark", true, true)).toBe("dark");
    expect(resolveTheme("light", false, true)).toBe("light");
    expect(resolveTheme("high_contrast", true, false)).toBe("high-contrast");
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

  it("Match system switches live to High Contrast when the OS asks for more contrast (A-019)", () => {
    const os = stubMatchMedia(true);
    applyThemePref("system");
    expect(document.documentElement.dataset.theme).toBe("light");
    os.fireContrast(true);
    expect(document.documentElement.dataset.theme).toBe("high-contrast");
    os.fireContrast(false);
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("stops following prefers-contrast: more once another choice is made (A-019)", () => {
    const os = stubMatchMedia(false, true);
    applyThemePref("system");
    expect(document.documentElement.dataset.theme).toBe("high-contrast");
    applyThemePref("dark");
    os.fireContrast(false);
    expect(document.documentElement.dataset.theme).toBe("dark");
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
