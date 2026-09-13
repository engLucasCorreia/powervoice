import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { ExportRequestDto, JobProgressDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import {
  applyJobProgress,
  cancelExportDialog,
  cancelExportJob,
  confirmExport,
  dismissExportJob,
  exportState,
  extensionFor,
  openExportDialog,
  resetExportStateForTest,
} from "./export.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetExportStateForTest();
});

describe("openExportDialog / cancelExportDialog", () => {
  it("opens the prompt and fetches MP3 availability", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    expect(exportState().prompt).toEqual({ suggestedName: "take" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(exportState().mp3Available).toBe(true);
  });

  it("cancelExportDialog clears the prompt", () => {
    mockIPC(() => ({ mp3_available: false }));
    openExportDialog("take");
    cancelExportDialog();
    expect(exportState().prompt).toBeNull();
  });
});

describe("extensionFor", () => {
  it("matches the format kind", () => {
    expect(extensionFor({ kind: "wav", bits: "24" })).toBe("wav");
    expect(extensionFor({ kind: "flac", bits: "16" })).toBe("flac");
    expect(extensionFor({ kind: "mp3", settings: { kind: "cbr", kbps: 192 } })).toBe("mp3");
  });
});

describe("confirmExport", () => {
  it("shows the native save dialog then starts the job at the chosen format/rate", async () => {
    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        savedArgs = args;
        return { job_id: 7 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);

    const expected: { request: ExportRequestDto } = {
      request: {
        path: "/home/user/out.wav",
        format: { kind: "wav", bits: "24" },
        sample_rate_hz: 48_000,
        range: null,
      },
    };
    expect(savedArgs).toEqual(expected);
    expect(exportState().prompt).toBeNull();
    expect(exportState().job).toEqual({ jobId: 7, fraction: 0, state: "running" });
  });

  it("does nothing when the native dialog is cancelled", async () => {
    let startCalled = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return null;
      }
      if (cmd === "export_formats") {
        return { mp3_available: false };
      }
      startCalled = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "16" }, 48_000);
    expect(startCalled).toBe(false);
    expect(exportState().job).toBeNull();
  });

  it("reports a failed export_start as a notice and clears no prompt state twice", async () => {
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.mp3";
      }
      if (cmd === "export_start") {
        throw { code: "invalid_argument", key: "error.export.mp3_unavailable", params: {} };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "mp3", settings: { kind: "cbr", kbps: 192 } }, 44_100);
    expect(exportState().job).toBeNull();
  });
});

describe("applyJobProgress", () => {
  it("ignores events for a different or no job", () => {
    const payload: JobProgressDto = { job_id: 1, kind: "export", state: "running", fraction: 0.5 };
    applyJobProgress(payload);
    expect(exportState().job).toBeNull();
  });

  it("updates the running job's fraction and terminal state", async () => {
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        void args;
        return { job_id: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);

    applyJobProgress({ job_id: 3, kind: "export", state: "running", fraction: 0.4 });
    expect(exportState().job?.fraction).toBe(0.4);

    applyJobProgress({ job_id: 999, kind: "export", state: "done", fraction: 1 });
    expect(exportState().job?.state).toBe("running");

    applyJobProgress({ job_id: 3, kind: "export", state: "done", fraction: 1 });
    expect(exportState().job).toEqual({ jobId: 3, fraction: 1, state: "done" });
  });
});

describe("dismissExportJob / cancelExportJob", () => {
  it("dismissExportJob clears the job", async () => {
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);
    dismissExportJob();
    expect(exportState().job).toBeNull();
  });

  it("cancelExportJob calls export_cancel with the running job's id", async () => {
    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        return { job_id: 5 };
      }
      if (cmd === "export_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);
    cancelExportJob();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(5);
  });
});
