import { describe, expect, it } from "vitest";
import {
  CONTRAST_PAIRS,
  compositeOver,
  contrastRatio,
  minRatio,
  parseCssColor,
  parseHex,
  parseThemes,
  resolveColor,
  resolveValue,
  type ContrastPair,
  type ThemeTokens,
} from "./contrast";
import { RESOLVED_THEMES } from "./theme.svelte";

// Raw text through Vite's `import.meta.glob` (the eqGraph lint test's recipe — no Node `fs`); a
// plain `?raw` import of a `.css` file comes back empty under Vitest's CSS handling.
const files = import.meta.glob("./design-tokens.css", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const css = files["./design-tokens.css"] ?? "";

describe("contrast math (WCAG 2.2 relative luminance)", () => {
  it("black on white is 21:1 and a colour on itself is 1:1", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#4da3ff", "#4da3ff")).toBeCloseTo(1, 5);
  });

  it("matches known reference values", () => {
    // #767676 on white is the classic "just passes AA" grey (4.54:1).
    expect(contrastRatio("#767676", "#ffffff")).toBeCloseTo(4.54, 2);
    expect(contrastRatio("#ffffff", "#767676")).toBeCloseTo(4.54, 2);
  });

  it("parses #rgb and #rrggbb, rejects everything else", () => {
    expect(parseHex("#fff")).toEqual([255, 255, 255]);
    expect(parseHex("#0c0E11")).toEqual([12, 14, 17]);
    expect(parseHex("rgba(0, 0, 0, 0.5)")).toBeNull();
    expect(parseHex("#12345")).toBeNull();
  });

  it("parseCssColor accepts hex (alpha 1) and rgba(), rejects everything else (H-79)", () => {
    expect(parseCssColor("#4da3ff")).toEqual({ rgb: [77, 163, 255], alpha: 1 });
    expect(parseCssColor("rgba(77, 163, 255, 0.22)")).toEqual({ rgb: [77, 163, 255], alpha: 0.22 });
    expect(parseCssColor("rgb(77, 163, 255)")).toEqual({ rgb: [77, 163, 255], alpha: 1 });
    expect(parseCssColor("not-a-color")).toBeNull();
  });

  it("compositeOver alpha-blends a translucent colour onto an opaque bg (H-79)", () => {
    // 50% white over black -> mid-gray.
    expect(compositeOver("rgba(255, 255, 255, 0.5)", "#000000")).toBe("#808080");
    // Fully opaque fg passes through unchanged (just re-cased).
    expect(compositeOver("#4da3ff", "#000000")).toBe("#4da3ff");
    // Fully transparent fg is indistinguishable from the bg.
    expect(compositeOver("rgba(255, 0, 0, 0)", "#123456")).toBe("#123456");
  });
});

describe("design-tokens.css themes", () => {
  const themes = parseThemes(css);

  it("loads the stylesheet text", () => {
    expect(css).toContain("--pv-bg-panel");
  });

  it("has exactly one block per theme the app can resolve to (data-driven, T-708)", () => {
    expect(Object.keys(themes).sort()).toEqual([...RESOLVED_THEMES].sort());
  });

  it("every theme block defines the same tokens — chrome and audio content", () => {
    const dark = Object.keys(themes.dark ?? {}).sort();
    expect(dark.length).toBeGreaterThan(100);
    expect(dark).toContain("--wave-bg");
    expect(dark).toContain("--eq-band-lp");
    for (const name of RESOLVED_THEMES) {
      expect(Object.keys(themes[name] ?? {}).sort(), name).toEqual(dark);
    }
  });

  it("every pair names a role that exists and resolves to an opaque colour", () => {
    for (const [name, theme] of Object.entries(themes)) {
      for (const pair of CONTRAST_PAIRS) {
        // H-79: `fg`/`bg` paired with `fgOver`/`bgOver` may resolve to a translucent `rgba()` wash
        // instead — that's fine, since the ratio check composites it before comparing (below).
        if (!pair.fgOver) {
          expect(resolveColor(theme, pair.fg), `${name}: ${pair.fg}`).not.toBeNull();
        }
        if (!pair.bgOver) {
          expect(resolveColor(theme, pair.bg), `${name}: ${pair.bg}`).not.toBeNull();
        }
      }
    }
  });

  it("H-79: a fg/bg paired with fgOver/bgOver at least resolves to *some* parseable colour", () => {
    for (const [name, theme] of Object.entries(themes)) {
      for (const pair of CONTRAST_PAIRS) {
        for (const [role, over] of [
          [pair.fg, pair.fgOver],
          [pair.bg, pair.bgOver],
        ] as const) {
          if (over) {
            const value = resolveValue(theme, role);
            expect(value, `${name}: ${role}`).not.toBeNull();
            expect(parseCssColor(value!), `${name}: ${role} = ${value}`).not.toBeNull();
            expect(resolveColor(theme, over), `${name}: ${over}`).not.toBeNull();
          }
        }
      }
    }
  });

  it("High Contrast asks for AAA text (7:1) and 4.5:1 indicators", () => {
    const text = CONTRAST_PAIRS.find((p) => p.fg === "--pv-text-tertiary")!;
    const ring = CONTRAST_PAIRS.find((p) => p.fg === "--pv-focus-ring")!;
    expect(minRatio(text, "high-contrast")).toBe(7);
    expect(minRatio(ring, "high-contrast")).toBe(4.5);
    expect(minRatio(text, "light")).toBe(4.5);
    expect(minRatio(ring, "dark")).toBe(3);
  });

  /** H-79: resolves `role` to an opaque hex — compositing it over `over`'s opaque colour first
   * when the pair says to (a translucent wash), otherwise the plain opaque resolution as before. */
  function resolvedOrComposited(theme: ThemeTokens, role: string, over: string | undefined): string | null {
    if (!over) {
      return resolveColor(theme, role);
    }
    const value = resolveValue(theme, role);
    const overColor = resolveColor(theme, over);
    return value && overColor ? compositeOver(value, overColor) : null;
  }

  for (const themeName of RESOLVED_THEMES) {
    describe(`${themeName} theme meets its contrast bar`, () => {
      for (const pair of CONTRAST_PAIRS) {
        const min = minRatio(pair, themeName);
        it(`${pair.fg} on ${pair.bg} ≥ ${min}:1 (${pair.use})`, () => {
          const theme = themes[themeName] ?? {};
          const fg = resolvedOrComposited(theme, pair.fg, pair.fgOver);
          const bg = resolvedOrComposited(theme, pair.bg, pair.bgOver);
          if (!fg || !bg) {
            throw new Error(`unresolved ${pair.fg} / ${pair.bg}`);
          }
          expect(contrastRatio(fg, bg)).toBeGreaterThanOrEqual(min);
        });
      }
    });
  }
});
