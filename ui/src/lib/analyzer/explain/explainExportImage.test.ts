import { describe, expect, it } from "vitest";
import {
  drawExplainExportImage,
  measureExplainExportLayout,
  resolveExportPalette,
  type DrawCtx2D,
} from "./explainExportImage";
import type { ExplainExportData } from "./explainExport";

// A real palette (dark theme's `design-tokens.css` values, via `themeTokenValue`'s fallback path
// — jsdom has no stylesheet loaded) rather than a literal colour written down in this test file
// (T-708 covers tests too in spirit, even though `colorLiterals.test.ts` only lints non-test
// sources).
const PALETTE = resolveExportPalette();

/**
 * H-115: jsdom has no real 2D canvas context (MEMORY.md — the same limitation `ExplainGraph`'s
 * own `draw()` is untested against), so this exercises the composition logic two ways instead:
 * `measureExplainExportLayout` is plain geometry (no canvas at all); `drawExplainExportImage`
 * is driven with a `DrawCtx2D` test double that records every call, since that interface is a
 * structural subset of `CanvasRenderingContext2D` a real one also satisfies.
 */

function data(overrides: Partial<ExplainExportData> = {}): ExplainExportData {
  return {
    title: "Voice Spectrum Analysis",
    subtitle: "8.4 s of audio analysed",
    durationLabel: "Duration: 8.4 s",
    takenAtMs: 1_726_000_000_000,
    dateLabel: "21 Sep 2026, 12:00",
    headline: "One measurement crossed a documented threshold.",
    basis: "Measured over 8.4 s of non-silent audio.",
    profile: [
      { id: "f0", label: "Pitch", value: "120 Hz", reading: "Measured", severity: null },
      { id: "body", label: "Body", value: "+2.5 dB", reading: "In the usual range", severity: "good" },
    ],
    focus: [{ id: "noise_floor", text: "No action suggested: nothing measured here justifies one.", action: null }],
    findings: [
      {
        id: "body",
        category: "tone",
        severity: "good",
        title: "Low-mid body (200–500 Hz)",
        measured: "+2.5 dB relative to the 1 kHz octave.",
        interpretation: "Inside the range a voice usually occupies here.",
        recommendation: null,
        action: null,
        nearThreshold: false,
      },
    ],
    showAnnotations: true,
    showEqAdvice: true,
    graph: {
      canvas: document.createElement("canvas"),
      widthPx: 800,
      heightPx: 320,
      plot: { x: 48, y: 10, width: 742, height: 290 },
      cards: [
        {
          rect: { x: 100, y: 40, width: 180, height: 48 },
          title: "Pitch",
          measured: "Median 120 Hz over the voiced frames.",
          severity: "info",
        },
      ],
      leaders: [{ from: { x: 120, y: 60 }, to: { x: 130, y: 90 } }],
    },
    beneath: [{ title: "Signal to noise", measured: "48 dB above the room tone." }],
    ...overrides,
  };
}

describe("measureExplainExportLayout", () => {
  it("widens the page to a minimum even when the graph itself is narrower", () => {
    const plan = measureExplainExportLayout(data({ graph: { ...data().graph, widthPx: 400, heightPx: 160 } }));
    expect(plan.widthPx).toBe(860);
  });

  it("keeps the page at the graph's own width when it is already wide enough", () => {
    const plan = measureExplainExportLayout(data({ graph: { ...data().graph, widthPx: 1200, heightPx: 480 } }));
    expect(plan.widthPx).toBe(1200);
  });

  it("includes the title and headline as text runs", () => {
    const plan = measureExplainExportLayout(data());
    const texts = plan.texts.map((t) => t.text);
    expect(texts).toContain("Voice Spectrum Analysis");
    expect(texts.some((t) => t.includes("One measurement crossed"))).toBe(true);
  });

  it("places one card per annotation when showAnnotations is on", () => {
    const plan = measureExplainExportLayout(data({ showAnnotations: true }));
    expect(plan.cards).toHaveLength(1);
    expect(plan.leaders).toHaveLength(1);
  });

  it("omits cards and leaders entirely when showAnnotations is off — the same choice the modal shows", () => {
    const plan = measureExplainExportLayout(data({ showAnnotations: false }));
    expect(plan.cards).toHaveLength(0);
    expect(plan.leaders).toHaveLength(0);
  });

  it("grows the page height to fit the 'also measured' cards when there are any", () => {
    const withBeneath = measureExplainExportLayout(data());
    const withoutBeneath = measureExplainExportLayout(data({ beneath: [] }));
    expect(withBeneath.heightPx).toBeGreaterThan(withoutBeneath.heightPx);
  });

  it("scales the graph image and its cards together when the page is wider than the source graph", () => {
    const narrow = data({ graph: { ...data().graph, widthPx: 400, heightPx: 160 } });
    const plan = measureExplainExportLayout(narrow);
    const scale = plan.widthPx / 400;
    expect(plan.graphImage.width).toBeCloseTo(plan.widthPx, 5);
    expect(plan.graphImage.height).toBeCloseTo(160 * scale, 5);
  });
});

function fakeCtx(): { ctx: DrawCtx2D; fillTexts: string[]; drawImageCalls: number; fillRects: number[][] } {
  const fillTexts: string[] = [];
  const fillRects: number[][] = [];
  let drawImageCalls = 0;
  const ctx: DrawCtx2D = {
    save() {},
    restore() {},
    scale() {},
    fillRect(x, y, w, h) {
      fillRects.push([x, y, w, h]);
    },
    fillText(text) {
      fillTexts.push(text);
    },
    beginPath() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    drawImage() {
      drawImageCalls += 1;
    },
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 0,
    font: "",
    textAlign: "left",
    textBaseline: "alphabetic",
  };
  return {
    ctx,
    fillTexts,
    get drawImageCalls() {
      return drawImageCalls;
    },
    fillRects,
  };
}

describe("drawExplainExportImage", () => {
  it("draws the graph canvas exactly once", () => {
    const plan = measureExplainExportLayout(data());
    const recorded = fakeCtx();
    drawExplainExportImage(recorded.ctx, plan, data().graph.canvas, PALETTE, 1);
    expect(recorded.drawImageCalls).toBe(1);
  });

  it("renders the title and every finding-card line as text", () => {
    const plan = measureExplainExportLayout(data());
    const recorded = fakeCtx();
    drawExplainExportImage(recorded.ctx, plan, data().graph.canvas, PALETTE, 1);
    expect(recorded.fillTexts).toContain("Voice Spectrum Analysis");
    expect(recorded.fillTexts).toContain("Pitch");
    expect(recorded.fillTexts.some((t) => t.includes("Median 120 Hz"))).toBe(true);
  });

  it("does not throw and draws no card text when there are no cards (annotations off)", () => {
    const plan = measureExplainExportLayout(data({ showAnnotations: false }));
    const recorded = fakeCtx();
    expect(() =>
      drawExplainExportImage(recorded.ctx, plan, data().graph.canvas, PALETTE, 1),
    ).not.toThrow();
    expect(recorded.fillTexts).not.toContain("Pitch");
  });

  it("fills the background at the full scaled canvas size", () => {
    const plan = measureExplainExportLayout(data());
    const recorded = fakeCtx();
    drawExplainExportImage(recorded.ctx, plan, data().graph.canvas, PALETTE, 2);
    expect(recorded.fillRects[0]).toEqual([0, 0, plan.widthPx * 2, plan.heightPx * 2]);
  });
});
