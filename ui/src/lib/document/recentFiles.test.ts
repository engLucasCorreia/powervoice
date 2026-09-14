import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { RecentFileDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { resetDocumentStateForTest } from "./document.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import {
  clearRecentFiles,
  pickRecentFile,
  recentFilesState,
  refreshRecentFiles,
  removeRecentFile,
  resetRecentFilesForTest,
  resolveRecentMissingPrompt,
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
  resetWaveformViewForTest();
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

  describe("pickRecentFile (H-15, SPEC-018 §2.12 recent-files missing-file flow)", () => {
    it("an entry that exists goes straight through the normal open flow, no dialog", async () => {
      let openedPath: string | null = null;
      mockIPC((cmd, args) => {
        if (cmd === "document_open") {
          openedPath = (args as { path: string }).path;
          return {
            name: "A.wav",
            path: "/vo/A.wav",
            sample_rate_hz: 48_000,
            len_samples: 100,
            dirty: false,
            audio_rev: 1,
            sidecar_dirty: false,
            spectral_view: null,
            waveform_view: null,
            recovered: false,
          };
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      await pickRecentFile("/vo/A.wav", true);
      expect(openedPath).toBe("/vo/A.wav");
      expect(recentFilesState().missingPrompt).toBeNull();
    });

    it("an entry with exists: null (unresolved existence check) also opens directly", async () => {
      let openedPath: string | null = null;
      mockIPC((cmd, args) => {
        if (cmd === "document_open") {
          openedPath = (args as { path: string }).path;
          return {
            name: "A.wav",
            path: "/vo/A.wav",
            sample_rate_hz: 48_000,
            len_samples: 100,
            dirty: false,
            audio_rev: 1,
            sidecar_dirty: false,
            spectral_view: null,
            waveform_view: null,
            recovered: false,
          };
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      await pickRecentFile("/vo/A.wav", null);
      expect(openedPath).toBe("/vo/A.wav");
    });

    it("a missing entry sets the missingPrompt instead of opening, until resolved", async () => {
      mockIPC((cmd) => {
        throw new Error(`unmocked command: ${cmd}`);
      });
      const pending = pickRecentFile("/vo/gone.wav", false);
      await new Promise((r) => setTimeout(r, 0));
      expect(recentFilesState().missingPrompt).toEqual({ path: "/vo/gone.wav", name: "gone.wav" });

      resolveRecentMissingPrompt("cancel");
      await pending;
      expect(recentFilesState().missingPrompt).toBeNull();
    });
  });
});
