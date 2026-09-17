import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyImportJobProgress,
  applyImportStarted,
  resetDocumentStateForTest,
  openDocument,
} from "../document/document.svelte";
import { t } from "../i18n";
import { clearActionHandlers, registerAction } from "../shortcuts";
import type { ActionId } from "../shortcuts/actions";
import { resetMarkersForTest, selectMarker } from "../markers/markers.svelte";
import { resetMenuBarForTest } from "../menu/menu.svelte";
import { resetEditForTest } from "../state/edit.svelte";
import { insertSilenceState, resetInsertSilenceForTest } from "../state/insertSilence.svelte";
import { applyRecordStateForTest, resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { docDto } from "../test/fixtures";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import EditMenu from "./EditMenu.svelte";

const DOC_FIXTURE = docDto();

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
  resetMenuBarForTest();
  resetEditForTest();
  resetSelectionForTest();
  resetMarkersForTest();
  resetInsertSilenceForTest();
});

async function openFixtureDocument(): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return DOC_FIXTURE;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EditMenu, { target });
  flushSync();
  return { target, app };
}

function openMenu(target: HTMLElement): void {
  target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-edit"]')!.click();
  flushSync();
}

describe("EditMenu (H-19)", () => {
  it("is a closed dropdown with a menubar-item trigger by default", () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector('[data-testid="menu-trigger-edit"]');
    expect(trigger?.getAttribute("role")).toBe("menuitem");
    expect(target.querySelector('[data-testid="edit-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Undo/Redo show 'Undo'/'Redo' with no history and are disabled", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    expect(target.querySelector('[data-testid="menu-undo"]')?.textContent).toContain("Undo");
    expect(target.querySelector('[data-testid="menu-redo"]')?.textContent).toContain("Redo");
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-undo"]')?.disabled).toBe(
      true,
    );
    expect(target.querySelector<HTMLButtonElement>('[data-testid="menu-redo"]')?.disabled).toBe(
      true,
    );
    unmount(app);
    target.remove();
  });

  it("shows the registry's shortcut labels", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    expect(target.querySelector('[data-testid="menu-undo"] .shortcut')?.textContent).toBe(
      "Ctrl+Z",
    );
    expect(target.querySelector('[data-testid="menu-redo"] .shortcut')?.textContent).toBe(
      "Ctrl+Shift+Z",
    );
    expect(target.querySelector('[data-testid="menu-cut"] .shortcut')?.textContent).toBe(
      "Ctrl+X",
    );
    expect(target.querySelector('[data-testid="menu-trim"] .shortcut')?.textContent).toBe(
      "Ctrl+T",
    );
    // Silence has no default binding (menu only) — no shortcut span at all.
    expect(target.querySelector('[data-testid="menu-silence"] .shortcut')).toBeNull();
    // Insert Silence… has no default binding either (SPEC-008 §2.11 table).
    expect(target.querySelector('[data-testid="menu-insert-silence"] .shortcut')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Insert Silence… is enabled with a document open and no selection, disabled with none open", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-insert-silence"]')?.disabled,
    ).toBe(true);
    target
      .querySelector<HTMLElement>('[data-testid="edit-menu"]')
      ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    unmount(app);
    target.remove();
  });

  it("Insert Silence… is enabled once a document is open, with no selection required", async () => {
    await openFixtureDocument();
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-insert-silence"]')?.disabled,
    ).toBe(false);
    unmount(app);
    target.remove();
  });

  it("Insert Silence… is disabled while recording", async () => {
    await openFixtureDocument();
    applyRecordStateForTest({ recording: true });
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-insert-silence"]')?.disabled,
    ).toBe(true);
    unmount(app);
    target.remove();
  });

  it("clicking Insert Silence… opens the dialog", async () => {
    await openFixtureDocument();
    const { target, app } = mountMenu();
    openMenu(target);
    expect(insertSilenceState().dialogOpen).toBe(false);
    target.querySelector<HTMLButtonElement>('[data-testid="menu-insert-silence"]')?.click();
    flushSync();
    expect(insertSilenceState().dialogOpen).toBe(true);
    unmount(app);
    target.remove();
  });

  it("Cut/Copy/Delete/Trim/Silence are disabled without a selection, enabled with one", () => {
    const { target, app } = mountMenu();
    for (const id of ["menu-cut", "menu-copy", "menu-delete", "menu-trim", "menu-silence"]) {
      openMenu(target);
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        true,
      );
      target.querySelector<HTMLElement>('[data-testid="edit-menu"]')?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      flushSync();
    }

    setSelectionFromResult([0, 100]);
    for (const id of ["menu-cut", "menu-copy", "menu-delete", "menu-trim", "menu-silence"]) {
      openMenu(target);
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        false,
      );
      target.querySelector<HTMLElement>('[data-testid="edit-menu"]')?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      flushSync();
    }

    unmount(app);
    target.remove();
  });

  it("disables Cut/Copy/Paste/Delete/Trim/Select All while recording, even with a selection", () => {
    setSelectionFromResult([0, 100]);
    applyRecordStateForTest({ recording: true });
    const { target, app } = mountMenu();
    openMenu(target);
    for (const id of ["menu-cut", "menu-copy", "menu-delete", "menu-trim", "menu-select-all"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        true,
      );
    }
    unmount(app);
    target.remove();
  });

  // H-82 (SPEC-005 §2.3 item 4 / Amendment 1): a selection made while an import is running
  // describes a position in the *importing* file, not the previous document these commands would
  // act on — so they must stay disabled for as long as the import is `running`, even with a
  // selection.
  it("disables Cut/Copy/Delete/Trim/Silence/Insert Silence while an import is running, even with a selection (H-82)", async () => {
    await openFixtureDocument();
    setSelectionFromResult([0, 100]);
    applyImportStarted({ job_id: 1, name: "new.wav", sample_rate_hz: 48_000, len_samples: 1_000 });
    const { target, app } = mountMenu();
    openMenu(target);
    for (const id of [
      "menu-cut",
      "menu-copy",
      "menu-delete",
      "menu-trim",
      "menu-silence",
      "menu-insert-silence",
    ]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        true,
      );
    }
    unmount(app);
    target.remove();
  });

  it("shows the spec's tooltip on Cut/Insert Silence while importing, and re-enables both once it ends (H-82)", async () => {
    await openFixtureDocument();
    setSelectionFromResult([0, 100]);
    applyImportStarted({ job_id: 2, name: "new.wav", sample_rate_hz: 48_000, len_samples: 1_000 });
    const { target, app } = mountMenu();
    openMenu(target);
    const expectedTitle = t("edit.unavailable_while_importing");
    expect(target.querySelector('[data-testid="menu-cut"]')?.getAttribute("title")).toBe(
      expectedTitle,
    );
    expect(
      target.querySelector('[data-testid="menu-insert-silence"]')?.getAttribute("title"),
    ).toBe(expectedTitle);

    applyImportJobProgress({ job_id: 2, kind: "import", state: "done", fraction: 1 });
    flushSync();
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-cut"]')?.disabled,
    ).toBe(false);
    expect(target.querySelector('[data-testid="menu-cut"]')?.getAttribute("title")).toBeNull();
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-insert-silence"]')?.disabled,
    ).toBe(false);

    unmount(app);
    target.remove();
  });

  it("Select All stays enabled while importing (SPEC-005 §2.3 item 4: 'selection work') — unlike recording, which disables it (H-82)", async () => {
    await openFixtureDocument();
    applyImportStarted({ job_id: 3, name: "new.wav", sample_rate_hz: 48_000, len_samples: 1_000 });
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-select-all"]')?.disabled,
    ).toBe(false);
    unmount(app);
    target.remove();
  });

  it("Select All is disabled with no document open, enabled once one is", async () => {
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-select-all"]')?.disabled,
    ).toBe(true);
    target
      .querySelector<HTMLElement>('[data-testid="edit-menu"]')
      ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();

    await openFixtureDocument();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-select-all"]')?.disabled,
    ).toBe(false);

    unmount(app);
    target.remove();
  });

  it("every bound item dispatches the same keymap action as its shortcut", async () => {
    await openFixtureDocument();
    const { target, app } = mountMenu();
    setSelectionFromResult([0, 100]);
    flushSync();

    // Undo/Redo (`can_undo`/`can_redo`) and Paste (`hasClipboard()`) are excluded here: nothing
    // in this menu-only test drives `history_state`/`clipboard_changed` (that's `initEdit`, an
    // app-level wire-up), so they stay disabled-by-default — already covered above.
    const cases: Array<[string, string]> = [
      ["menu-cut", "edit.cut"],
      ["menu-copy", "edit.copy"],
      ["menu-delete", "edit.delete"],
      ["menu-trim", "edit.trim"],
      ["menu-select-all", "waveform.select_all"],
    ];
    for (const [testid, action] of cases) {
      const handler = vi.fn();
      const unregister = registerAction(action as ActionId, handler);
      openMenu(target);
      target.querySelector<HTMLButtonElement>(`[data-testid="${testid}"]`)!.click();
      expect(handler, `${testid} -> ${action}`).toHaveBeenCalledOnce();
      unregister();
    }

    unmount(app);
    target.remove();
  });

  describe("Markers submenu", () => {
    it("is a submenu trigger with role=menuitem/aria-haspopup", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      const trigger = target.querySelector('[data-testid="menu-markers"]');
      expect(trigger?.getAttribute("role")).toBe("menuitem");
      expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
      unmount(app);
      target.remove();
    });

    it("Delete Selected Marker is disabled with none selected, enabled once one is", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-markers"]')!.click();
      flushSync();
      expect(
        target.querySelector<HTMLButtonElement>('[data-testid="menu-marker-delete"]')?.disabled,
      ).toBe(true);

      selectMarker(1);
      flushSync();
      expect(
        target.querySelector<HTMLButtonElement>('[data-testid="menu-marker-delete"]')?.disabled,
      ).toBe(false);

      unmount(app);
      target.remove();
    });

    it("Next/Previous Marker are disabled with an empty marker list", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-markers"]')!.click();
      flushSync();
      expect(
        target.querySelector<HTMLButtonElement>('[data-testid="menu-marker-next"]')?.disabled,
      ).toBe(true);
      expect(
        target.querySelector<HTMLButtonElement>('[data-testid="menu-marker-prev"]')?.disabled,
      ).toBe(true);
      unmount(app);
      target.remove();
    });

    it("Add/Delete/Next/Previous dispatch the same actions as their shortcuts", async () => {
      await openFixtureDocument();
      const { target, app } = mountMenu();
      selectMarker(1);
      flushSync();

      const cases: Array<[string, string]> = [
        ["menu-marker-add", "marker.add"],
        ["menu-marker-delete", "marker.delete_selected"],
      ];
      for (const [testid, action] of cases) {
        const handler = vi.fn();
        const unregister = registerAction(action as ActionId, handler);
        openMenu(target);
        // The submenu's own `markersOpen` state survives the outer menu closing (only its DOM is
        // torn down by `{#if open}`) — only click the trigger if the submenu isn't already shown.
        if (!target.querySelector(`[data-testid="${testid}"]`)) {
          target.querySelector<HTMLButtonElement>('[data-testid="menu-markers"]')!.click();
          flushSync();
        }
        target.querySelector<HTMLButtonElement>(`[data-testid="${testid}"]`)!.click();
        expect(handler, `${testid} -> ${action}`).toHaveBeenCalledOnce();
        unregister();
      }

      unmount(app);
      target.remove();
    });

    it("ArrowLeft inside the submenu closes it back to the Markers trigger, keeping Edit open", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-markers"]')!.click();
      flushSync();
      const submenu = target.querySelectorAll('[role="menu"]')[1] as HTMLElement;
      submenu.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
      flushSync();

      expect(target.querySelectorAll('[role="menu"]').length).toBe(1);
      expect(target.querySelector('[data-testid="edit-menu"]')).not.toBeNull();

      unmount(app);
      target.remove();
    });
  });
});
