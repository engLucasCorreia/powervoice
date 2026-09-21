import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DocumentDto, SpectrumReportDto, VoiceReportDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { docDto as doc } from "../test/fixtures";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import {
  acquireLiveVoice,
  applySpectrumJobProgress,
  applySpectrumReport,
  canAnalyzeAverage,
  diagnosticsState,
  freezeSnapshot,
  clearSnapshots,
  resetDiagnosticsForTest,
  startAverage,
  startSourceVsProcessed,
} from "./diagnostics.svelte";

/** H-42 (SPEC-007 §8.5): the long-term average job flow, snapshots and the voice stream's
 * reference counting. */

async function openFixture(overrides: Partial<DocumentDto> = {}): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return doc(overrides);
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

/** A `VXLT` frame: 3 bins, no room tone. */
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

const EMPTY: VoiceReportDto = {
  f0: null,
  tone: null,
  sibilance: null,
  hum: null,
  rumble_db: null,
  noise_floor_dbfs: -70,
  active_level_dbfs: null,
  snr_db: null,
  span_s: 2,
};

function report(jobId: number, sources: Array<"source" | "processed">): SpectrumReportDto {
  return {
    job_id: jobId,
    sample_rate_hz: 48_000,
    fft_size: 4,
    window: "hann",
    start_sample: 0,
    end_sample: 48_000,
    results: sources.map((source) => ({ source, frames: 10, has_noise: false, report: EMPTY })),
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetSelectionForTest();
  resetDiagnosticsForTest();
});

describe("long-term average job (H-42)", () => {
  it("needs a document, and analyzes the selection or the whole file", async () => {
    expect(canAnalyzeAverage()).toBe(false);
    await openFixture({ len_samples: 96_000 });
    expect(canAnalyzeAverage()).toBe(true);

    const requests: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        requests.push((args as { request: unknown }).request);
        return { job_id: 3 };
      }
      return null;
    });
    await startAverage();
    expect(requests[0]).toEqual({
      start_sample: 0,
      end_sample: 96_000,
      sources: ["processed"],
      fft_size: 16_384,
      window: "hann",
    });
    expect(diagnosticsState().job).toMatchObject({ jobId: 3, state: "running", purpose: "average" });
    // A second start while running is refused.
    expect(canAnalyzeAverage()).toBe(false);

    resetDiagnosticsForTest();
    setSelectionFromResult([1000, 5000]);
    await startAverage();
    expect(requests[1]).toMatchObject({ start_sample: 1000, end_sample: 5000 });
  });

  it("follows progress, then fetches each curve when the report arrives", async () => {
    await openFixture({ len_samples: 96_000 });
    const fetched: Array<[number, number]> = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        return { job_id: 9 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        fetched.push([a.jobId, a.index]);
        return vxlt(a.jobId, a.index, [-10, -20, -30]);
      }
      return null;
    });
    await startAverage();
    applySpectrumJobProgress({ job_id: 9, kind: "spectrum_analyze", state: "running", fraction: 0.4 });
    applySpectrumJobProgress({ job_id: 9, kind: "loudness_analyze", state: "running", fraction: 0.9 });
    applySpectrumJobProgress({ job_id: 8, kind: "spectrum_analyze", state: "running", fraction: 0.9 });
    expect(diagnosticsState().job?.fraction).toBe(0.4);
    applySpectrumJobProgress({ job_id: 9, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport(report(9, ["processed"]));
    expect(fetched).toEqual([[9, 0]]);
    const [avg] = diagnosticsState().averages;
    expect(avg?.source).toBe("processed");
    expect(Array.from(avg!.curve.freqsHz)).toEqual([0, 12_000, 24_000]);
    expect(Array.from(avg!.curve.levelsDb)).toEqual([-10, -20, -30]);
    // A stale report for another job changes nothing.
    await applySpectrumReport(report(1, ["source"]));
    expect(diagnosticsState().averages.length).toBe(1);
  });

  it("Source vs Processed freezes the two results as A and B", async () => {
    await openFixture({ len_samples: 96_000 });
    const requests: Array<{ sources: string[] }> = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        requests.push((args as { request: { sources: string[] } }).request);
        return { job_id: 4 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        return vxlt(4, a.index, a.index === 0 ? [-1, -2, -3] : [-4, -5, -6]);
      }
      return null;
    });
    await startSourceVsProcessed();
    expect(requests[0]?.sources).toEqual(["source", "processed"]);
    await applySpectrumReport(report(4, ["source", "processed"]));
    const { a, b } = diagnosticsState().snapshots;
    expect(a?.origin).toBe("source");
    expect(Array.from(a!.curve.levelsDb)).toEqual([-1, -2, -3]);
    expect(b?.origin).toBe("processed");
    expect(Array.from(b!.curve.levelsDb)).toEqual([-4, -5, -6]);
  });
});

describe("snapshots and the live voice stream (H-42)", () => {
  it("freezes a copy, not a view", () => {
    const levels = Float32Array.from([-1, -2]);
    freezeSnapshot("a", "live", { freqsHz: [100, 200], levelsDb: levels, resolution: "bands" });
    levels[0] = 0;
    expect(diagnosticsState().snapshots.a?.curve.levelsDb[0]).toBe(-1);
    clearSnapshots();
    expect(diagnosticsState().snapshots).toEqual({ a: null, b: null });
  });

  it("subscribes once for many holders and unsubscribes after the last", async () => {
    const calls: Array<[string, unknown]> = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      return cmd === "analyzer_voice_subscribe" ? 5 : null;
    });
    const r1 = acquireLiveVoice();
    const r2 = acquireLiveVoice();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls.filter(([c]) => c === "analyzer_voice_subscribe").length).toBe(1);
    r1();
    r1();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe").length).toBe(0);
    r2();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe")).toEqual([["analyzer_unsubscribe", { id: 5 }]]);
  });
});

describe("H-96 belt-and-braces recovery for the average job (H-92 needs this: it starts this job itself)", () => {
  it("recovers via job_status if the terminal spectrum_report/job_progress pair never arrives", async () => {
    await openFixture({ len_samples: 96_000 });
    vi.useFakeTimers();
    try {
      mockIPC((cmd) => {
        if (cmd === "spectrum_analyze_start") {
          return { job_id: 55 };
        }
        if (cmd === "job_status") {
          return { job_id: 55, kind: "spectrum_analyze", state: "done", fraction: 1 };
        }
        return null;
      });
      await startAverage();
      expect(diagnosticsState().job?.state).toBe("running");
      await vi.advanceTimersByTimeAsync(3_000);
      expect(diagnosticsState().job).toMatchObject({ jobId: 55, state: "done", fraction: 1 });
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops polling once the store is reset (no leaked timers)", async () => {
    await openFixture({ len_samples: 96_000 });
    vi.useFakeTimers();
    try {
      let statusCalls = 0;
      mockIPC((cmd) => {
        if (cmd === "spectrum_analyze_start") {
          return { job_id: 56 };
        }
        if (cmd === "job_status") {
          statusCalls += 1;
          return { job_id: 56, kind: "spectrum_analyze", state: "running", fraction: 0.3 };
        }
        return null;
      });
      await startAverage();
      await vi.advanceTimersByTimeAsync(3_000);
      expect(statusCalls).toBe(1);
      resetDiagnosticsForTest();
      await vi.advanceTimersByTimeAsync(30_000);
      expect(statusCalls).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("H-108: the no-progress timeout backstop", () => {
  it("fails a job that goes completely silent for the timeout, cancels it best-effort, and notifies", async () => {
    await openFixture({ len_samples: 96_000 });
    vi.useFakeTimers();
    try {
      const cancelled: number[] = [];
      mockIPC((cmd, args) => {
        if (cmd === "spectrum_analyze_start") {
          return { job_id: 70 };
        }
        if (cmd === "spectrum_analyze_cancel") {
          cancelled.push((args as { jobId: number }).jobId);
          return null;
        }
        // "job_status" (H-96's own recovery) unmocked here too — the scenario is *everything*
        // going silent, not just the real `job_progress` listener.
        return null;
      });
      await startAverage();
      expect(diagnosticsState().job?.state).toBe("running");
      await vi.advanceTimersByTimeAsync(30_000);
      expect(diagnosticsState().job).toMatchObject({ jobId: 70, state: "failed" });
      expect(cancelled).toContain(70);
      expect(noticesState().toasts.some((n) => n.key === "error.spectrum.timeout")).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a real progress tick resets the watchdog, so a legitimately slow job is never timed out", async () => {
    await openFixture({ len_samples: 96_000 });
    vi.useFakeTimers();
    try {
      mockIPC((cmd) => (cmd === "spectrum_analyze_start" ? { job_id: 71 } : null));
      await startAverage();
      // A tick every 20 s (well inside the 30 s timeout) for over a minute total — the timeout
      // must never fire as long as *something* keeps arriving.
      for (let i = 1; i <= 4; i++) {
        await vi.advanceTimersByTimeAsync(20_000);
        applySpectrumJobProgress({ job_id: 71, kind: "spectrum_analyze", state: "running", fraction: i / 4 });
      }
      expect(diagnosticsState().job).toMatchObject({ jobId: 71, state: "running" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops the watchdog once the store is reset (no leaked timer failing a later job)", async () => {
    await openFixture({ len_samples: 96_000 });
    vi.useFakeTimers();
    try {
      const cancelled: number[] = [];
      mockIPC((cmd, args) => {
        if (cmd === "spectrum_analyze_start") {
          return { job_id: 72 };
        }
        if (cmd === "spectrum_analyze_cancel") {
          cancelled.push((args as { jobId: number }).jobId);
        }
        return null;
      });
      await startAverage();
      resetDiagnosticsForTest();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(cancelled).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});
