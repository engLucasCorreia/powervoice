import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ExportRequestDto, JobProgressDto, ParamInfoDto, RackSlotDto, RackStateDto } from "../ipc/bindings";
import { loadRack, resetRackForTest } from "../rack/rack.svelte";
import { clearNotices } from "../state/notices.svelte";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import {
  applyJobProgress,
  cancelExportDialog,
  cancelExportJob,
  cancelNoiseOnlyExport,
  confirmExport,
  continueNoiseOnlyExport,
  dismissExportJob,
  exportState,
  extensionFor,
  openExportDialog,
  rackHasNoiseOnlyOn,
  resetExportStateForTest,
  slotIsNoiseOnly,
} from "./export.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetExportStateForTest();
  resetRackForTest();
});

function noiseOnlyParam(overrides: Partial<ParamInfoDto> = {}): ParamInfoDto {
  return {
    id: 2,
    key: "noise_only",
    name: { text: "Output noise only", key: null },
    group: null,
    unit: { kind: "none" },
    min: 0,
    max: 1,
    default: 0,
    taper: { kind: "linear" },
    step: 1,
    enum_labels: [],
    decimals: 0,
    smoothing_ms: 0,
    flags: {
      automatable: true,
      stepped: true,
      boolean: true,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...overrides,
  };
}

/** An `org.powervoice.noise-reduction` slot, "Output noise only" on by default. */
function nrSlotFixture(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  const params = overrides.params ?? [noiseOnlyParam()];
  return rackSlotDto({
    module: "org.powervoice.noise-reduction@1.0.0",
    module_id: "org.powervoice.noise-reduction",
    name: "Noise Reduction",
    params,
    values: params.map((p) => ({ id: p.id, value: 1, normalized: 1, text: "On" })),
    noise_profile: "loaded",
    ...overrides,
  });
}

/** Seeds the live rack store (`rackState()`) via a mocked `loadRack()` — the way `confirmExport`
 * reads it for the SPEC-014 §2.6 noise-only check. */
async function seedRack(slots: RackSlotDto[]): Promise<void> {
  const state: RackStateDto = rackStateDto(slots);
  mockIPC((cmd) => {
    if (cmd === "rack_list_modules") {
      return [];
    }
    if (cmd === "rack_get") {
      return state;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await loadRack();
  clearMocks();
}

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

describe("confirmExport with a selection range (H-08)", () => {
  it("sends the range through to export_start unchanged", async () => {
    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        savedArgs = args;
        return { job_id: 9 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000, { start_sample: 100, end_sample: 200 });

    const expected: { request: ExportRequestDto } = {
      request: {
        path: "/home/user/out.wav",
        format: { kind: "wav", bits: "24" },
        sample_rate_hz: 48_000,
        range: { start_sample: 100, end_sample: 200 },
      },
    };
    expect(savedArgs).toEqual(expected);
  });
});

describe("slotIsNoiseOnly / rackHasNoiseOnlyOn (SPEC-014 §2.6)", () => {
  it("is true for a non-bypassed NR slot with noise_only on", () => {
    const slot = nrSlotFixture();
    expect(slotIsNoiseOnly(slot)).toBe(true);
    expect(rackHasNoiseOnlyOn([slot])).toBe(true);
  });

  it("is false when the slot is bypassed", () => {
    const slot = nrSlotFixture({ bypass: true });
    expect(slotIsNoiseOnly(slot)).toBe(false);
    expect(rackHasNoiseOnlyOn([slot])).toBe(false);
  });

  it("is false when noise_only is off", () => {
    const param = noiseOnlyParam();
    const slot = nrSlotFixture({
      params: [param],
      values: [{ id: param.id, value: 0, normalized: 0, text: "Off" }],
    });
    expect(slotIsNoiseOnly(slot)).toBe(false);
    expect(rackHasNoiseOnlyOn([slot])).toBe(false);
  });

  it("is false for a slot with no noise_only parameter at all", () => {
    const gainParam = noiseOnlyParam({ id: 0, key: "gain_db" });
    const slot = nrSlotFixture({
      module: "org.powervoice.gain@1.0.0",
      params: [gainParam],
      values: [{ id: gainParam.id, value: -6, normalized: 0.5, text: "-6.0 dB" }],
    });
    expect(slotIsNoiseOnly(slot)).toBe(false);
    expect(rackHasNoiseOnlyOn([slot])).toBe(false);
  });

  it("is true when any slot in the rack has it on", () => {
    const clean = nrSlotFixture({
      uid: 2,
      values: [{ id: 2, value: 0, normalized: 0, text: "Off" }],
    });
    const noisy = nrSlotFixture({ uid: 3 });
    expect(rackHasNoiseOnlyOn([clean, noisy])).toBe(true);
  });
});

describe("confirmExport noise-only confirmation (SPEC-014 §2.6)", () => {
  it("shows the confirmation instead of the native dialog when a live NR slot outputs noise only", async () => {
    await seedRack([nrSlotFixture()]);
    let saveDialogCalled = false;
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: false };
      }
      if (cmd === "plugin:dialog|save") {
        saveDialogCalled = true;
        return "/home/user/should-not-be-used.wav";
      }
      throw new Error(`unexpected command: ${cmd}`);
    });

    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);

    expect(saveDialogCalled).toBe(false);
    expect(exportState().prompt).toBeNull();
    expect(exportState().noiseOnlyConfirm).toEqual({
      format: { kind: "wav", bits: "24" },
      sampleRateHz: 48_000,
      range: null,
    });
    expect(exportState().job).toBeNull();
  });

  it("cancelNoiseOnlyExport drops the pending export without starting a job", async () => {
    await seedRack([nrSlotFixture()]);
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);

    cancelNoiseOnlyExport();
    expect(exportState().noiseOnlyConfirm).toBeNull();
    expect(exportState().job).toBeNull();
  });

  it("continueNoiseOnlyExport proceeds to the native dialog and starts the job", async () => {
    await seedRack([nrSlotFixture()]);
    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/noise-only.wav";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 11 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);
    expect(exportState().noiseOnlyConfirm).not.toBeNull();

    await continueNoiseOnlyExport();

    expect(exportState().noiseOnlyConfirm).toBeNull();
    expect(startArgs).toMatchObject({ request: { path: "/home/user/noise-only.wav" } });
    expect(exportState().job).toEqual({ jobId: 11, fraction: 0, state: "running" });
  });

  it("does not show the confirmation when the only NR slot is bypassed", async () => {
    await seedRack([nrSlotFixture({ bypass: true })]);
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        return { job_id: 4 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);

    expect(exportState().noiseOnlyConfirm).toBeNull();
    expect(exportState().job).toEqual({ jobId: 4, fraction: 0, state: "running" });
  });
});

describe("H-96: job_progress ordering and recovery", () => {
  /** The regression test named by the ticket: a job whose terminal event fires before the
   * listener would have attached. `export_start`'s mock applies the event as a side effect of
   * resolving — standing in for a fast export whose backend thread runs to completion and emits
   * `Done` before the start command's own promise resolves. Before the ordering fix,
   * `ensureListening()` only ran *after* this point (`await exportStart(...)` then
   * `await ensureListening()`), so this event was undeliverable and the store stayed stuck at
   * `running` — this test must fail against that code. */
  it("keeps a terminal event that fires before the start command resolves", async () => {
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "export_start") {
        applyJobProgress({ job_id: 9, kind: "export", state: "done", fraction: 1 });
        return { job_id: 9 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await confirmExport({ kind: "wav", bits: "24" }, 48_000);
    expect(exportState().job).toEqual({ jobId: 9, fraction: 1, state: "done" });
  });

  /** Belt and braces (item 2): even with the ordering fix, a UI that somehow still misses the
   * terminal event must recover — polling `job_status` on a timeout rather than trusting
   * `job_progress` alone. */
  it("recovers via job_status if the terminal event is missed entirely", async () => {
    vi.useFakeTimers();
    try {
      mockIPC((cmd) => {
        if (cmd === "plugin:dialog|save") {
          return "/home/user/out.wav";
        }
        if (cmd === "export_start") {
          return { job_id: 21 };
        }
        if (cmd === "job_status") {
          return { job_id: 21, kind: "export", state: "done", fraction: 1 };
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      openExportDialog("take");
      await confirmExport({ kind: "wav", bits: "24" }, 48_000);
      expect(exportState().job?.state).toBe("running");

      await vi.advanceTimersByTimeAsync(3_000);
      expect(exportState().job).toEqual({ jobId: 21, fraction: 1, state: "done" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops polling once the job is dismissed (no leaked timers)", async () => {
    vi.useFakeTimers();
    try {
      let statusCalls = 0;
      mockIPC((cmd) => {
        if (cmd === "plugin:dialog|save") {
          return "/home/user/out.wav";
        }
        if (cmd === "export_start") {
          return { job_id: 31 };
        }
        if (cmd === "job_status") {
          statusCalls += 1;
          return { job_id: 31, kind: "export", state: "running", fraction: 0.2 };
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      openExportDialog("take");
      await confirmExport({ kind: "wav", bits: "24" }, 48_000);
      await vi.advanceTimersByTimeAsync(3_000);
      expect(statusCalls).toBe(1);

      dismissExportJob();
      await vi.advanceTimersByTimeAsync(30_000);
      expect(statusCalls).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
