import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { ClipboardChangedDto, EditResultDto } from "../ipc/bindings";
import { clearActionHandlers } from "../keymap";
import { clearNotices } from "./notices.svelte";
import { editState, initEdit, paste, resetEditForTest } from "./edit.svelte";
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
});
