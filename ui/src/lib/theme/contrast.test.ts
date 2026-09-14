import { describe, expect, it } from "vitest";
import {
  CONTRAST_PAIRS,
  contrastRatio,
  parseHex,
  parseThemes,
  resolveColor,
} from "./contrast";

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
});

describe("design-tokens.css themes", () => {
  const themes = parseThemes(css);

  it("loads the stylesheet text", () => {
    expect(css).toContain("--pv-bg-panel");
  });

  it("defines a dark and a light theme with the same role names", () => {
    expect(Object.keys(themes).sort()).toEqual(["dark", "light"]);
    const dark = Object.keys(themes.dark ?? {}).sort();
    const light = Object.keys(themes.light ?? {}).sort();
    expect(dark.length).toBeGreaterThan(40);
    expect(light).toEqual(dark);
  });

  it("every pair names a role that exists and resolves to an opaque colour", () => {
    for (const [name, theme] of Object.entries(themes)) {
      for (const pair of CONTRAST_PAIRS) {
        for (const role of [pair.fg, pair.bg]) {
          const value = resolveColor(theme, role);
          expect(value, `${name}: ${role}`).not.toBeNull();
        }
      }
    }
  });

  for (const themeName of ["dark", "light"] as const) {
    describe(`${themeName} theme meets WCAG AA`, () => {
      for (const pair of CONTRAST_PAIRS) {
        it(`${pair.fg} on ${pair.bg} ≥ ${pair.min}:1 (${pair.use})`, () => {
          const theme = themes[themeName] ?? {};
          const fg = resolveColor(theme, pair.fg);
          const bg = resolveColor(theme, pair.bg);
          if (!fg || !bg) {
            throw new Error(`unresolved ${pair.fg} / ${pair.bg}`);
          }
          expect(contrastRatio(fg, bg)).toBeGreaterThanOrEqual(pair.min);
        });
      }
    });
  }
});
