import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, JobProgressDto, LoudnessReportDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import {
  applyLoudnessJobProgress,
  applyLoudnessReport,
  canAnalyzeLoudness,
  cancelLoudnessAnalyze,
  loudnessState,
  resetLoudnessForTest,
  setLoudnessSource,
  startLoudnessAnalyze,
} from "./loudness.svelte";

function doc(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    ...overrides,
  };
}

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

const REPORT: LoudnessReportDto = {
  job_id: 0,
  integrated_lufs: -19.0,
  max_momentary_lufs: -15.0,
  max_short_term_lufs: -17.0,
  lra_lu: 4.0,
  sample_peak_dbfs: -3.0,
  true_peak_dbtp: -2.5,
};

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetLoudnessForTest();
});

describe("canAnalyzeLoudness (same scope convention as normalize)", () => {
  it("is false with no document, or an empty one", async () => {
    expect(canAnalyzeLoudness()).toBe(false);
    await openFixture({ len_samples: 0 });
    expect(canAnalyzeLoudness()).toBe(false);
  });

  it("is true once a non-empty document is open", async () => {
    await openFixture();
    expect(canAnalyzeLoudness()).toBe(true);
  });
});

describe("setLoudnessSource", () => {
  it("defaults to processed and can be toggled", () => {
    expect(loudnessState().source).toBe("processed");
    setLoudnessSource("source");
    expect(loudnessState().source).toBe("source");
  });
});

describe("startLoudnessAnalyze", () => {
  it("sends the whole file with no selection, and the selection when one exists", async () => {
    await openFixture({ len_samples: 480_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "loudness_analyze_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await startLoudnessAnalyze();
    expect(calls).toEqual([
      { request: { start_sample: 0, end_sample: 480_000, source: "processed" } },
    ]);
    expect(loudnessState().job).toEqual({ jobId: 1, fraction: 0, state: "running" });

    setSelectionFromResult([1_000, 5_000]);
    setLoudnessSource("source");
    await startLoudnessAnalyze();
    expect(calls[1]).toEqual({
      request: { start_sample: 1_000, end_sample: 5_000, source: "source" },
    });
  });

  it("does nothing with no document open", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startLoudnessAnalyze();
    expect(calls).toHaveLength(0);
  });
});

describe("applyLoudnessJobProgress", () => {
  it("ignores events for a different job or kind", () => {
    applyLoudnessJobProgress({
      job_id: 1,
      kind: "loudness_analyze",
      state: "running",
      fraction: 0.5,
    });
    expect(loudnessState().job).toBeNull();
  });

  it("updates the running job's fraction and terminal state, ignoring other kinds", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "loudness_analyze_start") {
        return { job_id: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startLoudnessAnalyze();

    applyLoudnessJobProgress({ job_id: 3, kind: "export", state: "done", fraction: 1 } as JobProgressDto);
    expect(loudnessState().job?.state).toBe("running");

    applyLoudnessJobProgress({ job_id: 3, kind: "loudness_analyze", state: "running", fraction: 0.4 });
    expect(loudnessState().job?.fraction).toBe(0.4);

    applyLoudnessJobProgress({ job_id: 3, kind: "loudness_analyze", state: "done", fraction: 1 });
    expect(loudnessState().job).toEqual({ jobId: 3, fraction: 1, state: "done" });
  });
});

describe("applyLoudnessReport", () => {
  it("ignores a report for a different or no job", () => {
    applyLoudnessReport({ ...REPORT, job_id: 1 });
    expect(loudnessState().report).toBeNull();
  });

  it("stores the report for the current job", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "loudness_analyze_start") {
        return { job_id: 9 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startLoudnessAnalyze();
    applyLoudnessReport({ ...REPORT, job_id: 9 });
    expect(loudnessState().report).toEqual({ ...REPORT, job_id: 9 });
  });
});

describe("cancelLoudnessAnalyze", () => {
  it("calls loudness_analyze_cancel with the running job's id", async () => {
    await openFixture();
    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "loudness_analyze_start") {
        return { job_id: 5 };
      }
      if (cmd === "loudness_analyze_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startLoudnessAnalyze();
    cancelLoudnessAnalyze();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(5);
  });

  it("does nothing with no running job", () => {
    expect(() => cancelLoudnessAnalyze()).not.toThrow();
  });
});
