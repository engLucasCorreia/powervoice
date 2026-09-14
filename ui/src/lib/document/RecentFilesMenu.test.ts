import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RecentFileDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { resetDocumentStateForTest } from "./document.svelte";
import RecentFilesMenu from "./RecentFilesMenu.svelte";
import { resetRecentFilesForTest } from "./recentFiles.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecentFilesForTest();
});

function mountMenu(): { target: HTMLDivElement; app: unknown } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RecentFilesMenu, { target });
  flushSync();
  return { target, app };
}

describe("RecentFilesMenu (T-306, SPEC-018 §2.12)", () => {
  it("opening the dropdown refetches and shows entries, missing ones greyed", async () => {
    const entries: RecentFileDto[] = [
      { path: "/vo/A.wav", name: "A.wav", folder: "/vo", exists: true },
      { path: "/vo/gone.wav", name: "gone.wav", folder: "/vo", exists: false },
    ];
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return entries;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    const rows = target.querySelectorAll('[data-testid="recent-entry"]');
    expect(rows).toHaveLength(2);
    expect(rows[1]?.className).toContain("missing");
    expect(target.textContent).toContain("(missing)");

    unmount(app as Parameters<typeof unmount>[0]);
    target.remove();
  });

  it("picking an existing entry opens it via document_open", async () => {
    const entries: RecentFileDto[] = [
      { path: "/vo/A.wav", name: "A.wav", folder: "/vo", exists: true },
    ];
    let openedPath: string | null = null;
    mockIPC((cmd, args) => {
      if (cmd === "recent_files_get") {
        return entries;
      }
      if (cmd === "document_open") {
        openedPath = (args as { path: string }).path;
        return {
          name: "A.wav",
          path: "/vo/A.wav",
          sample_rate_hz: 48_000,
          len_samples: 1,
          dirty: false,
          audio_rev: 1,
          sidecar_dirty: false,
          spectral_view: null, waveform_view: null,
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-open"]')!.click();
    await new Promise((r) => setTimeout(r, 0));
    await Promise.resolve();

    expect(openedPath).toBe("/vo/A.wav");

    unmount(app as Parameters<typeof unmount>[0]);
    target.remove();
  });

  it("clicking a missing entry never calls document_open", async () => {
    const entries: RecentFileDto[] = [
      { path: "/vo/gone.wav", name: "gone.wav", folder: "/vo", exists: false },
    ];
    let openCalled = false;
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return entries;
      }
      openCalled = true;
      throw new Error(`unexpected command: ${cmd}`);
    });

    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-open"]')!.click();
    await Promise.resolve();
    expect(openCalled).toBe(false);

    unmount(app as Parameters<typeof unmount>[0]);
    target.remove();
  });

  it("Remove drops one entry via recent_files_remove", async () => {
    const entries: RecentFileDto[] = [
      { path: "/vo/A.wav", name: "A.wav", folder: "/vo", exists: true },
    ];
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return entries;
      }
      if (cmd === "recent_files_remove") {
        return [];
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="recent-entry-remove"]')!.click();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    expect(target.querySelectorAll('[data-testid="recent-entry"]')).toHaveLength(0);

    unmount(app as Parameters<typeof unmount>[0]);
    target.remove();
  });

  it("Clear Recent Files empties the list via recent_files_clear", async () => {
    const entries: RecentFileDto[] = [
      { path: "/vo/A.wav", name: "A.wav", folder: "/vo", exists: true },
    ];
    let cleared = false;
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return entries;
      }
      if (cmd === "recent_files_clear") {
        cleared = true;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-open-recent"]')!.click();
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="menu-clear-recent"]')!.click();
    await new Promise((r) => setTimeout(r, 0));

    expect(cleared).toBe(true);

    unmount(app as Parameters<typeof unmount>[0]);
    target.remove();
  });
});
