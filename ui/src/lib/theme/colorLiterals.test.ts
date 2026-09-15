import { describe, expect, it } from "vitest";

/**
 * T-708: no component or renderer writes a colour down — every colour is a theme token (CSS
 * custom property in `design-tokens.css`, read by components with `var(--…)` and by renderers
 * through `themeColors()`). A lint-style scan of every source file for hex colours, `rgb()`/
 * `rgba()`/`hsl()`/`hsla()` and CSS named colours in declarations.
 *
 * Allowed: the token files themselves, the spectrogram colormaps (scientific data tables, not
 * theme colours), tests, and this file.
 */
const sources = import.meta.glob(["/src/**/*.ts", "/src/**/*.svelte"], {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const ALLOWLIST: readonly RegExp[] = [
  /\.test\.ts$/,
  /^\/src\/lib\/theme\/(design-tokens|tokens|theme-bridge)\.css$/,
  /^\/src\/lib\/spectrogram\/colormap\.ts$/, // Inferno/Viridis control points (SPEC-007 §2.5)
  /^\/src\/spike\/colormap\.ts$/, // the ADR-009 spike's copy of the same LUTs
];

const HEX = /#[0-9a-fA-F]{3,8}\b/g;
const FUNCTIONAL = /\b(?:rgba?|hsla?)\(\s*[\d.]/g;
// Named colours as a CSS value (`color: white;`, `background: black`) — not words in prose.
const NAMED = /:\s*(?:white|black|red|green|blue|gray|grey|yellow|orange|purple|pink|silver)\s*[;}]/g;

/** Strips comments so documentation may mention a colour (JS/TS/CSS block and line comments,
 * and HTML comments in Svelte markup). */
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1");
}

export function findColorLiterals(source: string): string[] {
  const code = stripComments(source);
  return [...code.matchAll(HEX), ...code.matchAll(FUNCTIONAL), ...code.matchAll(NAMED)].map((m) => m[0]);
}

const covered = Object.entries(sources).filter(([path]) => !ALLOWLIST.some((re) => re.test(path)));

describe("no raw colour literals outside the token files (T-708)", () => {
  it("scans the whole UI (guards against the glob silently matching nothing)", () => {
    expect(covered.length).toBeGreaterThan(150);
    expect(covered.some(([path]) => path.endsWith("/WaveformView.svelte"))).toBe(true);
    expect(covered.some(([path]) => path.endsWith("/render/quads.ts"))).toBe(true);
  });

  it("the scanner finds what it should and ignores comments/Svelte blocks", () => {
    expect(findColorLiterals(`ctx.fillStyle = "#7fc8ff";`)).toEqual(["#7fc8ff"]);
    expect(findColorLiterals(`background: rgba(0, 0, 0, 0.45);`)).toEqual(["rgba(0"]);
    expect(findColorLiterals(`  color: white;`)).toHaveLength(1);
    expect(findColorLiterals(`/* #ffffff */ // rgba(1, 2, 3, 1)\n{#each items as item}{#if a}`)).toEqual([]);
    expect(findColorLiterals(`const url = "https://example.com/#abc";`)).toEqual(["#abc"]);
  });

  for (const [path, source] of covered) {
    it(`${path} uses theme tokens only`, () => {
      expect(findColorLiterals(source)).toEqual([]);
    });
  }
});
