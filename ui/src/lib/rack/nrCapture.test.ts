import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { JobProgressDto } from "../ipc/bindings";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import {
  applyJobProgress,
  canCapture,
  cancelCapture,
  isCapturing,
  nrCaptureState,
  resetNrCaptureForTest,
  startCapture,
} from "./nrCapture.svelte";
import { resetRackForTest } from "./rack.svelte";

/**
 * Capture Noise Print store tests (S3-06, SPEC-014 §2.3, AC-21 status/button part). Mirrors
 * `export.test.ts`'s shape: `mockIPC` for the command round trip, `applyJobProgress` tested as a
 * pure function.
 */

afterEach(() => {
  clearMocks();
  clearNotices();
  resetSelectionForTest();
  resetRecordForTest();
  resetRackForTest();
  resetNrCaptureForTest();
});

describe("canCapture", () => {
  it("is false without a selection", () => {
    expect(canCapture()).toBe(false);
  });

  it("is true with a non-empty selection and not recording", () => {
    setSelectionFromResult([1_000, 5_000]);
    expect(canCapture()).toBe(true);
  });
});

describe("startCapture", () => {
  it("is a no-op without a selection", async () => {
    let called = false;
    mockIPC(() => {
      called = true;
      throw new Error("should not be called");
    });
    await startCapture(0);
    expect(called).toBe(false);
    expect(nrCaptureState().job).toBeNull();
  });

  it("sends the clicked slot as the hint, and the current selection", async () => {
    setSelectionFromResult([1_000, 25_000]);
    let sentArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "nr_capture_start") {
        sentArgs = args;
        return { job_id: 7, slot: 2 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(2);
    expect(sentArgs).toEqual({ hintSlot: 2, start: 1_000, end: 25_000 });
    expect(nrCaptureState().job).toEqual({ jobId: 7, slot: 2, fraction: 0, state: "running" });
    expect(isCapturing(2)).toBe(true);
    expect(isCapturing(0)).toBe(false);
  });

  it("falls back to the last-focused slot when no hint is given (Shift+P)", async () => {
    setSelectionFromResult([0, 24_000]);
    // Simulate a prior focus via the rack store's own tracker (RackSlot.svelte's onfocusin).
    const { noteSlotFocused } = await import("./rack.svelte");
    noteSlotFocused(3);
    let sentArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "nr_capture_start") {
        sentArgs = args;
        return { job_id: 1, slot: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(null);
    expect(sentArgs).toEqual({ hintSlot: 3, start: 0, end: 24_000 });
  });

  it("does nothing while a capture is already running", async () => {
    setSelectionFromResult([0, 24_000]);
    let calls = 0;
    mockIPC((cmd) => {
      if (cmd === "nr_capture_start") {
        calls += 1;
        return { job_id: 1, slot: 0 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(0);
    await startCapture(0);
    expect(calls).toBe(1);
  });

  it("reports a failed nr_capture_start as a notice", async () => {
    setSelectionFromResult([0, 24_000]);
    mockIPC((cmd) => {
      if (cmd === "nr_capture_start") {
        throw { code: "invalid_argument", key: "error.nr_capture.too_short", params: {} };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(0);
    expect(nrCaptureState().job).toBeNull();
    expect(noticesState().toasts.some((n) => n.key === "error.nr_capture.too_short")).toBe(true);
  });
});

describe("applyJobProgress", () => {
  it("ignores events of a different kind, or for no/a different job", () => {
    applyJobProgress({ job_id: 1, kind: "export", state: "running", fraction: 0.5 });
    expect(nrCaptureState().job).toBeNull();
  });

  it("updates the running job's fraction and terminal state", async () => {
    setSelectionFromResult([0, 24_000]);
    mockIPC((cmd) => {
      if (cmd === "nr_capture_start") {
        return { job_id: 9, slot: 0 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(0);

    applyJobProgress({ job_id: 999, kind: "nr_capture", state: "done", fraction: 1 });
    expect(nrCaptureState().job?.state).toBe("running");

    const midway: JobProgressDto = { job_id: 9, kind: "nr_capture", state: "running", fraction: 0.6 };
    applyJobProgress(midway);
    expect(nrCaptureState().job?.fraction).toBe(0.6);

    applyJobProgress({ job_id: 9, kind: "nr_capture", state: "done", fraction: 1 });
    expect(nrCaptureState().job).toEqual({ jobId: 9, slot: 0, fraction: 1, state: "done" });
    expect(isCapturing(0)).toBe(false);
  });
});

describe("cancelCapture", () => {
  it("calls nr_capture_cancel with the running job's id", async () => {
    setSelectionFromResult([0, 24_000]);
    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "nr_capture_start") {
        return { job_id: 4, slot: 0 };
      }
      if (cmd === "nr_capture_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await startCapture(0);
    cancelCapture();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(4);
  });
});
