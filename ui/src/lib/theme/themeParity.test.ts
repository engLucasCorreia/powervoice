import { describe, expect, it } from "vitest";
import { parseHex } from "./contrast";

// Raw stylesheet text through Vite (see contrast.test.ts; vitest.config lets these CSS files through).
const files = import.meta.glob(["./tokens.css", "./theme-bridge.css"], {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const legacy = files["./tokens.css"] ?? "";
const bridge = files["./theme-bridge.css"] ?? "";

function lightBlock(css: string): string {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const blocks = [...clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  return blocks
    .filter((m) => m[1]!.trim() === '[data-theme="light"]')
    .map((m) => m[2])
    .join("\n");
}

describe("light theme parity (H-25)", () => {
  it("loads both stylesheets", () => {
    expect(legacy).toContain("--wave-bg");
    expect(bridge).toContain("--pv-bg-panel");
  });

  it("every audio content token in tokens.css has a light value in theme-bridge.css", () => {
    const content = new Set(
      [...legacy.matchAll(/(--(?:wave|waveform|spec|analyzer|eq)-[\w-]+)\s*:/g)].map((m) => m[1]!),
    );
    expect(content.size).toBeGreaterThan(20);
    const light = lightBlock(bridge);
    const missing = [...content].filter((name) => !new RegExp(`${name}\\s*:`).test(light));
    expect(missing).toEqual([]);
  });

  it("light content values use the shapes the renderers parse (#rrggbb or rgba())", () => {
    for (const m of lightBlock(bridge).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
      const value = m[2]!.trim();
      const ok = parseHex(value) !== null || /^rgba\(\s*\d+,\s*\d+,\s*\d+,\s*[\d.]+\s*\)$/.test(value);
      expect(ok, `${m[1]}: ${value}`).toBe(true);
    }
  });

  it("the recording colour is a token in both themes (no hard-coded fallback)", () => {
    expect(bridge).toMatch(/--wave-record:\s*var\(--pv-record\)/);
    expect(bridge).toMatch(/--wave-record-head:\s*var\(--pv-record\)/);
  });
});
