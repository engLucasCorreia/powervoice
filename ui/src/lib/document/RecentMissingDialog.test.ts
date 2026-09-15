import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { resetDocumentStateForTest } from "./document.svelte";
import RecentMissingDialog from "./RecentMissingDialog.svelte";
import { pickRecentFile, resetRecentFilesForTest } from "./recentFiles.svelte";
import { docDto } from "../test/fixtures";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecentFilesForTest();
});

function mountDialog(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RecentMissingDialog, { target });
  flushSync();
  return { target, app };
}

describe("RecentMissingDialog (H-15, SPEC-018 §2.12 recent-files missing-file flow)", () => {
  it("is hidden with no pending prompt", () => {
    const { target, app } = mountDialog();
    expect(target.querySelector('[data-testid="recent-missing-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("choosing a missing entry shows \"‹path› can't be found.\" with Locate/Remove from List/Cancel", async () => {
    mockIPC((cmd) => {
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountDialog();
    const pending = pickRecentFile("/vo/gone.wav", false);
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    const dialog = target.querySelector('[data-testid="recent-missing-dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog?.textContent).toContain("gone.wav");
    expect(dialog?.textContent).toContain("can't be found");
    expect(target.querySelector('[data-testid="recent-missing-locate"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="recent-missing-remove"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="recent-missing-cancel"]')).not.toBeNull();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-missing-cancel"]')!.click();
    await pending;
    flushSync();
    expect(target.querySelector('[data-testid="recent-missing-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Remove from List calls recent_files_remove with the missing path and closes the dialog", async () => {
    let removedPath: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "recent_files_remove") {
        removedPath = (args as { path: string }).path;
        return [];
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountDialog();
    const pending = pickRecentFile("/vo/gone.wav", false);
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-missing-remove"]')!.click();
    await pending;
    expect(removedPath).toBe("/vo/gone.wav");
    flushSync();
    expect(target.querySelector('[data-testid="recent-missing-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Locate… opens the native picker, opens the picked file, and re-points the entry (drops the old one)", async () => {
    const removed: string[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|open") {
        return "/vo/found-elsewhere.wav";
      }
      if (cmd === "document_open") {
        expect((args as { path: string }).path).toBe("/vo/found-elsewhere.wav");
        return docDto({ name: "found-elsewhere.wav", path: "/vo/found-elsewhere.wav", len_samples: 1000 });
      }
      if (cmd === "recent_files_remove") {
        removed.push((args as { path: string }).path);
        return [];
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountDialog();
    const pending = pickRecentFile("/vo/gone.wav", false);
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-missing-locate"]')!.click();
    await pending;
    expect(removed).toEqual(["/vo/gone.wav"]);
    flushSync();
    expect(target.querySelector('[data-testid="recent-missing-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Locate… cancelled at the native picker leaves the recent list untouched", async () => {
    let removeCalled = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|open") {
        return null;
      }
      if (cmd === "recent_files_remove") {
        removeCalled = true;
        return [];
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountDialog();
    const pending = pickRecentFile("/vo/gone.wav", false);
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-missing-locate"]')!.click();
    await pending;
    expect(removeCalled).toBe(false);

    unmount(app);
    target.remove();
  });
});
