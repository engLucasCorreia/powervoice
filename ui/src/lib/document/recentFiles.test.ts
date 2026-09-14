import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { RecentFileDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { resetDocumentStateForTest } from "./document.svelte";
import {
  clearRecentFiles,
  recentFilesState,
  refreshRecentFiles,
  removeRecentFile,
  resetRecentFilesForTest,
} from "./recentFiles.svelte";

function entry(path: string, exists: boolean | null = true): RecentFileDto {
  const name = path.split("/").pop() ?? path;
  const folder = path.slice(0, path.length - name.length - 1);
  return { path, name, folder, exists };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetRecentFilesForTest();
});

describe("recentFiles store (T-306, SPEC-018 §2.12)", () => {
  it("refreshRecentFiles loads the list from recent_files_get", async () => {
    const fixture = [entry("/vo/B.wav"), entry("/vo/A.wav")];
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await refreshRecentFiles();
    expect(recentFilesState().entries).toEqual(fixture);
  });

  it("removeRecentFile calls recent_files_remove and applies the returned list", async () => {
    const remaining = [entry("/vo/B.wav")];
    let received: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "recent_files_remove") {
        received = args;
        return remaining;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await removeRecentFile("/vo/A.wav");
    expect(received).toEqual({ path: "/vo/A.wav" });
    expect(recentFilesState().entries).toEqual(remaining);
  });

  it("clearRecentFiles empties the list", async () => {
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return [entry("/vo/A.wav")];
      }
      if (cmd === "recent_files_clear") {
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await refreshRecentFiles();
    expect(recentFilesState().entries).toHaveLength(1);
    await clearRecentFiles();
    expect(recentFilesState().entries).toEqual([]);
  });

  it("a missing entry is reported with exists: false", async () => {
    mockIPC((cmd) => {
      if (cmd === "recent_files_get") {
        return [entry("/vo/gone.wav", false)];
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await refreshRecentFiles();
    expect(recentFilesState().entries[0]?.exists).toBe(false);
  });
});
