import { flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { parseThemes, resolveValue } from "./contrast";
import { applyThemePref, resetThemeForTest } from "./theme.svelte";
import { crispOffset, eqBandColor, readThemeColors, resetThemeColorsForTest, themeColors } from "./themeColors";

const files = import.meta.glob("./design-tokens.css", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const themes = parseThemes(files["./design-tokens.css"] ?? "");
const token = (theme: string, name: string) => resolveValue(themes[theme] ?? {}, name);

afterEach(() => {
  vi.restoreAllMocks();
  resetThemeForTest();
  resetThemeColorsForTest();
  document.documentElement.style.removeProperty("--wave-bg");
});

describe("themeColors (T-708)", () => {
  it("reads every renderer colour from the theme's tokens — no hard-coded values", () => {
    for (const theme of ["dark", "light", "high-contrast"] as const) {
      const c = readThemeColors(theme);
      expect(c.wave.bg.css).toBe(token(theme, "--wave-bg"));
      expect(c.wave.fill.css).toBe(token(theme, "--wave-fill"));
      expect(c.wave.playhead.css).toBe(token(theme, "--wave-playhead"));
      expect(c.wave.record.css).toBe(token(theme, "--pv-record"));
      expect(c.spec.bg.css).toBe(token(theme, "--spec-bg"));
      expect(c.analyzer.bg.css).toBe(token(theme, "--pv-bg-inset"));
      expect(c.analyzer.peak.css).toBe(token(theme, "--analyzer-peak"));
      expect(c.eq.curve.css).toBe(token(theme, "--eq-curve"));
      expect(eqBandColor(c, "ls").css).toBe(token(theme, "--eq-band-ls"));
      // Every colour parsed into a usable RGBA (not the mid-grey "unparsed" fallback).
      for (const group of [c.wave, c.spec, c.analyzer]) {
        for (const [name, value] of Object.entries(group)) {
          expect(value.css, `${theme} ${name}`).not.toBe("");
          expect(value.rgba, `${theme} ${name}`).not.toEqual([0.5, 0.5, 0.5, 1]);
        }
      }
    }
  });

  it("parses translucent fills into RGBA with their alpha", () => {
    const c = readThemeColors("dark");
    // H-79 (SPEC-006 §2.12 Amendment 2): the selection wash's alpha.
    expect(c.wave.selectionFill.rgba[3]).toBeCloseTo(0.28, 5);
    expect(c.wave.bg.rgba[3]).toBe(1);
  });

  it("High Contrast draws thicker content lines than Dark and Light", () => {
    expect(readThemeColors("dark").strokePx).toBe(1);
    expect(readThemeColors("light").strokePx).toBe(1);
    expect(readThemeColors("high-contrast").strokePx).toBe(2);
    expect(readThemeColors("high-contrast").emphasisStrokePx).toBeGreaterThan(readThemeColors("dark").emphasisStrokePx);
  });

  it("is cached until the theme changes, then invalidated (live repaint, no reload)", () => {
    applyThemePref("dark");
    const first = themeColors();
    expect(themeColors()).toBe(first);
    expect(first.theme).toBe("dark");

    applyThemePref("light");
    const light = themeColors();
    expect(light).not.toBe(first);
    expect(light.theme).toBe("light");
    expect(light.wave.bg.css).toBe(token("light", "--wave-bg"));
    expect(themeColors()).toBe(light);

    applyThemePref("high_contrast");
    expect(themeColors().theme).toBe("high-contrast");
  });

  it("invalidates when data-theme is stamped from outside applyThemePref", () => {
    applyThemePref("dark");
    const dark = themeColors();
    document.documentElement.dataset.theme = "light";
    expect(themeColors()).not.toBe(dark);
    expect(themeColors().theme).toBe("light");
  });

  it("prefers the live cascade (getComputedStyle) over the token file", () => {
    applyThemePref("dark");
    document.documentElement.style.setProperty("--wave-bg", "#123456");
    const c = readThemeColors("dark");
    // jsdom may not expose inline custom properties through getComputedStyle; when it does, the
    // cascade wins.
    const live = getComputedStyle(document.documentElement).getPropertyValue("--wave-bg").trim();
    expect(c.wave.bg.css).toBe(live || token("dark", "--wave-bg"));
  });

  it("an $effect that reads themeColors() re-runs on a theme switch", () => {
    applyThemePref("dark");
    const seen: string[] = [];
    const cleanup = $effect.root(() => {
      $effect(() => {
        seen.push(themeColors().theme);
      });
    });
    flushSync();
    applyThemePref("light");
    flushSync();
    applyThemePref("light");
    flushSync();
    cleanup();
    expect(seen).toEqual(["dark", "light"]);
  });

  it("crispOffset centres odd widths on half pixels", () => {
    expect(crispOffset(1)).toBe(0.5);
    expect(crispOffset(2)).toBe(0);
    expect(crispOffset(3)).toBe(0.5);
  });
});

/** What `vite build`'s CSS minifier turns a colour token into: `rgba(r, g, b, a)` → `#rrggbbaa`,
 * a shortenable `#rrggbb` → `#rgb` (H-121 — observed in `ui/dist/assets/index-*.css`). */
function minifiedForm(value: string): string {
  const hex2 = (n: number) => Math.round(n).toString(16).padStart(2, "0");
  const rgba = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(value);
  if (rgba) {
    const alpha = rgba[4] === undefined ? "" : hex2(Number(rgba[4]) * 255);
    return `#${hex2(Number(rgba[1]))}${hex2(Number(rgba[2]))}${hex2(Number(rgba[3]))}${alpha}`;
  }
  const long = /^#([0-9a-f])\1([0-9a-f])\2([0-9a-f])\3$/i.exec(value);
  return long ? `#${long[1]}${long[2]}${long[3]}` : value;
}

/** Every `{ css, rgba }` colour in a snapshot, keyed by its path (`wave.selectionFill`, …). */
function allColors(value: unknown, path = ""): [string, { css: string; rgba: readonly number[] }][] {
  if (value && typeof value === "object") {
    if ("css" in value && "rgba" in value) {
      return [[path, value as { css: string; rgba: readonly number[] }]];
    }
    return Object.entries(value).flatMap(([key, child]) => allColors(child, path ? `${path}.${key}` : key));
  }
  return [];
}

/** Stubs the live cascade so every token reads back in its production-minified form. */
function stubMinifiedCascade(theme: string): void {
  const tokens = themes[theme] ?? {};
  vi.spyOn(window, "getComputedStyle").mockImplementation(
    () =>
      ({
        getPropertyValue: (name: string) => {
          const source = resolveValue(tokens, name);
          return source === null ? "" : minifiedForm(source);
        },
      }) as unknown as CSSStyleDeclaration,
  );
}

/** `gl.blendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)` into an `alpha: false` backbuffer, per channel —
 * exactly how both WebGL2 renderers composite the selection wash over the content under it. */
function blendOver(src: readonly number[], dst: readonly number[]): number[] {
  const a = src[3]!;
  return [0, 1, 2].map((i) => Math.round((src[i]! * a + dst[i]! * (1 - a)) * 255));
}

describe("themeColors in the production build (H-121)", () => {
  // H-121: `vite build` minifies design-tokens.css (`rgba(233, 99, 184, 0.28)` → `#e963b847`,
  // `#ffffff` → `#fff`). Every earlier check ran the dev server, which serves the CSS as written,
  // so no test or screenshot ever saw the shipped token forms — and the WebGL2 renderers, which
  // parse colours themselves, drew the selection as the parser's opaque mid-gray fallback.
  it("reads every colour identically when the minifier has rewritten the tokens (#rrggbbaa, #rgb)", () => {
    for (const theme of ["dark", "light", "high-contrast"] as const) {
      const source = new Map(allColors(readThemeColors(theme)));
      stubMinifiedCascade(theme);
      const shipped = allColors(readThemeColors(theme));
      vi.restoreAllMocks();
      let changedForm = 0;
      for (const [path, color] of shipped) {
        const expected = source.get(path)!;
        if (color.css !== expected.css) {
          changedForm += 1;
        }
        color.rgba.forEach((channel, i) => {
          expect(channel, `${theme} ${path} (${color.css} vs ${expected.css}) channel ${i}`).toBeCloseTo(
            expected.rgba[i]!,
            2,
          );
        });
      }
      // The stub really exercised the minified forms (the selection wash at least).
      expect(changedForm, theme).toBeGreaterThan(0);
    }
  });

  it("asks the browser to canonicalize a colour form the parser doesn't know (e.g. a named colour)", () => {
    resetThemeColorsForTest();
    const fillStyles: string[] = [];
    const fake = {
      set fillStyle(value: string) {
        fillStyles.push(value);
      },
      get fillStyle() {
        const last = fillStyles.at(-1)!;
        if (last === "tan") {
          return "#d2b48c";
        }
        // A rejected value leaves fillStyle at the previous (sentinel) colour, like a browser.
        return last === "transparent" || last === "not-a-colour" ? "rgba(0, 0, 0, 0)" : last;
      },
    };
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(fake as unknown as RenderingContext);
    vi.spyOn(window, "getComputedStyle").mockImplementation(
      () =>
        ({
          getPropertyValue: (name: string) =>
            name === "--wave-marker" ? "tan" : name === "--wave-loop" ? "not-a-colour" : "",
        }) as unknown as CSSStyleDeclaration,
    );
    const c = readThemeColors("dark");
    expect(c.wave.marker.rgba).toEqual([210 / 255, 180 / 255, 140 / 255, 1]);
    expect(c.wave.loop.rgba).toEqual([0.5, 0.5, 0.5, 1]);
  });

  it("the shipped selection wash tints the content under it instead of hiding it (pixel maths of the GL blend)", () => {
    for (const theme of ["dark", "light", "high-contrast"] as const) {
      stubMinifiedCascade(theme);
      const c = readThemeColors(theme);
      vi.restoreAllMocks();
      const fill = c.wave.selectionFill.rgba;
      expect(c.wave.selectionFill.css, theme).toMatch(/^#[0-9a-f]{8}$/);
      expect(fill[3], theme).toBeGreaterThan(0);
      expect(fill[3], theme).toBeLessThan(0.5);
      // Two very different spectrogram pixels (Inferno's dark purple and its bright orange) must
      // stay clearly different under the wash — an opaque fill makes them the same output pixel.
      const purple = [92 / 255, 36 / 255, 106 / 255];
      const orange = [244 / 255, 133 / 255, 61 / 255];
      const a = blendOver(fill, purple);
      const b = blendOver(fill, orange);
      const diff = Math.abs(a[0]! - b[0]!) + Math.abs(a[1]! - b[1]!) + Math.abs(a[2]! - b[2]!);
      expect(diff, theme).toBeGreaterThan(150);
    }
  });
});
