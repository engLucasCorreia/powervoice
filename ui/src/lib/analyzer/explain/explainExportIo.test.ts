import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ExplainExportData } from "./explainExport";

/**
 * H-115 security amendment: the native save dialog now opens on the **backend**
 * (`explain_export_pick_path`) — this module never supplies a path, only UI hints, and gets back
 * a single-use token it must present (in a header) to `explain_export_write_bytes` along with the
 * raw bytes as the whole request body. `renderExplainExportPng` is stubbed out (it needs a real
 * 2D canvas context, which jsdom doesn't have — MEMORY.md); its own composition logic has its own
 * tests (`explainExportImage.test.ts`) that don't need a real canvas either.
 *
 * `@tauri-apps/api/mocks`'s `mockIPC` callback only ever receives `(cmd, args)` — it silently
 * drops the third `invoke()` argument (`options`, where headers travel), so asserting the token
 * actually reaches `explain_export_write_bytes` as a header needs a thin spy wrapped around
 * `window.__TAURI_INTERNALS__.invoke` *after* `mockIPC` has installed its own, forwarding to it.
 */
vi.mock("./explainExportImage", () => ({
  renderExplainExportPng: vi.fn().mockResolvedValue(new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" })),
}));

const { exportExplainImage, exportExplainReport } = await import("./explainExportIo");

function data(): ExplainExportData {
  return {
    title: "Voice Spectrum Analysis",
    subtitle: "8.4 s of audio analysed",
    durationLabel: "Duration: 8.4 s",
    takenAtMs: 1_726_000_000_000,
    dateLabel: "21 Sep 2026, 12:00",
    headline: "clean",
    basis: "",
    profile: [],
    focus: [],
    findings: [],
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
  };
}

interface Invocation {
  cmd: string;
  args: unknown;
  headers: Record<string, string> | undefined;
}

type InvokeFn = (cmd: string, args?: unknown, options?: { headers?: HeadersInit }) => unknown;

/** Same `window as unknown as {...}` cast `test/liveFrame.ts` uses to reach `mockIPC`'s installed
 * internals — there is no ambient type for `window.__TAURI_INTERNALS__`. */
function internals(): { invoke: InvokeFn } {
  return (window as unknown as { __TAURI_INTERNALS__: { invoke: InvokeFn } }).__TAURI_INTERNALS__;
}

/** Installs `mockIPC`, then wraps the `invoke` it installs so every call (including its
 * `options.headers`, which `mockIPC`'s own callback signature discards) is recorded. */
function mockIpcWithHeaders(handler: (cmd: string, args: unknown) => unknown): Invocation[] {
  mockIPC(handler);
  const calls: Invocation[] = [];
  const target = internals();
  const wrapped = target.invoke;
  target.invoke = (cmd, args, options) => {
    calls.push({ cmd, args, headers: options?.headers as Record<string, string> | undefined });
    return wrapped(cmd, args, options);
  };
  return calls;
}

afterEach(() => {
  clearMocks();
});

describe("exportExplainImage", () => {
  it("picks the path on the backend (no path argument), then writes the raw bytes with the returned token in a header", async () => {
    const calls = mockIpcWithHeaders((cmd) => {
      if (cmd === "explain_export_pick_path") {
        return "explain-token-123";
      }
      if (cmd === "explain_export_write_bytes") {
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const saved = await exportExplainImage(data());

    expect(saved).toBe(true);
    const pick = calls.find((c) => c.cmd === "explain_export_pick_path")!;
    expect(pick.args).toEqual({
      suggestedFileName: "voice-spectrum-analysis-2024-09-10.png",
      filterName: "PNG",
      filterExtensions: ["png"],
    });
    // No `path` field anywhere in what the frontend sends — the whole point of the amendment.
    expect(JSON.stringify(pick.args)).not.toContain("path");

    const write = calls.find((c) => c.cmd === "explain_export_write_bytes")!;
    expect(write.headers).toEqual({ "X-Explain-Export-Token": "explain-token-123" });
    expect(write.args).toBeInstanceOf(Uint8Array);
    expect(Array.from(write.args as Uint8Array)).toEqual([1, 2, 3]);
  });

  it("writes nothing and resolves false when the save dialog is cancelled (no token)", async () => {
    let writeCalled = false;
    mockIPC((cmd) => {
      if (cmd === "explain_export_pick_path") {
        return null;
      }
      if (cmd === "explain_export_write_bytes") {
        writeCalled = true;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const saved = await exportExplainImage(data());

    expect(saved).toBe(false);
    expect(writeCalled).toBe(false);
  });
});

describe("exportExplainReport", () => {
  it("writes a self-contained HTML document (as raw bytes) embedding the rendered PNG, with the token in a header", async () => {
    const calls = mockIpcWithHeaders((cmd) => {
      if (cmd === "explain_export_pick_path") {
        return "explain-token-456";
      }
      if (cmd === "explain_export_write_bytes") {
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const saved = await exportExplainReport(data());

    expect(saved).toBe(true);
    const pick = calls.find((c) => c.cmd === "explain_export_pick_path")!;
    expect(pick.args).toEqual({
      suggestedFileName: "voice-spectrum-analysis-2024-09-10.html",
      filterName: "HTML",
      filterExtensions: ["html"],
    });

    const write = calls.find((c) => c.cmd === "explain_export_write_bytes")!;
    expect(write.headers).toEqual({ "X-Explain-Export-Token": "explain-token-456" });
    // Not `toBeInstanceOf(Uint8Array)`: `TextEncoder.encode()` under vitest/jsdom can return a
    // typed array from a different realm than this test file's own `Uint8Array` global, which
    // fails an identity check despite being a perfectly good Uint8Array — `ArrayBuffer.isView`
    // is realm-agnostic.
    expect(ArrayBuffer.isView(write.args)).toBe(true);
    const html = new TextDecoder().decode(write.args as Uint8Array);
    expect(html).toContain("<!DOCTYPE html>");
    expect(html).toContain("data:image/png;base64,");
  });

  it("resolves false without writing when the save dialog is cancelled", async () => {
    let writeCalled = false;
    mockIPC((cmd) => {
      if (cmd === "explain_export_pick_path") {
        return null;
      }
      if (cmd === "explain_export_write_bytes") {
        writeCalled = true;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const saved = await exportExplainReport(data());

    expect(saved).toBe(false);
    expect(writeCalled).toBe(false);
  });
});
