/**
 * H-115's exported report: a single, self-contained HTML file (no dependency named in
 * PROMPT.md/an ADR generates PDF anywhere in this repo — see the ticket report for why HTML was
 * chosen over adding one) — the exported image embedded inline as a `data:` URI, plus a table
 * with every finding's measured value and interpretation (not only the ones that had room on the
 * graph), the take's duration and the date it was analysed. Opens in any browser with no network
 * access, and prints legibly (`@media print` keeps a finding's row from splitting across a page
 * break).
 *
 * Pure string building — no DOM, so this is fully unit-tested without a canvas.
 */
import type { FindingProse } from "./prose";
import type { ExplainExportData } from "./explainExport";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

const SEVERITY_LABEL: Record<FindingProse["severity"], string> = {
  good: "Good",
  info: "Info",
  attention: "Attention",
  significant: "Significant",
};

const SEVERITY_COLOR: Record<FindingProse["severity"], string> = {
  good: "#157a3b",
  info: "#1a64cc",
  attention: "#8a5a00",
  significant: "#c4262d",
};

function findingRow(prose: FindingProse): string {
  const color = SEVERITY_COLOR[prose.severity];
  return `<tr>
    <td class="finding">${escapeHtml(prose.title)}</td>
    <td class="severity"><span class="dot" style="background:${color}"></span>${SEVERITY_LABEL[prose.severity]}</td>
    <td class="measured">${escapeHtml(prose.measured)}</td>
    <td class="interpretation">${escapeHtml(prose.interpretation)}${
      prose.recommendation ? `<div class="recommendation">${escapeHtml(prose.recommendation)}</div>` : ""
    }</td>
  </tr>`;
}

/**
 * Builds the whole report document. `imagePngDataUrl` is a `data:image/png;base64,...` URI (the
 * same PNG {@link renderExplainExportPng} produces for the image export) so the file needs
 * nothing else alongside it to be complete.
 */
export function buildExplainExportHtml(data: ExplainExportData, imagePngDataUrl: string): string {
  const focusItems = data.focus.map((item) => `<li>${escapeHtml(item.text)}</li>`).join("\n");
  const profileRows = data.profile
    .map(
      (row) =>
        `<tr><td>${escapeHtml(row.label)}</td><td>${escapeHtml(row.value || "—")}</td><td>${escapeHtml(row.reading)}</td></tr>`,
    )
    .join("\n");
  const findingRows = data.findings.map(findingRow).join("\n");
  const title = escapeHtml(data.title);

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title}</title>
<style>
  :root { color-scheme: light; }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    padding: 32px;
    background: #ffffff;
    color: #1a1d22;
    font-family: system-ui, -apple-system, "Segoe UI", Cantarell, "Noto Sans", Ubuntu, Roboto, "Helvetica Neue", Arial, sans-serif;
    line-height: 1.45;
  }
  .page { max-width: 960px; margin: 0 auto; }
  h1 { font-size: 22px; margin: 0 0 4px; }
  .meta { color: #5f6671; font-size: 13px; margin: 0 0 24px; }
  h2 { font-size: 14px; text-transform: uppercase; letter-spacing: 0.02em; color: #525964; margin: 28px 0 8px; }
  .headline { font-size: 16px; font-weight: 600; margin: 0 0 4px; }
  .basis { color: #5f6671; font-size: 13px; margin: 0; }
  img.graph { display: block; width: 100%; height: auto; border: 1px solid #e6e8ec; border-radius: 6px; margin-top: 8px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid #e6e8ec; vertical-align: top; }
  th { color: #525964; font-weight: 600; font-size: 12px; text-transform: uppercase; letter-spacing: 0.02em; }
  .profile-table td:nth-child(2) { font-variant-numeric: tabular-nums; white-space: nowrap; }
  .findings-table .finding { font-weight: 600; white-space: nowrap; }
  .findings-table .severity { white-space: nowrap; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }
  .recommendation { margin-top: 6px; padding-top: 6px; border-top: 1px dashed #e6e8ec; color: #1a1d22; }
  ul.focus { margin: 0; padding-left: 20px; }
  ul.focus li { margin-bottom: 4px; }
  footer { margin-top: 32px; color: #8a909b; font-size: 11px; }
  @media print {
    body { padding: 0; }
    tr { break-inside: avoid; }
    img.graph { break-inside: avoid; }
  }
</style>
</head>
<body>
<div class="page">
  <h1>${title}</h1>
  <p class="meta">${escapeHtml(data.durationLabel)} · ${escapeHtml(data.subtitle)} · analysed ${escapeHtml(data.dateLabel)}</p>

  <p class="headline">${escapeHtml(data.headline)}</p>
  <p class="basis">${escapeHtml(data.basis)}</p>

  <h2>Voice profile</h2>
  <table class="profile-table">
    <thead><tr><th>Measurement</th><th>Value</th><th>Reading</th></tr></thead>
    <tbody>
${profileRows}
    </tbody>
  </table>

  <h2>Suggested focus</h2>
  <ul class="focus">
${focusItems}
  </ul>

  <h2>Annotated spectrum</h2>
  <img class="graph" src="${imagePngDataUrl}" alt="${title}">

  <h2>Every measurement</h2>
  <table class="findings-table">
    <thead><tr><th>Finding</th><th>Severity</th><th>Measured</th><th>Interpretation</th></tr></thead>
    <tbody>
${findingRows}
    </tbody>
  </table>

  <footer>Exported from PowerVoice — Voice Spectrum Analysis.</footer>
</div>
</body>
</html>
`;
}
