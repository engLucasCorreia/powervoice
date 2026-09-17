import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { EditResultDto } from "../ipc/bindings";
import {
  applyImportJobProgress,
  applyImportStarted,
  openDocument,
  resetDocumentStateForTest,
} from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  applyInsertSilenceDialog,
  canInsertSilence,
  closeInsertSilenceDialog,
  insertSilenceState,
  openInsertSilenceDialog,
  resetInsertSilenceForTest,
  setInsertSilenceDialogText,
} from "./insertSilence.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "./selection.svelte";
import { resetTransportForTest } from "./transport.svelte";
import { resetWaveformViewForTest } from "./waveformView.svelte";
import { docDto } from "../test/fixtures";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetInsertSilenceForTest();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetSelectionForTest();
  resetTransportForTest();
});

async function openFixtureDocument(sampleRateHz = 48_000): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return docDto({ sample_rate_hz: sampleRateHz });
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

describe("insertSilence store (H-56, SPEC-008 §2.5)", () => {
  it("canInsertSilence requires an open document", () => {
    expect(canInsertSilence()).toBe(false);
  });

  it("opens with the default 1.000 s, and remembers the last accepted value for the session", async () => {
    await openFixtureDocument();
    expect(canInsertSilence()).toBe(true);

    openInsertSilenceDialog();
    expect(insertSilenceState().dialogOpen).toBe(true);
    expect(insertSilenceState().dialogText).toBe("1.000");
    expect(insertSilenceState().dialogValid).toBe(true);
    expect(insertSilenceState().lengthLabel).toBe("48000 samples at 48 kHz");
    closeInsertSilenceDialog();

    setInsertSilenceDialogText("2");
    closeInsertSilenceDialog(); // Cancel: never applied, so nothing is remembered.
    openInsertSilenceDialog();
    expect(insertSilenceState().dialogText).toBe("1.000");
    closeInsertSilenceDialog();
  });

  it("OK is disabled for out-of-range or unparseable text", async () => {
    await openFixtureDocument();
    openInsertSilenceDialog();

    setInsertSilenceDialogText("0");
    expect(insertSilenceState().dialogValid).toBe(false);

    setInsertSilenceDialogText("3600.001");
    expect(insertSilenceState().dialogValid).toBe(false);

    setInsertSilenceDialogText("abc");
    expect(insertSilenceState().dialogValid).toBe(false);

    setInsertSilenceDialogText("3600");
    expect(insertSilenceState().dialogValid).toBe(true);
  });

  it("applies at the cursor with no selection, remembering the accepted text", async () => {
    await openFixtureDocument();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_insert_silence") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000 + 48_000,
          selection: [0, 48_000],
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    openInsertSilenceDialog();
    await applyInsertSilenceDialog();

    expect(calls).toEqual([{ target: { kind: "cursor", at_samples: 0 }, lenSamples: 48_000 }]);
    expect(insertSilenceState().dialogOpen).toBe(false);

    // The accepted value is now remembered for the rest of the session.
    openInsertSilenceDialog();
    expect(insertSilenceState().dialogText).toBe("1.000");
  });

  it("applies at the selection start with a selection (never removes anything)", async () => {
    await openFixtureDocument();
    setSelectionFromResult([1_000, 5_000]);
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_insert_silence") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000 + 100,
          selection: [1_000, 1_100],
          playhead_samples: 1_000,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    openInsertSilenceDialog();
    setInsertSilenceDialogText("100 smp");
    await applyInsertSilenceDialog();

    expect(calls).toEqual([
      { target: { kind: "range", start_samples: 1_000, end_samples: 5_000 }, lenSamples: 100 },
    ]);
  });

  it("a no-op apply while invalid leaves the dialog open", async () => {
    await openFixtureDocument();
    openInsertSilenceDialog();
    setInsertSilenceDialogText("0");
    await applyInsertSilenceDialog();
    expect(insertSilenceState().dialogOpen).toBe(true);
  });

  // H-82 (SPEC-005 §2.3 item 4 / Amendment 1): the cursor/selection target would be the
  // *importing* file's, not this document's — the Edit menu/right-click menu already disable the
  // item, but a direct call (e.g. a bypassed disabled state) must stay a safe no-op too.
  it("H-82: openInsertSilenceDialog no-ops while an import is running, and works again once it ends", async () => {
    await openFixtureDocument();
    applyImportStarted({ job_id: 1, name: "new.wav", sample_rate_hz: 48_000, len_samples: 1_000 });

    openInsertSilenceDialog();
    expect(insertSilenceState().dialogOpen).toBe(false);

    applyImportJobProgress({ job_id: 1, kind: "import", state: "done", fraction: 1 });
    openInsertSilenceDialog();
    expect(insertSilenceState().dialogOpen).toBe(true);
  });
});
