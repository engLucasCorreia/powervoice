import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { AcxCheckReportDto, DocumentDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import { acxState, resetAcxForTest, runAcxCheck } from "./acx.svelte";
import { resetLoudnessForTest, setLoudnessSource } from "./loudness.svelte";

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

const PASSING_REPORT: AcxCheckReportDto = {
  passes: true,
  rms: { measured_db: -20.0, status: "pass" },
  peak: { measured_db: -6.0, status: "pass" },
  noise_floor: { measured_db: -65.0, status: "pass" },
};

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetLoudnessForTest();
  resetAcxForTest();
});

describe("runAcxCheck", () => {
  it("sends the current loudness source and stores the report", async () => {
    await openFixture();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "acx_check") {
        calls.push(args);
        return PASSING_REPORT;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await runAcxCheck();
    expect(calls).toEqual([{ request: { source: "processed" } }]);
    expect(acxState().report).toEqual(PASSING_REPORT);
    expect(acxState().running).toBe(false);

    setLoudnessSource("source");
    await runAcxCheck();
    expect(calls[1]).toEqual({ request: { source: "source" } });
  });

  it("reports a failed acx_check as a notice and leaves the report untouched", async () => {
    await openFixture();
    mockIPC((cmd) => {
      if (cmd === "acx_check") {
        throw { code: "not_found", key: "error.document.none", params: {} };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await runAcxCheck();
    expect(acxState().report).toBeNull();
    expect(acxState().running).toBe(false);
    expect(noticesState().toasts.some((n) => n.key === "error.document.none")).toBe(true);
  });

  it("does nothing with no document open", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      throw new Error(`unmocked command: ${cmd}`);
    });
    await runAcxCheck();
    expect(calls).toHaveLength(0);
    expect(acxState().report).toBeNull();
  });

  it("sets running while the command is in flight", async () => {
    await openFixture();
    let resolveCommand: ((value: AcxCheckReportDto) => void) | undefined;
    const deferred = new Promise<AcxCheckReportDto>((resolve) => {
      resolveCommand = resolve;
    });
    mockIPC((cmd) => {
      if (cmd === "acx_check") {
        return deferred;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = runAcxCheck();
    expect(acxState().running).toBe(true);
    resolveCommand?.(PASSING_REPORT);
    await pending;
    expect(acxState().running).toBe(false);
  });
});
