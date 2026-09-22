import { describe, expect, it } from "vitest";
import { buildExplainExportHtml } from "./explainExportHtml";
import type { ExplainExportData } from "./explainExport";

/**
 * H-115 ticket §4: "a self-contained report ... with the graph, every finding's measured value
 * and interpretation, the take's duration and the date." Pure string building — no DOM needed.
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
    profile: [{ id: "f0", label: "Pitch", value: "120 Hz", reading: "Measured", severity: null }],
    focus: [{ id: "noise_floor", text: "No action suggested: nothing measured here justifies one.", action: null }],
    findings: [
      {
        id: "body",
        category: "tone",
        severity: "attention",
        title: "Low-mid body (200–500 Hz)",
        measured: "+9.2 dB relative to the 1 kHz octave.",
        interpretation: "Low-mid energy sits 0.2 dB past the threshold — barely across it.",
        recommendation: "A wide cut of about 2 dB around 350 Hz is the conservative move.",
        action: null,
        nearThreshold: true,
      },
    ],
    showAnnotations: true,
    showEqAdvice: true,
    graph: {
      canvas: document.createElement("canvas"),
      widthPx: 800,
      heightPx: 320,
      plot: { x: 48, y: 10, width: 742, height: 290 },
      cards: [],
      leaders: [],
    },
    beneath: [],
    ...overrides,
  };
}

describe("buildExplainExportHtml", () => {
  it("is a complete, self-contained HTML document (no external references)", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toMatch(/^<!DOCTYPE html>/);
    expect(html).toContain("<html");
    expect(html).not.toMatch(/https?:\/\//);
    expect(html).not.toContain("<link ");
    expect(html).not.toContain("<script ");
  });

  it("embeds the image inline as the given data URI", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toContain('src="data:image/png;base64,AAAA"');
  });

  it("carries the take's duration and the analysis date", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toContain("Duration: 8.4 s");
    expect(html).toContain("21 Sep 2026, 12:00");
  });

  it("carries every finding's measured value and interpretation, including its recommendation", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toContain("+9.2 dB relative to the 1 kHz octave.");
    expect(html).toContain("barely across it.");
    expect(html).toContain("A wide cut of about 2 dB around 350 Hz");
  });

  it("carries the headline and basis", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toContain("One measurement crossed a documented threshold.");
    expect(html).toContain("Measured over 8.4 s of non-silent audio.");
  });

  it("HTML-escapes measured/interpretation text so a stray '<' or '&' can never break the page", () => {
    const html = buildExplainExportHtml(
      data({
        findings: [
          {
            id: "body",
            category: "tone",
            severity: "good",
            title: "A & B <test>",
            measured: "5 < 10 dB",
            interpretation: "ok",
            recommendation: null,
            action: null,
            nearThreshold: false,
          },
        ],
      }),
      "data:image/png;base64,AAAA",
    );
    expect(html).toContain("A &amp; B &lt;test&gt;");
    expect(html).toContain("5 &lt; 10 dB");
    expect(html).not.toContain("<test>");
  });

  it("has print-friendly CSS that avoids splitting a row or the image across a page break", () => {
    const html = buildExplainExportHtml(data(), "data:image/png;base64,AAAA");
    expect(html).toContain("@media print");
    expect(html).toContain("break-inside: avoid");
  });
});
