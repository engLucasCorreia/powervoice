import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { EditResultDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { cut, resetEditForTest } from "../state/edit.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { clearSelection, resetSelectionForTest, selectAllOf } from "../state/selection.svelte";
import EditMenu from "./EditMenu.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetEditForTest();
  resetSelectionForTest();
  resetRecordForTest();
});

function mountMenu(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EditMenu, { target });
  flushSync();
  return { target, app };
}

describe("EditMenu (S2-01, SPEC-008 AC-8)", () => {
  it("disables Cut/Copy/Delete/Trim/Silence with no selection, and Paste with an empty clipboard", () => {
    clearSelection();
    const { target, app } = mountMenu();

    for (const id of ["menu-cut", "menu-copy", "menu-delete", "menu-trim", "menu-silence"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        true,
      );
    }
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-paste"]')?.disabled).toBe(
      true,
    );
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-undo"]')?.disabled).toBe(
      true,
    );
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-redo"]')?.disabled).toBe(
      true,
    );
    expect(target.querySelector('[data-testid="menu-undo"]')?.textContent?.trim()).toBe("Undo");

    unmount(app);
    target.remove();
  });

  it("enables the selection-based commands once there's a non-empty selection", () => {
    selectAllOf(1_000);
    const { target, app } = mountMenu();

    for (const id of ["menu-cut", "menu-copy", "menu-delete", "menu-trim", "menu-silence"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        false,
      );
    }

    unmount(app);
    target.remove();
  });

  it("Cut calls edit_cut with the selection's exact range and applies the result", async () => {
    selectAllOf(1_000);
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_cut") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 0,
          selection: null,
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await cut();
    const { target, app } = mountMenu();

    expect(calls).toEqual([{ startSamples: 0, endSamples: 1_000 }]);
    // Cut clears the selection (SPEC-008 §2.3) — Cut/Copy/etc. are disabled again.
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-cut"]')?.disabled).toBe(
      true,
    );

    unmount(app);
    target.remove();
  });
});
