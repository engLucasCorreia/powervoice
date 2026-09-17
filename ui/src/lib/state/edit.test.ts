import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { ClipboardChangedDto, EditResultDto } from "../ipc/bindings";
import { clearActionHandlers } from "../shortcuts";
import {
  applyImportJobProgress,
  applyImportStarted,
  resetDocumentStateForTest,
} from "../document/document.svelte";
import { clearNotices } from "./notices.svelte";
import {
  cancelPasteJob,
  copy,
  cut,
  deleteSelection,
  dismissPasteJob,
  editState,
  initEdit,
  paste,
  resetEditForTest,
  silence,
  trim,
} from "./edit.svelte";
import { resetSelectionForTest, selectAllOf, selectionState } from "./selection.svelte";
import { historyStateDto } from "../test/fixtures";
import { resetTransportForTest } from "./transport.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetEditForTest();
  resetSelectionForTest();
  resetTransportForTest();
  resetDocumentStateForTest();
});

describe("edit store (S2-01)", () => {
  it("applies history_state and clipboard_changed events", async () => {
    mockIPC(() => null, { shouldMockEvents: true });
    const stop = await initEdit();

    const history = historyStateDto({ can_undo: true, undo_label: "history.cut" });
    await emit("history_state", history);
    expect(editState().history).toEqual(history);

    const clipboard: ClipboardChangedDto = { len_samples: 4_000, sample_rate_hz: 48_000 };
    await emit("clipboard_changed", clipboard);
    expect(editState().clipboard).toEqual(clipboard);

    stop();
  });

  it("paste targets the cursor with no selection, and the selection when one exists", async () => {
    const calls: unknown[] = [];
    mockIPC(
      (cmd, args) => {
        if (cmd === "edit_paste") {
          calls.push(args);
          return {
            changed: true,
            audio_rev: 1,
            len_samples: 100,
            selection: [0, 100],
            playhead_samples: 0,
          } satisfies EditResultDto;
        }
        throw new Error(`unmocked command: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const stop = await initEdit();

    // No selection: a no-op with an empty clipboard (never calls the command).
    await paste();
    expect(calls).toHaveLength(0);

    const clipboard: ClipboardChangedDto = { len_samples: 10, sample_rate_hz: 48_000 };
    await emit("clipboard_changed", clipboard);

    await paste();
    expect(calls).toEqual([{ target: { kind: "cursor", at_samples: 0 } }]);
    // The command's result becomes the selection (SPEC-008 §2.3).
    expect(selectionState().current).toEqual({ startSample: 0, endSample: 100 });

    selectAllOf(500);
    await paste();
    expect(calls[1]).toEqual({ target: { kind: "range", start_samples: 0, end_samples: 500 } });

    stop();
  });

  it("H-56: tracks a cross-document paste job's progress via job_progress (kind paste)", async () => {
    const cancelCalls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_paste_cancel") {
        cancelCalls.push(args);
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    }, { shouldMockEvents: true });
    const stop = await initEdit();

    expect(editState().pasteJob).toBeNull();

    await emit("job_progress", { job_id: 7, kind: "import", state: "running", fraction: 0.5 });
    expect(editState().pasteJob).toBeNull(); // a different job kind is ignored

    await emit("job_progress", { job_id: 7, kind: "paste", state: "running", fraction: 0.25 });
    expect(editState().pasteJob).toEqual({ jobId: 7, fraction: 0.25, state: "running" });

    await emit("job_progress", { job_id: 7, kind: "paste", state: "running", fraction: 0.75 });
    expect(editState().pasteJob).toEqual({ jobId: 7, fraction: 0.75, state: "running" });

    cancelPasteJob();
    expect(cancelCalls).toEqual([{ jobId: 7 }]);

    await emit("job_progress", { job_id: 7, kind: "paste", state: "cancelled", fraction: 0 });
    expect(editState().pasteJob?.state).toBe("cancelled");

    dismissPasteJob();
    expect(editState().pasteJob).toBeNull();

    stop();
  });

  // H-82 (SPEC-005 §2.3 item 4 / Amendment 1): a selection/cursor made while an import is running
  // describes a position in the *importing* file, not this (previous) document — so Cut, Copy,
  // Delete, Trim and Silence must no-op then, the same way they already no-op with no selection.
  it("H-82: Cut/Copy/Delete/Trim/Silence no-op while an import is running, and work again once it ends", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return {
        changed: true,
        audio_rev: 1,
        len_samples: 100,
        selection: null,
        playhead_samples: 0,
      } satisfies EditResultDto;
    });

    selectAllOf(500);
    applyImportStarted({ job_id: 1, name: "big.wav", sample_rate_hz: 48_000, len_samples: 480_000 });

    await cut();
    await copy();
    await deleteSelection();
    await trim();
    await silence();
    expect(calls).toHaveLength(0);

    applyImportJobProgress({ job_id: 1, kind: "import", state: "done", fraction: 1 });
    await cut();
    expect(calls).toEqual(["edit_cut"]);
  });

  it("H-82: Paste no-ops while an import is running, and works again once it ends", async () => {
    const calls: string[] = [];
    mockIPC(
      (cmd) => {
        calls.push(cmd);
        return {
          changed: true,
          audio_rev: 1,
          len_samples: 100,
          selection: [0, 100],
          playhead_samples: 0,
        } satisfies EditResultDto;
      },
      { shouldMockEvents: true },
    );
    const stop = await initEdit();
    await emit("clipboard_changed", { len_samples: 10, sample_rate_hz: 48_000 } satisfies ClipboardChangedDto);

    applyImportStarted({ job_id: 2, name: "big.wav", sample_rate_hz: 48_000, len_samples: 480_000 });
    await paste();
    expect(calls).toHaveLength(0);

    applyImportJobProgress({ job_id: 2, kind: "import", state: "done", fraction: 1 });
    await paste();
    expect(calls).toEqual(["edit_paste"]);

    stop();
  });
});
