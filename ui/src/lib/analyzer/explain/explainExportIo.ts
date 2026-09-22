/**
 * The I/O half of H-115's export: opening the native save dialog and writing the bytes.
 *
 * H-115 security amendment: the dialog now opens on the **backend**
 * (`explain_export_pick_path`) — this module never sees, and never supplies, a filesystem path.
 * It gets back a single-use token bound to whatever path the user picked, then hands the raw
 * bytes to `explain_export_write_bytes` with that token in a header (binary transfer — see
 * `commands.ts`'s doc comments and `explain_export_commands.rs`'s module doc for the measurement
 * that justified it). `explainExport.ts`/`explainExportImage.ts`/`explainExportHtml.ts` stay
 * pure; this is the only module that touches `invoke`, so it is the only one a test needs
 * `mockIPC` for.
 */
import { explainExportPickPath, explainExportWriteBytes } from "../../ipc/commands";
import type { ExplainExportData } from "./explainExport";
import { explainExportFileBase } from "./explainExport";
import { buildExplainExportHtml } from "./explainExportHtml";
import { renderExplainExportPng } from "./explainExportImage";

function blobToBytes(blob: Blob): Promise<Uint8Array> {
  return blob.arrayBuffer().then((buffer) => new Uint8Array(buffer));
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("explain export: FileReader failed"));
    reader.readAsDataURL(blob);
  });
}

/** `true` once a file was actually written, `false` when the user cancelled the save dialog —
 * callers use this to decide whether a "saved" notice is warranted. */
export async function exportExplainImage(data: ExplainExportData): Promise<boolean> {
  const png = await renderExplainExportPng(data);
  const token = await explainExportPickPath({
    suggestedFileName: `${explainExportFileBase(data)}.png`,
    filterName: "PNG",
    filterExtensions: ["png"],
  });
  if (token === null) {
    return false;
  }
  await explainExportWriteBytes(token, await blobToBytes(png));
  return true;
}

/** Renders the same PNG the image export produces, embeds it in the self-contained HTML report
 * (`explainExportHtml.ts`), and writes that. */
export async function exportExplainReport(data: ExplainExportData): Promise<boolean> {
  const png = await renderExplainExportPng(data);
  const imageDataUrl = await blobToDataUrl(png);
  const html = buildExplainExportHtml(data, imageDataUrl);
  const token = await explainExportPickPath({
    suggestedFileName: `${explainExportFileBase(data)}.html`,
    filterName: "HTML",
    filterExtensions: ["html"],
  });
  if (token === null) {
    return false;
  }
  await explainExportWriteBytes(token, new TextEncoder().encode(html));
  return true;
}
