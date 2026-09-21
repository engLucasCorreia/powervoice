import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { SpectrumReportDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import { docDto } from "../test/fixtures";
import { resetAnalyzerForTest } from "./analyzer.svelte";
import { resetOutputDeviceStatusForTest } from "./outputDeviceStatus.svelte";
import { applySpectrumJobProgress, applySpectrumReport } from "./diagnostics.svelte";
import { explainModalState, resetExplainModalForTest } from "./explain/explainModal.svelte";
import { explainVoiceState } from "./explain/explainVoice.svelte";
import { balancedReport } from "./explain/voiceFixtures";
import AnalyzerPanel from "./AnalyzerPanel.svelte";

/**
 * H-24 item 5 (SPEC-007 §2.9): the analyzer used to draw grid lines with no frequency/dB labels
 * at all. These tests cover the persistent DOM-based axis labels and the ticket's regression for
 * item 4 ("every canvas sits in a container with a definite size ... never sized from its own
 * content"): the plot area's measured size must not change just because the canvas redraws.
 */

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

function unstubSize(): void {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

afterEach(() => {
  clearMocks();
  resetAnalyzerForTest();
  resetOutputDeviceStatusForTest();
  unstubSize();
});

describe("AnalyzerPanel axes (H-24 item 5)", () => {
  it("renders a dB gutter with its unit shown once, and a frequency axis strip", async () => {
    mockIPC(() => null);
    stubSize(400, 120);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const dbAxis = target.querySelector('[data-testid="analyzer-db-axis"]')!;
    expect(dbAxis).not.toBeNull();
    expect(dbAxis.textContent).toContain("dBFS");
    // The unit appears exactly once even though there are several dB ticks.
    expect(dbAxis.textContent?.match(/dBFS/g)?.length).toBe(1);

    const freqAxis = target.querySelector('[data-testid="analyzer-freq-axis"]')!;
    expect(freqAxis).not.toBeNull();
    expect(freqAxis.querySelectorAll(".tick").length).toBeGreaterThan(0);

    unmount(app);
    target.remove();
  });

  it("the canvas backing store tracks the container's size and never grows on its own across redraws (item 4 regression)", async () => {
    // The bug this guards against: sizing the canvas from its own rendered content instead of
    // its container, which grows the container next frame (measured here), which grows the
    // canvas again, etc. `stubSize` pins the container's `clientWidth`/`clientHeight` (jsdom has
    // no real layout engine, MEMORY.md) — the canvas's backing store (`canvasEl.width/height`,
    // a real, readable attribute even in jsdom) must stay locked to that pinned size no matter
    // how many animation frames redraw it.
    mockIPC(() => null);
    stubSize(400, 150);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const canvas = target.querySelector('[data-testid="analyzer-panel"] canvas') as HTMLCanvasElement;
    const expectedW = canvas.width;
    const expectedH = canvas.height;
    expect(expectedW).toBeGreaterThan(0);
    expect(expectedH).toBeGreaterThan(0);

    // Peak-hold ballistics redraw every animation frame (a real timer under jsdom, MEMORY.md) —
    // several frames must never change the backing store size.
    await new Promise((resolve) => setTimeout(resolve, 80));
    flushSync();
    expect(canvas.width).toBe(expectedW);
    expect(canvas.height).toBe(expectedH);

    unmount(app);
    target.remove();
  });
});

// --- H-42: modes, toggles, diagnostics panel (SPEC-007 §8) --------------------------------------

import { diagnosticsState, resetDiagnosticsForTest } from "./diagnostics.svelte";

describe("AnalyzerPanel diagnostics (H-42)", () => {
  afterEach(() => {
    resetDiagnosticsForTest();
    document.body.innerHTML = "";
  });

  function recordCalls(): Array<[string, Record<string, unknown>]> {
    const calls: Array<[string, Record<string, unknown>]> = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
      return cmd === "analyzer_voice_subscribe" ? 77 : null;
    });
    return calls;
  }

  function segment(target: HTMLElement, testid: string, label: string): HTMLElement {
    return [...target.querySelectorAll<HTMLElement>(`[data-testid="${testid}"] [role="radio"], [data-testid="${testid}"] button`)].find(
      (el) => el.textContent?.trim() === label,
    )!;
  }

  it("keeps the live look by default: peaks on, diagnostics hidden, no extra bars", async () => {
    recordCalls();
    stubSize(600, 150);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    expect(target.querySelector('[data-testid="analyzer-peaks-toggle"]')!.getAttribute("aria-pressed")).toBe("true");
    expect(target.querySelector('[data-testid="analyzer-diagnostics-toggle"]')!.getAttribute("aria-pressed")).toBe("false");
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).toBeNull();
    expect(target.querySelector('[data-testid="analyzer-average-bar"]')).toBeNull();
    expect(target.querySelector('[data-testid="analyzer-compare-bar"]')).toBeNull();
    unmount(app);
  });

  it("the Diagnostics toggle shows the panel and holds the live voice stream only while shown", async () => {
    const calls = recordCalls();
    stubSize(900, 180);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-diagnostics-toggle"]')!.click();
    await settle();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).not.toBeNull();
    expect(calls.filter(([c]) => c === "analyzer_voice_subscribe").length).toBe(1);
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-diagnostics-close"]')!.click();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-diagnostics"]')).toBeNull();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id)).toContain(77);
    unmount(app);
  });

  it("Average and Compare modes show their bars; Analyze needs something to analyze", async () => {
    recordCalls();
    stubSize(900, 180);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    segment(target, "analyzer-mode", "Average").click();
    await settle();
    expect(diagnosticsState().mode).toBe("average");
    expect(target.querySelector('[data-testid="analyzer-average-bar"]')).not.toBeNull();
    expect(target.querySelector<HTMLButtonElement>('[data-testid="analyzer-average-analyze"]')!.disabled).toBe(true);
    expect(target.querySelector('[data-testid="analyzer-plot"]')!.textContent).toContain("Open a file");
    segment(target, "analyzer-mode", "Compare").click();
    await settle();
    expect(target.querySelector('[data-testid="analyzer-compare-bar"]')).not.toBeNull();
    // No live frame yet: nothing to freeze.
    expect(target.querySelector<HTMLButtonElement>('[data-testid="analyzer-freeze-a"]')!.disabled).toBe(true);
    unmount(app);
  });
});

// --- H-92: the "Explain My Voice" button freezes the *Average* FFT-bin curve ---------------------

/** A `VXLT` frame carrying `levels`, for one result of a `spectrum_report`. */
function vxlt(jobId: number, index: number, levels: number[]): ArrayBuffer {
  const buf = new ArrayBuffer(36 + 4 * levels.length);
  const dv = new DataView(buf);
  "VXLT".split("").forEach((c, i) => dv.setUint8(i, c.charCodeAt(0)));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 36, true);
  dv.setUint32(8, jobId, true);
  dv.setUint32(12, index, true);
  dv.setUint32(16, 48_000, true);
  dv.setUint32(20, 4, true);
  dv.setUint32(24, 0, true);
  dv.setUint32(28, levels.length, true);
  dv.setUint32(32, 0, true);
  levels.forEach((v, k) => dv.setFloat32(36 + 4 * k, v, true));
  return buf;
}

function averageReport(jobId: number): SpectrumReportDto {
  return {
    job_id: jobId,
    sample_rate_hz: 48_000,
    fft_size: 4,
    window: "hann",
    start_sample: 0,
    end_sample: 48_000,
    results: [{ source: "processed", frames: 10, has_noise: false, report: balancedReport() }],
  };
}

describe("Explain My Voice (H-92)", () => {
  afterEach(() => {
    resetDocumentStateForTest();
    resetWaveformViewForTest();
    resetSelectionForTest();
    resetExplainModalForTest();
    // H-108: a couple of these tests start a job and never run it to "done"/"failed"/"cancelled"
    // — without this, the leftover `running` job in the shared diagnostics store makes the very
    // next `canAnalyzeAverage()` call (any later describe block, not just this one) refuse a
    // fresh Explain/Analyze click for no visible reason.
    resetDiagnosticsForTest();
    document.body.innerHTML = "";
  });

  it("stays disabled with no document open", async () => {
    stubSize(900, 200);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();
    expect(target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-open"]')!.disabled).toBe(true);
    unmount(app);
  });

  it("clicking it starts a long-term Average job itself — it never freezes the live bands — and opens once that job completes", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    clearMocks();

    stubSize(900, 200);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-open"]')!;
    // No Average result exists yet, but a document is open: the button is enabled and, unlike
    // before H-96 was reused here, does its own analysis rather than requiring the Average tab.
    expect(button.disabled).toBe(false);

    const started: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        started.push((args as { request: unknown }).request);
        return { job_id: 42 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        return vxlt(a.jobId, a.index, [-10, -20, -30]);
      }
      return null;
    });
    button.click();
    await settle();

    expect(started).toHaveLength(1);
    expect(explainModalState().open).toBe(false); // still analyzing — no selection, so whole file
    expect(target.querySelector('[data-testid="analyzer-explain-open"]')?.textContent).toContain("Analyzing");

    applySpectrumJobProgress({ job_id: 42, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport(averageReport(42));
    await settle();

    expect(explainModalState().open).toBe(true);
    const snapshot = explainVoiceState().snapshot!;
    expect(snapshot.origin).toBe("average");
    expect(snapshot.resolution).toBe("bins");
    // The frozen curve is the Average result's FFT bins — never the live 1/24-octave bands.
    expect(Array.from(snapshot.rawDb)).toEqual([-10, -20, -30]);

    unmount(app);
  });

  it("analyzes the selection when one exists, not the whole file", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    clearMocks();
    setSelectionFromResult([1000, 5000]);

    stubSize(900, 200);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const requests: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        requests.push((args as { request: unknown }).request);
        return { job_id: 7 };
      }
      return null;
    });
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-open"]')!.click();
    await settle();
    expect(requests[0]).toMatchObject({ start_sample: 1000, end_sample: 5000 });

    unmount(app);
  });
});

// --- H-108: "Analyzing…" always ends somewhere visible, with real progress and a Cancel --------

describe("Explain My Voice: progress, cancel and failure (H-108)", () => {
  // H-92's own tests (above) leave a `running` job in the shared diagnostics store when they
  // don't run it to completion — reset before each test here too, not just after, so that leak
  // never makes `canAnalyzeAverage()` refuse a fresh Explain click.
  beforeEach(() => {
    resetDiagnosticsForTest();
  });
  afterEach(() => {
    resetDocumentStateForTest();
    resetWaveformViewForTest();
    resetSelectionForTest();
    resetExplainModalForTest();
    resetDiagnosticsForTest();
    clearNotices();
    document.body.innerHTML = "";
  });

  it("shows the job's own real progress (not a bare spinner) and a Cancel that stops it", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    clearMocks();

    stubSize(900, 200);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    const cancelled: number[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        return { job_id: 91 };
      }
      if (cmd === "spectrum_analyze_cancel") {
        cancelled.push((args as { jobId: number }).jobId);
        return null;
      }
      return null;
    });
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-open"]')!.click();
    await settle();

    applySpectrumJobProgress({ job_id: 91, kind: "spectrum_analyze", state: "running", fraction: 0.42 });
    await settle();
    const button = target.querySelector('[data-testid="analyzer-explain-open"]')!;
    expect(button.textContent).toContain("42");

    const cancelButton = target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-cancel"]');
    expect(cancelButton).not.toBeNull();
    cancelButton!.click();
    await settle();
    expect(cancelled).toEqual([91]);

    applySpectrumJobProgress({ job_id: 91, kind: "spectrum_analyze", state: "cancelled", fraction: 0 });
    await settle();
    // Cancelled is one of the three states the user can see: the button goes back to its resting
    // label and the Cancel affordance disappears — never stuck on "Analyzing…" forever.
    expect(target.querySelector('[data-testid="analyzer-explain-open"]')!.textContent).toContain("Explain My Voice");
    expect(target.querySelector('[data-testid="analyzer-explain-cancel"]')).toBeNull();
    expect(explainModalState().open).toBe(false);

    unmount(app);
  });

  it("a done report that produces no curve for the requested source fails cleanly instead of hanging forever", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    clearMocks();

    stubSize(900, 200);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AnalyzerPanel, { target });
    await settle();

    mockIPC((cmd) => {
      if (cmd === "spectrum_analyze_start") {
        return { job_id: 92 };
      }
      if (cmd === "spectrum_analyze_curve") {
        // The one real-world way `diag.averages` can end up with nothing for the requested
        // source once the job is "done": the curve fetch itself fails (H-108's second
        // hypothesis, ruled out on the current backend by a Rust-level regression test, but the
        // UI must still not hang forever if it ever does happen).
        throw { code: "invalid_argument", key: "error.spectrum.no_result", params: {} };
      }
      return null;
    });
    target.querySelector<HTMLButtonElement>('[data-testid="analyzer-explain-open"]')!.click();
    await settle();

    applySpectrumJobProgress({ job_id: 92, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport({
      job_id: 92,
      sample_rate_hz: 48_000,
      fft_size: 4,
      window: "hann",
      start_sample: 0,
      end_sample: 48_000,
      results: [{ source: "processed", frames: 10, has_noise: false, report: balancedReport() }],
    } satisfies SpectrumReportDto);
    await settle();

    expect(explainModalState().open).toBe(false);
    expect(target.querySelector('[data-testid="analyzer-explain-open"]')!.textContent).toContain("Explain My Voice");
    expect(noticesState().toasts.some((n) => n.key === "error.spectrum.no_result")).toBe(true);

    unmount(app);
  });
});
