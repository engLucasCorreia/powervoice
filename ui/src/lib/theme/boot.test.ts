import { afterEach, describe, expect, it, vi } from "vitest";
import type { ThemePref } from "../ipc/bindings";
import { RESOLVED_THEMES, resolveTheme, THEME_STORAGE_KEY, THEMES } from "./theme.svelte";

/**
 * T-708 "no flash of the wrong theme": index.html's inline script stamps `data-theme` from the
 * mirrored preference before anything paints. This runs that exact script (extracted from
 * index.html) and checks it against `resolveTheme()` for every preference and OS setting.
 */
const html = Object.values(
  import.meta.glob("/index.html", { eager: true, query: "?raw", import: "default" }) as Record<string, string>,
)[0] ?? "";

function bootScript(): string {
  const match = /<script>([\s\S]*?)<\/script>/.exec(html);
  if (!match) {
    throw new Error("index.html has no inline boot script");
  }
  return match[1]!;
}

const originalMatchMedia = window.matchMedia;

function runBoot(stored: string | null, osLight: boolean): { theme: string | null; scheme: string } {
  const root = document.documentElement;
  root.removeAttribute("data-theme");
  root.style.removeProperty("color-scheme");
  if (stored === null) {
    localStorage.removeItem(THEME_STORAGE_KEY);
  } else {
    localStorage.setItem(THEME_STORAGE_KEY, stored);
  }
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: query === "(prefers-color-scheme: light)" && osLight,
  }));
  new Function(bootScript())();
  return { theme: root.getAttribute("data-theme"), scheme: root.style.colorScheme };
}

afterEach(() => {
  window.matchMedia = originalMatchMedia;
  localStorage.removeItem(THEME_STORAGE_KEY);
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.style.removeProperty("color-scheme");
});

describe("index.html pre-mount theme script (T-708)", () => {
  it("is inline in <head>, before the app's module script", () => {
    const head = html.slice(0, html.indexOf("</head>"));
    expect(head).toContain(THEME_STORAGE_KEY);
    expect(html.indexOf(THEME_STORAGE_KEY)).toBeLessThan(html.indexOf('src="/src/main.ts"'));
  });

  it("resolves every preference exactly like resolveTheme()", () => {
    for (const pref of THEMES.map((c) => c.pref) as ThemePref[]) {
      for (const osLight of [false, true]) {
        const { theme, scheme } = runBoot(pref, osLight);
        expect(theme, `${pref} / OS light=${osLight}`).toBe(resolveTheme(pref, osLight));
        expect(scheme).toBe(theme === "light" ? "light" : "dark");
        expect(RESOLVED_THEMES).toContain(theme);
      }
    }
  });

  it("falls back to Dark with nothing stored or an unknown value (A-018)", () => {
    expect(runBoot(null, true).theme).toBe("dark");
    expect(runBoot("sepia", true).theme).toBe("dark");
  });
});
