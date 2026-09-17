import { flushSync } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
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
