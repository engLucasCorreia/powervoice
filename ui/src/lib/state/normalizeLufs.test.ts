import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, NormalizeResultDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  applyNormalizeLufsDialog,
  applyNormalizeLufsJobProgress,
  applyNormalizeLufsResult,
  canNormalizeLufs,
  cancelNormalizeLufsJob,
  closeNormalizeLufsDialog,
  dismissNormalizeLufsJob,
  normalizeLufsFavorite,
  normalizeLufsState,
  openNormalizeLufsDialog,
  parseTargetLufs,
  resetNormalizeLufsForTest,
  setNormalizeLufsDialogText,
} from "./normalizeLufs.svelte";
import { resetSelectionForTest, selectionState, setSelectionFromResult } from "./selection.svelte";
import { docDto as doc } from "../test/fixtures";
import { resetWaveformViewForTest } from "./waveformView.svelte";
import { MINUS } from "../ui/units";

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

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetSelectionForTest();
  resetNormalizeLufsForTest();
});

describe("parseTargetLufs (S4-01 custom-target range)", () => {
  it("accepts values inside [-60, 0]", () => {
    expect(parseTargetLufs("-16")).toBe(-16);
    expect(parseTargetLufs("-19.0")).toBeCloseTo(-19.0);
    expect(parseTargetLufs("0")).toBe(0);
    expect(parseTargetLufs("-60")).toBe(-60);
  });

  it("accepts a Unicode minus sign", () => {
    expect(parseTargetLufs("−23.0")).toBe(-23);
  });

  it("rejects out-of-range and unparseable text", () => {
    expect(parseTargetLufs("-60.01")).toBeNull();
    expect(parseTargetLufs("0.01")).toBeNull();
    expect(parseTargetLufs("abc")).toBeNull();
    expect(parseTargetLufs("")).toBeNull();
  });
});

describe("canNormalizeLufs (same scope convention as peak normalize)", () => {
  it("is false with no document, or an empty one", async () => {
    expect(canNormalizeLufs()).toBe(false);
    await openFixture({ len_samples: 0 });
    expect(canNormalizeLufs()).toBe(false);
  });

  it("is true once a non-empty document is open", async () => {
    await openFixture();
    expect(canNormalizeLufs()).toBe(true);
  });

  it("is false while a job is running", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_lufs_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-19);
    expect(canNormalizeLufs()).toBe(false);
  });
});

describe("normalizeLufsFavorite (H-09: starts a job)", () => {
  it("sends the whole file with no selection, and the selection when one exists", async () => {
    await openFixture({ len_samples: 480_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_lufs_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await normalizeLufsFavorite(-16);
    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetLufs: -16 }]);
    expect(normalizeLufsState().job).toEqual({ jobId: 1, fraction: 0, state: "running" });

    setSelectionFromResult([1_000, 5_000]);
    await normalizeLufsFavorite(-19);
    expect(calls[1]).toEqual({ startSamples: 1_000, endSamples: 5_000, targetLufs: -19 });
  });

  it("does nothing with no document open", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-23);
    expect(calls).toHaveLength(0);
  });
});

describe("applyNormalizeLufsJobProgress / applyNormalizeLufsResult", () => {
  it("ignores events for a different job, id, or kind", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_lufs_start") {
        return { job_id: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-19);

    applyNormalizeLufsJobProgress({ job_id: 3, kind: "normalize_peak", state: "done", fraction: 1 });
    expect(normalizeLufsState().job?.state).toBe("running");

    applyNormalizeLufsJobProgress({ job_id: 999, kind: "normalize_lufs", state: "done", fraction: 1 });
    expect(normalizeLufsState().job?.state).toBe("running");

    applyNormalizeLufsJobProgress({ job_id: 3, kind: "normalize_lufs", state: "running", fraction: 0.5 });
    expect(normalizeLufsState().job).toEqual({ jobId: 3, fraction: 0.5, state: "running" });
  });

  it("a normalize_result event for the running job updates the selection", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_lufs_start") {
        return { job_id: 7 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-19);

    const result: NormalizeResultDto = {
      job_id: 7,
      kind: "normalize_lufs",
      result: {
        changed: true,
        audio_rev: 2,
        len_samples: 480_000,
        selection: [10, 20],
        playhead_samples: 10,
      },
    };
    applyNormalizeLufsResult(result);
    expect(selectionState().current).toEqual({ startSample: 10, endSample: 20 });
  });
});

describe("cancelNormalizeLufsJob / dismissNormalizeLufsJob", () => {
  it("cancelNormalizeLufsJob calls edit_normalize_lufs_cancel with the running job's id", async () => {
    await openFixture();
    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_lufs_start") {
        return { job_id: 5 };
      }
      if (cmd === "edit_normalize_lufs_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-19);
    cancelNormalizeLufsJob();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(5);
  });

  it("dismissNormalizeLufsJob clears the job", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_lufs_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeLufsFavorite(-19);
    dismissNormalizeLufsJob();
    expect(normalizeLufsState().job).toBeNull();
  });
});

describe("Normalize (LUFS)… dialog", () => {
  it("does not open with no document", () => {
    openNormalizeLufsDialog();
    expect(normalizeLufsState().dialogOpen).toBe(false);
  });

  it("opens with a document, tracks field validity, and Apply starts a job with the parsed value", async () => {
    await openFixture();
    openNormalizeLufsDialog();
    expect(normalizeLufsState().dialogOpen).toBe(true);

    setNormalizeLufsDialogText("-60.01");
    expect(normalizeLufsState().dialogValid).toBe(false);

    setNormalizeLufsDialogText("-14.0");
    expect(normalizeLufsState().dialogValid).toBe(true);

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_lufs_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await applyNormalizeLufsDialog();
    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetLufs: -14 }]);
    expect(normalizeLufsState().dialogOpen).toBe(false);
  });

  it("Cancel closes without sending a command", async () => {
    await openFixture();
    openNormalizeLufsDialog();
    closeNormalizeLufsDialog();
    expect(normalizeLufsState().dialogOpen).toBe(false);
  });

  // H-28 item 4: the target field must show the true minus (U+2212, `units.ts::formatNumber`),
  // not the ASCII `-` `toFixed` writes, and must still accept an ASCII `-` typed back in.
  it("shows the true minus sign for the default target, and still parses an ASCII '-' back", async () => {
    await openFixture();
    openNormalizeLufsDialog();
    expect(normalizeLufsState().dialogText).toBe(`${MINUS}19.0`);

    setNormalizeLufsDialogText("-14.0");
    expect(normalizeLufsState().dialogValid).toBe(true);
    expect(parseTargetLufs(`${MINUS}14.0`)).toBe(-14);
  });
});
