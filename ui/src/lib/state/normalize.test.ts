import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, NormalizeResultDto, Settings } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  applyNormalizeDialog,
  applyNormalizeJobProgress,
  applyNormalizeResult,
  canNormalize,
  cancelNormalizeJob,
  closeNormalizeDialog,
  dismissNormalizeJob,
  normalizeFavorite,
  normalizeState,
  openNormalizeDialog,
  parseNormalizeTarget,
  parseTargetDb,
  resetNormalizeForTest,
  setNormalizeDialogText,
  setNormalizeDialogUnit,
  targetDbToPct,
  targetPctToDb,
} from "./normalize.svelte";
import { resetSelectionForTest, selectionState, setSelectionFromResult } from "./selection.svelte";
import { loadSettings, resetSettingsStateForTest } from "./settings.svelte";
import { resetWaveformViewForTest } from "./waveformView.svelte";

function doc(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    sidecar_dirty: false,
    spectral_view: null, waveform_view: null, recovered: false,
    ...overrides,
  };
}

function settingsFixture(overrides: Partial<Settings> = {}): Settings {
  return {
    version: 1,
    device: {
      host: "pipewire",
      input_device: null,
      input_channel: 1,
      output_device: null,
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    default_format: { sample_rate_hz: 48_000, bit_depth: "24" },
    monitor_mode: "off",
    monitor_hint_shown: false,
    telemetry_rate_hz: 60,
    memory_budget_mib: 2048,
    normalize_dialog: { value: -1, unit: "db" },
    recent_files: [],
    spectral_defaults: {
      freq_scale: "log",
      colormap: "inferno",
      display_floor_db: -120,
      display_ceil_db: 0,
      fft_size: null,
    },
    analyzer_visible: true,
    analyzer_response: "medium",
    analyzer_peak_hold: true,
    multichannel_policy: "ask",
    renderer_preference: "auto",
    layout: {
      markers_width_px: 240,
      rack_width_px: 280,
      dock_height_px: 240,
      markers_collapsed: false,
      rack_collapsed: false,
      dock_tab: "meters",
    },
    record: {
      mode: "insert",
      punch_on_selection: true,
      preroll_s: 5,
      postroll_s: 1,
      preroll_at_cursor: false,
      hear_original: false,
      punch_xfade_ms: 10,
    },
    record_offsets: [],
    save_dither: "tpdf",
    theme: "dark",
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

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetSelectionForTest();
  resetNormalizeForTest();
  resetSettingsStateForTest();
});

describe("parseTargetDb / parseNormalizeTarget (SPEC-010 §2.4 ranges)", () => {
  it("dB mode accepts values inside [-60, 0]", () => {
    expect(parseTargetDb("-1")).toBe(-1);
    expect(parseTargetDb("-0.1")).toBeCloseTo(-0.1);
    expect(parseTargetDb("0")).toBe(0);
    expect(parseTargetDb("-60")).toBe(-60);
  });

  it("accepts a Unicode minus sign", () => {
    expect(parseTargetDb("−1.00")).toBe(-1);
  });

  it("dB mode rejects out-of-range and unparseable text", () => {
    expect(parseTargetDb("-60.01")).toBeNull();
    expect(parseTargetDb("0.01")).toBeNull();
    expect(parseTargetDb("abc")).toBeNull();
    expect(parseTargetDb("")).toBeNull();
  });

  it("% mode accepts values inside [0.1, 100.0] and rejects outside it", () => {
    expect(parseNormalizeTarget("50.0", "pct")).toBe(50);
    expect(parseNormalizeTarget("0.1", "pct")).toBe(0.1);
    expect(parseNormalizeTarget("100.0", "pct")).toBe(100);
    expect(parseNormalizeTarget("0.0", "pct")).toBeNull();
    expect(parseNormalizeTarget("100.1", "pct")).toBeNull();
  });
});

describe("targetDbToPct / targetPctToDb (SPEC-010 §2.4 AC-2 %-target math)", () => {
  it("converts −1.00 dB ↔ 89.1 % (SPEC-010 §2.4 example)", () => {
    expect(targetDbToPct(-1)).toBeCloseTo(89.1, 1);
    expect(targetPctToDb(89.125)).toBeCloseTo(-1, 2);
  });

  it("50.0 %, 100.0 % and 0.1 % give −6.02, 0.00 and −60.00 dBFS (AC-2)", () => {
    expect(targetPctToDb(50.0)).toBeCloseTo(-6.02, 2);
    expect(targetPctToDb(100.0)).toBeCloseTo(0.0, 2);
    expect(targetPctToDb(0.1)).toBeCloseTo(-60.0, 2);
  });

  it("round-trips", () => {
    for (const db of [-60, -23.5, -6.02, -1, -0.1, 0]) {
      expect(targetPctToDb(targetDbToPct(db))).toBeCloseTo(db, 6);
    }
  });
});

describe("canNormalize (SPEC-010 §2.1 scope)", () => {
  it("is false with no document, or an empty one", async () => {
    expect(canNormalize()).toBe(false);
    await openFixture({ len_samples: 0 });
    expect(canNormalize()).toBe(false);
  });

  it("is true once a non-empty document is open", async () => {
    await openFixture();
    expect(canNormalize()).toBe(true);
  });

  it("is false while a job is running (SPEC-010 §2.1: disabled while another job runs)", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);
    expect(canNormalize()).toBe(false);
  });
});

describe("normalizeFavorite (SPEC-010 §2.1/AC-14, H-09: starts a job)", () => {
  it("sends the whole file with no selection, and the selection when one exists", async () => {
    await openFixture({ len_samples: 480_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await normalizeFavorite(-1);
    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: -1, targetPct: null },
    ]);
    expect(normalizeState().job).toEqual({ jobId: 1, fraction: 0, state: "running" });

    setSelectionFromResult([1_000, 5_000]);
    dismissNormalizeJob();
    await normalizeFavorite(-0.1);
    expect(calls[1]).toEqual({
      startSamples: 1_000,
      endSamples: 5_000,
      targetDb: -0.1,
      targetPct: null,
    });
  });

  it("does nothing with no document open", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-3);
    expect(calls).toHaveLength(0);
  });
});

describe("applyNormalizeJobProgress / applyNormalizeResult", () => {
  it("ignores events for a different job, id, or kind", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);

    applyNormalizeJobProgress({ job_id: 3, kind: "normalize_lufs", state: "done", fraction: 1 });
    expect(normalizeState().job?.state).toBe("running");

    applyNormalizeJobProgress({ job_id: 999, kind: "normalize_peak", state: "done", fraction: 1 });
    expect(normalizeState().job?.state).toBe("running");

    applyNormalizeJobProgress({ job_id: 3, kind: "normalize_peak", state: "running", fraction: 0.4 });
    expect(normalizeState().job).toEqual({ jobId: 3, fraction: 0.4, state: "running" });

    applyNormalizeJobProgress({ job_id: 3, kind: "normalize_peak", state: "done", fraction: 1 });
    expect(normalizeState().job).toEqual({ jobId: 3, fraction: 1, state: "done" });
  });

  it("a normalize_result event for the running job updates the selection", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 7 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);

    const result: NormalizeResultDto = {
      job_id: 7,
      kind: "normalize_peak",
      result: {
        changed: true,
        audio_rev: 2,
        len_samples: 480_000,
        selection: [10, 20],
        playhead_samples: 10,
      },
    };
    applyNormalizeResult(result);
    expect(selectionState().current).toEqual({ startSample: 10, endSample: 20 });
  });
});

describe("cancelNormalizeJob / dismissNormalizeJob", () => {
  it("cancelNormalizeJob calls edit_normalize_peak_cancel with the running job's id", async () => {
    await openFixture();
    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 5 };
      }
      if (cmd === "edit_normalize_peak_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);
    cancelNormalizeJob();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(5);
  });

  it("dismissNormalizeJob clears the job", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);
    dismissNormalizeJob();
    expect(normalizeState().job).toBeNull();
  });
});

describe("Normalize… dialog (SPEC-010 §2.4)", () => {
  it("does not open with no document", () => {
    openNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(false);
  });

  it("opens with a document, tracks field validity, and Apply starts a job with the parsed value", async () => {
    await openFixture();
    openNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(true);

    setNormalizeDialogText("-60.01");
    expect(normalizeState().dialogValid).toBe(false);

    setNormalizeDialogText("-2.00");
    expect(normalizeState().dialogValid).toBe(true);

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await applyNormalizeDialog();
    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: -2, targetPct: null },
    ]);
    expect(normalizeState().dialogOpen).toBe(false);
  });

  it("Apply persists the applied value and unit via settings.svelte.ts's saveSettings", async () => {
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();
    await openFixture();
    openNormalizeDialog();
    setNormalizeDialogText("-2.00");

    let savedSettings: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        return { job_id: 1 };
      }
      if (cmd === "settings_set") {
        savedSettings = (args as { settings: Settings }).settings;
        return savedSettings;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await applyNormalizeDialog();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(savedSettings?.normalize_dialog).toEqual({ value: -2, unit: "db" });
  });

  it("Cancel closes without sending a command", async () => {
    await openFixture();
    openNormalizeDialog();
    closeNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(false);
  });

  it("the % unit toggle converts the shown value and validates against the % range", async () => {
    await openFixture();
    openNormalizeDialog();
    setNormalizeDialogText("-1.00");
    expect(normalizeState().dialogUnit).toBe("db");

    setNormalizeDialogUnit("pct");
    expect(normalizeState().dialogUnit).toBe("pct");
    expect(Number(normalizeState().dialogText)).toBeCloseTo(89.1, 1);
    expect(normalizeState().dialogValid).toBe(true);

    setNormalizeDialogUnit("db");
    expect(normalizeState().dialogUnit).toBe("db");
    expect(Number(normalizeState().dialogText)).toBeCloseTo(-1.0, 1);
  });

  it("Apply in % mode sends targetPct (not targetDb) — the value in the unit it was entered", async () => {
    await openFixture();
    openNormalizeDialog();
    setNormalizeDialogUnit("pct");
    setNormalizeDialogText("50.0");
    expect(normalizeState().dialogValid).toBe(true);

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await applyNormalizeDialog();
    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: null, targetPct: 50 },
    ]);
  });

  it("reopens with the last applied value and unit (SPEC-010 §2.4 dialog memory)", async () => {
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        return settingsFixture({ normalize_dialog: { value: 75.0, unit: "pct" } });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();
    await openFixture();

    openNormalizeDialog();
    expect(normalizeState().dialogUnit).toBe("pct");
    expect(normalizeState().dialogText).toBe("75.0");
  });
});
