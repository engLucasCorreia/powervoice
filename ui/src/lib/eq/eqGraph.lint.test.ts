import { describe, expect, it } from "vitest";

/**
 * AC-17 (graph draws Rust's curve only): "The UI bundle for the graph imports no filter or taper
 * math (a lint test greps `ui/src/lib/eq/` for `Math.tan`, `sin`, `cos` and `pow` outside
 * `freqAxis.ts`)." `freqAxis.ts` is the one file allowed to compute the log-frequency mapping;
 * every other file in this directory only maps values Rust already computed onto pixels.
 *
 * Reads sibling sources as raw text through Vite's `import.meta.glob` (no Node `fs`/`@types/node`
 * dependency, consistent with this being browser/Vitest code, not a Node script).
 */
const sources = import.meta.glob(["./*.ts", "./*.svelte"], {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const FORBIDDEN = /Math\.(sin|cos|tan|pow)\b/;

function coveredFiles(): [string, string][] {
  return Object.entries(sources).filter(
    ([path]) => !path.endsWith(".test.ts") && !path.endsWith("/freqAxis.ts"),
  );
}

describe("AC-17: no filter/taper math outside freqAxis.ts", () => {
  it("lists at least the files this lint is meant to cover", () => {
    // Guards against the glob silently matching nothing (e.g. a rename breaking the pattern).
    expect(coveredFiles().length).toBeGreaterThan(0);
  });

  for (const [path, content] of coveredFiles()) {
    it(`${path} contains no Math.sin/cos/tan/pow`, () => {
      expect(FORBIDDEN.test(content)).toBe(false);
    });
  }
});
