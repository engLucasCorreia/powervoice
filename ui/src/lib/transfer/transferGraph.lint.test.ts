import { describe, expect, it } from "vitest";

/**
 * H-63, SPEC-016 §4.11 / AC-17: the graph draws Rust's curve and nothing else. Every dB value on
 * screen comes from `rack_transfer_curve`; this directory only maps values onto pixels, so none
 * of its files may contain gain-computer or detector maths. Mirrors `eq/eqGraph.lint.test.ts`.
 *
 * Reads sibling sources as raw text through Vite's `import.meta.glob` (no Node `fs` dependency).
 */
const sources = import.meta.glob(["./*.ts", "./*.svelte"], {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const FORBIDDEN = /Math\.(sin|cos|tan|pow|log|log10|log2|exp)\b/;

function coveredFiles(): [string, string][] {
  return Object.entries(sources).filter(([path]) => !path.endsWith(".test.ts"));
}

describe("no level maths in the transfer graph", () => {
  it("lists the files this lint is meant to cover", () => {
    expect(coveredFiles().length).toBeGreaterThan(0);
  });

  for (const [path, content] of coveredFiles()) {
    it(`${path} computes no dB values of its own`, () => {
      expect(FORBIDDEN.test(content)).toBe(false);
    });
  }
});
