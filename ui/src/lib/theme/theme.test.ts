import { afterEach, describe, expect, it, vi } from "vitest";
import { applyThemePref, resetThemeForTest, resolveTheme, themeState } from "./theme.svelte";

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
  it("dark and light are explicit; system follows the OS", () => {
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
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

  it("works without matchMedia (treated as a dark OS)", () => {
    // @ts-expect-error — simulate an environment without matchMedia
    window.matchMedia = undefined;
    applyThemePref("system");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
