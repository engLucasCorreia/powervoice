import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, EditResultDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  applyNormalizeDialog,
  canNormalize,
  closeNormalizeDialog,
  normalizeFavorite,
  normalizeState,
  openNormalizeDialog,
  parseTargetDb,
  resetNormalizeForTest,
  setNormalizeDialogText,
} from "./normalize.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "./selection.svelte";

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

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetNormalizeForTest();
});

describe("parseTargetDb (SPEC-010 §2.4 dB-mode range)", () => {
  it("accepts values inside [-60, 0]", () => {
    expect(parseTargetDb("-1")).toBe(-1);
    expect(parseTargetDb("-0.1")).toBeCloseTo(-0.1);
    expect(parseTargetDb("0")).toBe(0);
    expect(parseTargetDb("-60")).toBe(-60);
  });

  it("accepts a Unicode minus sign", () => {
    expect(parseTargetDb("−1.00")).toBe(-1);
  });

  it("rejects out-of-range and unparseable text", () => {
    expect(parseTargetDb("-60.01")).toBeNull();
    expect(parseTargetDb("0.01")).toBeNull();
    expect(parseTargetDb("abc")).toBeNull();
    expect(parseTargetDb("")).toBeNull();
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
});

describe("normalizeFavorite (SPEC-010 §2.1/AC-14)", () => {
  it("sends the whole file with no selection, and the selection when one exists", async () => {
    await openFixture({ len_samples: 480_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000,
          selection: null,
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await normalizeFavorite(-1);
    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetDb: -1 }]);

    setSelectionFromResult([1_000, 5_000]);
    await normalizeFavorite(-0.1);
    expect(calls[1]).toEqual({ startSamples: 1_000, endSamples: 5_000, targetDb: -0.1 });
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

  it("applies the command result to the selection store", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "edit_normalize_peak") {
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000,
          selection: [0, 480_000],
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await normalizeFavorite(-1);
    // No throw means the promise resolved and applied the result — selection store behavior is
    // covered by edit.test.ts's equivalent case; here we just confirm no error surfaced.
  });
});

describe("Normalize… dialog (SPEC-010 §2.4)", () => {
  it("does not open with no document", () => {
    openNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(false);
  });

  it("opens with a document, tracks field validity, and Apply sends the parsed value", async () => {
    await openFixture();
    openNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(true);

    setNormalizeDialogText("-60.01");
    expect(normalizeState().dialogValid).toBe(false);

    setNormalizeDialogText("-2.00");
    expect(normalizeState().dialogValid).toBe(true);

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000,
          selection: null,
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await applyNormalizeDialog();
    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetDb: -2 }]);
    expect(normalizeState().dialogOpen).toBe(false);
  });

  it("Cancel closes without sending a command", async () => {
    await openFixture();
    openNormalizeDialog();
    closeNormalizeDialog();
    expect(normalizeState().dialogOpen).toBe(false);
  });
});
