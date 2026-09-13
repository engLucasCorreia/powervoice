import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, EditResultDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  applyNormalizeLufsDialog,
  canNormalizeLufs,
  closeNormalizeLufsDialog,
  normalizeLufsFavorite,
  normalizeLufsState,
  openNormalizeLufsDialog,
  parseTargetLufs,
  resetNormalizeLufsForTest,
  setNormalizeLufsDialogText,
} from "./normalizeLufs.svelte";
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
});

describe("normalizeLufsFavorite", () => {
  it("sends the whole file with no selection, and the selection when one exists", async () => {
    await openFixture({ len_samples: 480_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_lufs") {
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

    await normalizeLufsFavorite(-16);
    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetLufs: -16 }]);

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

describe("Normalize (LUFS)… dialog", () => {
  it("does not open with no document", () => {
    openNormalizeLufsDialog();
    expect(normalizeLufsState().dialogOpen).toBe(false);
  });

  it("opens with a document, tracks field validity, and Apply sends the parsed value", async () => {
    await openFixture();
    openNormalizeLufsDialog();
    expect(normalizeLufsState().dialogOpen).toBe(true);

    setNormalizeLufsDialogText("-60.01");
    expect(normalizeLufsState().dialogValid).toBe(false);

    setNormalizeLufsDialogText("-14.0");
    expect(normalizeLufsState().dialogValid).toBe(true);

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_lufs") {
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
});
