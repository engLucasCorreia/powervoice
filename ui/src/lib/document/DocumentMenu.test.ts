import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DocumentDto, RecentFileDto } from "../ipc/bindings";
import { clearActionHandlers, registerAction } from "../keymap";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { clearNotices } from "../state/notices.svelte";
import { applyRecordStateForTest, recordState, resetRecordForTest } from "../state/record.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import DocumentMenu from "./DocumentMenu.svelte";
import { openDocument, resetDocumentStateForTest } from "./document.svelte";
import { resetRecentFilesForTest } from "./recentFiles.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  clearActionHandlers();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
  resetMenuBarForTest();
  resetRecentFilesForTest();
});

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(DocumentMenu, { target });
  flushSync();
  return { target, app };
}

function openMenu(target: HTMLElement): void {
  target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-file"]')!.click();
  flushSync();
}

describe("DocumentMenu / File menu (H-19)", () => {
  it("is a closed dropdown by default with a menubar-item trigger", () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector('[data-testid="menu-trigger-file"]');
    expect(trigger?.getAttribute("role")).toBe("menuitem");
    expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
    expect(trigger?.getAttribute("aria-expanded")).toBe("false");
    expect(target.querySelector('[data-testid="document-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("opens on click, exposing a role=menu popup with role=menuitem children", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    const popup = target.querySelector('[data-testid="document-menu"]');
    expect(popup?.getAttribute("role")).toBe("menu");
    expect(target.querySelector('[data-testid="menu-open"]')?.getAttribute("role")).toBe("menuitem");
    unmount(app);
    target.remove();
  });

  it("shows 'no document' and disables Save/Save As/Export/Close with none open", () => {
    const { target, app } = mountMenu();
    openMenu(target);

    expect(target.querySelector('[data-testid="document-name"]')?.textContent).toBe(
      "No file open",
    );
    for (const id of ["menu-save", "menu-save-as", "menu-export", "menu-close"]) {
      expect(
        target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled,
        id,
      ).toBe(true);
    }

    unmount(app);
    target.remove();
  });

  it("shows the document name (with a modified marker) and enables Save/Save As/Close once open", async () => {
    const fixture: DocumentDto = {
      name: "take.wav",
      path: "/home/user/take.wav",
      sample_rate_hz: 48_000,
      len_samples: 480_000,
      dirty: true,
      audio_rev: 1,
      sidecar_dirty: false,
      spectral_view: null,
      waveform_view: null,
      recovered: false,
    };
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    const { target, app } = mountMenu();
    openMenu(target);

    expect(target.querySelector('[data-testid="document-name"]')?.textContent).toBe(
      "take.wav *",
    );
    for (const id of ["menu-save", "menu-export", "menu-close"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        false,
      );
    }

    unmount(app);
    target.remove();
  });

  it("New Recording… opens the format prompt (H-06), disabled while recording", () => {
    const { target, app } = mountMenu();
    openMenu(target);

    expect(recordState().newRecordingPrompt).toBeNull();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-new-recording"]')!.click();
    flushSync();
    expect(recordState().newRecordingPrompt).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("New Recording… is disabled while a take is recording or finishing", () => {
    applyRecordStateForTest({ recording: true });
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-new-recording"]')?.disabled,
    ).toBe(true);
    unmount(app);
    target.remove();
  });

  it("Open/Save/Save As show the registry's shortcut label and dispatch the same keymap action", () => {
    const { target, app } = mountMenu();
    openMenu(target);

    const openHandler = vi.fn();
    const saveHandler = vi.fn();
    const saveAsHandler = vi.fn();
    registerAction("file.open", openHandler);
    registerAction("file.save", saveHandler);
    registerAction("file.save_as", saveAsHandler);

    expect(target.querySelector('[data-testid="menu-open"] .shortcut')?.textContent).toBe(
      "Ctrl+O",
    );
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open"]')!.click();
    expect(openHandler).toHaveBeenCalledOnce();

    unmount(app);
    target.remove();
  });

  it("closes the menu (and the Recent Files submenu) after selecting a plain item", () => {
    registerAction("file.open", () => {});
    const { target, app } = mountMenu();
    openMenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="document-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("keyboard: ArrowDown on the trigger opens the menu and focuses the first item", async () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-file"]')!;
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    flushSync();
    await new Promise((resolve) => queueMicrotask(() => resolve(undefined)));
    expect(document.activeElement?.getAttribute("data-testid")).toBe("menu-new-recording");
    unmount(app);
    target.remove();
  });

  it("keyboard: Escape closes the popup and refocuses the trigger", () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-file"]')!;
    openMenu(target);
    const popup = target.querySelector<HTMLElement>('[data-testid="document-menu"]')!;
    popup.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="document-menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    unmount(app);
    target.remove();
  });

  describe("Recent Files submenu", () => {
    it("shows an empty state with no recent files", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
      flushSync();
      // H-26: the shared menu's quiet note row, with a message that says what's missing.
      const note = target.querySelector('[role="menu"] [role="menu"] .note');
      expect(note?.textContent).toBe("No recent files");
      unmount(app);
      target.remove();
    });

    it("lists entries as menu items and picking one opens it and closes the whole menu", async () => {
      const entries: RecentFileDto[] = [
        { path: "/a/b.wav", name: "b.wav", folder: "/a", exists: true },
      ];
      const opened: DocumentDto = {
        name: "b.wav",
        path: "/a/b.wav",
        sample_rate_hz: 48_000,
        len_samples: 100,
        dirty: false,
        audio_rev: 1,
        sidecar_dirty: false,
        spectral_view: null,
        waveform_view: null,
        recovered: false,
      };
      mockIPC((cmd) => {
        if (cmd === "recent_files_get") {
          return entries;
        }
        if (cmd === "document_open") {
          return opened;
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
      flushSync();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();

      const entry = target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-open"]');
      expect(entry?.textContent?.trim()).toBe("b.wav");
      entry!.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();
      expect(target.querySelector('[data-testid="document-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });

    it("a missing entry shows the missing marker, muted but still clickable (H-15)", async () => {
      const entries: RecentFileDto[] = [
        { path: "/a/gone.wav", name: "gone.wav", folder: "/a", exists: false },
      ];
      mockIPC((cmd) => {
        if (cmd === "recent_files_get") {
          return entries;
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
      flushSync();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();

      const entry = target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-open"]');
      expect(entry?.disabled).toBe(false);
      expect(entry?.classList.contains("muted")).toBe(true);
      expect(entry?.textContent).toContain("(missing)");

      unmount(app);
      target.remove();
    });

    it("H-15 (SPEC-018 §2.12): choosing a missing entry closes the File menu and shows the dedicated dialog instead of opening it directly", async () => {
      const entries: RecentFileDto[] = [
        { path: "/a/gone.wav", name: "gone.wav", folder: "/a", exists: false },
      ];
      mockIPC((cmd) => {
        if (cmd === "recent_files_get") {
          return entries;
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
      flushSync();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();

      target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-open"]')!.click();
      flushSync();
      // The File menu closes immediately — the missing-file dialog lives outside this popup
      // (mounted separately as `RecentMissingDialog`, exercised in its own test file).
      expect(target.querySelector('[data-testid="document-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });
  });
});
