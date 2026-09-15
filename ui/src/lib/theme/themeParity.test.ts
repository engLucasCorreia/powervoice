import { describe, expect, it } from "vitest";
import type { ThemePref } from "../ipc/bindings";
import { parseHex, parseThemes, resolveValue } from "./contrast";
import { RESOLVED_THEMES, THEMES, resolveTheme } from "./theme.svelte";

// Raw stylesheet text through Vite (see contrast.test.ts; vitest.config lets these CSS files through).
const files = import.meta.glob(["./design-tokens.css", "./tokens.css", "./theme-bridge.css"], {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const tokens = files["./design-tokens.css"] ?? "";
const base = files["./tokens.css"] ?? "";
const bridge = files["./theme-bridge.css"] ?? "";
const themes = parseThemes(tokens);

const CONTENT = /^--(?:wave|spec|analyzer|eq)-|^--pv-(?:meter|clip|playhead|waveform|selection)/;
const RGBA = /^rgba\(\s*\d+,\s*\d+,\s*\d+,\s*[\d.]+\s*\)$/;

describe("theme parity (H-25, T-708)", () => {
  it("loads the stylesheets", () => {
    expect(tokens).toContain("--wave-bg");
    expect(bridge).toContain("--surface-panel");
  });

  it("audio content colours use the shapes the renderers parse (#rrggbb or rgba()) in every theme", () => {
    for (const name of RESOLVED_THEMES) {
      const theme = themes[name] ?? {};
      for (const token of Object.keys(theme).filter((t) => CONTENT.test(t))) {
        const value = resolveValue(theme, token) ?? "";
        const ok = parseHex(value) !== null || RGBA.test(value);
        expect(ok, `${name} ${token}: ${value}`).toBe(true);
      }
    }
  });

  it("stroke widths are positive px values, thicker in High Contrast", () => {
    const px = (theme: string, token: string) => Number.parseFloat(themes[theme]?.[token] ?? "");
    for (const name of RESOLVED_THEMES) {
      expect(px(name, "--pv-stroke-content"), name).toBeGreaterThan(0);
      expect(px(name, "--pv-focus-width"), name).toBeGreaterThan(0);
    }
    expect(px("high-contrast", "--pv-stroke-content")).toBeGreaterThan(px("dark", "--pv-stroke-content"));
    expect(px("high-contrast", "--pv-focus-width")).toBeGreaterThan(px("dark", "--pv-focus-width"));
  });

  it("the recording colour is a token in every theme (no hard-coded fallback)", () => {
    for (const name of RESOLVED_THEMES) {
      expect(themes[name]?.["--wave-record"]).toBe("var(--pv-record)");
      expect(themes[name]?.["--wave-record-head"]).toBe("var(--pv-record)");
    }
  });

  it("colours live only in design-tokens.css (tokens.css and the bridge hold none)", () => {
    const literal = /#[0-9a-fA-F]{3,8}\b|rgba?\(/;
    const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(literal.test(strip(base))).toBe(false);
    expect(literal.test(strip(bridge))).toBe(false);
  });

  it("every UI choice resolves to a theme that has a token block", () => {
    const prefs: ThemePref[] = THEMES.map((c) => c.pref);
    // The generated Rust enum and the UI list agree (a new variant must be offered, and vice versa).
    const all: Record<ThemePref, true> = { dark: true, light: true, system: true, high_contrast: true };
    expect([...prefs].sort()).toEqual(Object.keys(all).sort());
    for (const pref of prefs) {
      for (const os of [false, true]) {
        for (const contrastMore of [false, true]) {
          expect(RESOLVED_THEMES).toContain(resolveTheme(pref, os, contrastMore));
        }
      }
    }
  });
});
