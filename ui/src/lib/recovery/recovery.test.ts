import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, RecoverableSessionDto } from "../ipc/bindings";
import { documentState, resetDocumentStateForTest, titleFor } from "../document/document.svelte";
import { clearNotices, noticesState } from "../state/notices.svelte";
import RecoveryDialog from "./RecoveryDialog.svelte";
import { formatBytes, formatDuration } from "./format";
import { initRecovery, openRecoveryStorage, resetRecoveryForTest } from "./recovery.svelte";

const SESSION: RecoverableSessionDto = {
  id: "1789-42-0",
  name: "voice.wav",
  path: "/home/user/voice.wav",
  last_modified_unix_ms: 1_789_000_000_000,
  unsaved_changes: 3,
  recording_samples: 120_000,
  sample_rate_hz: 48_000,
  source_changed: true,
  source_missing: false,
  damaged: false,
  size_bytes: 150_000_000,
};

const RECOVERED: DocumentDto = {
  name: "voice.wav",
  path: "/home/user/voice.wav",
  sample_rate_hz: 48_000,
  len_samples: 480_000,
  dirty: true,
  audio_rev: 4,
  waveform_view: null,
  sidecar_dirty: false,
  spectral_view: null,
  recovered: true,
};

afterEach(() => {
  clearMocks();
  clearNotices();
  resetRecoveryForTest();
  resetDocumentStateForTest();
});

function mountDialog(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RecoveryDialog, { target });
  flushSync();
  return { target, app };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}

function q<T extends Element = HTMLElement>(root: HTMLElement, id: string): T | null {
  return root.querySelector<T>(`[data-testid="${id}"]`);
}

describe("recovery dialog (T-301, SPEC-004 §2.7, AC-10)", () => {
  it("stays hidden when nothing is recoverable", async () => {
    mockIPC((cmd) => (cmd === "recovery_list" ? [] : null));
    await initRecovery();
    const { target, app } = mountDialog();
    expect(q(target, "recovery-dialog")).toBeNull();
    unmount(app);
    target.remove();
  });

  it("lists the file name, path, unsaved changes, the recording length and the changed-on-disk warning", async () => {
    mockIPC((cmd) => (cmd === "recovery_list" ? [SESSION] : null));
    await initRecovery();
    const { target, app } = mountDialog();
    const dialog = q(target, "recovery-dialog")!;
    expect(dialog.dataset.mode).toBe("startup");
    expect(dialog.textContent).toContain("didn't shut down properly");
    expect(dialog.textContent).toContain("voice.wav");
    expect(dialog.textContent).toContain("/home/user/voice.wav");
    expect(q(target, "recovery-unsaved")?.textContent).toBe("3 unsaved changes");
    expect(q(target, "recovery-recording")?.textContent).toContain("0:02.5");
    expect(q(target, "recovery-changed")?.textContent).toContain("changed on disk");
    expect(q<HTMLSelectElement>(target, "recovery-take-action")?.value).toBe("apply");
    unmount(app);
    target.remove();
  });

  it("Recover sends the take choice and opens the document modified and titled (recovered)", async () => {
    let args: Record<string, unknown> | undefined;
    mockIPC((cmd, payload) => {
      if (cmd === "recovery_list") return [SESSION];
      if (cmd === "recovery_recover") {
        args = payload as Record<string, unknown>;
        return { document: RECOVERED, lost_changes: 0 };
      }
      return null;
    });
    await initRecovery();
    const { target, app } = mountDialog();
    const select = q<HTMLSelectElement>(target, "recovery-take-action")!;
    select.value = "new_document";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    q<HTMLButtonElement>(target, "recovery-recover")!.click();
    await settle();
    expect(args).toMatchObject({ id: SESSION.id, takeAction: "new_document" });
    expect(documentState().current.recovered).toBe(true);
    expect(titleFor(documentState().current)).toBe("voice.wav (recovered) * — PowerVoice");
    expect(q(target, "recovery-dialog")).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Decide later closes the dialog without recovering or deleting anything", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return cmd === "recovery_list" ? [SESSION] : null;
    });
    await initRecovery();
    const { target, app } = mountDialog();
    q<HTMLButtonElement>(target, "recovery-decide-later")!.click();
    flushSync();
    expect(q(target, "recovery-dialog")).toBeNull();
    expect(calls).toEqual(["recovery_list"]);
    unmount(app);
    target.remove();
  });

  it("Discard asks for confirmation and deletes only the confirmed session", async () => {
    const discarded: string[] = [];
    mockIPC((cmd, payload) => {
      if (cmd === "recovery_list") return [SESSION, { ...SESSION, id: "other", name: null, path: null }];
      if (cmd === "recovery_discard") {
        discarded.push((payload as { id: string }).id);
        return [{ ...SESSION, id: "other", name: null, path: null }];
      }
      return null;
    });
    await initRecovery();
    const { target, app } = mountDialog();
    q<HTMLButtonElement>(target, "recovery-discard")!.click();
    flushSync();
    expect(q(target, "recovery-discard-confirm")?.textContent).toContain(
      "Permanently delete the unsaved changes to voice.wav?",
    );
    q<HTMLButtonElement>(target, "recovery-discard-cancel")!.click();
    flushSync();
    expect(discarded).toEqual([]);

    q<HTMLButtonElement>(target, "recovery-discard")!.click();
    flushSync();
    q<HTMLButtonElement>(target, "recovery-discard-proceed")!.click();
    await settle();
    expect(discarded).toEqual([SESSION.id]);
    const rows = target.querySelectorAll('[data-testid="recovery-session"]');
    expect(rows).toHaveLength(1);
    expect(rows[0]?.textContent).toContain("Untitled recording");
    unmount(app);
    target.remove();
  });

  it("a session with nothing intact offers only Discard", async () => {
    mockIPC((cmd) => {
      if (cmd === "recovery_list") return [SESSION];
      if (cmd === "recovery_recover") {
        throw { code: "internal", key: "error.recovery.nothing_intact", params: {} };
      }
      return null;
    });
    await initRecovery();
    const { target, app } = mountDialog();
    q<HTMLButtonElement>(target, "recovery-recover")!.click();
    await settle();
    expect(q<HTMLButtonElement>(target, "recovery-recover")?.disabled).toBe(true);
    expect(q(target, "recovery-unrecoverable")).not.toBeNull();
    expect(noticesState().toasts.length + noticesState().banners.length).toBeGreaterThan(0);
    unmount(app);
    target.remove();
  });

  it("File → Recovery & Storage shows the session storage line", async () => {
    mockIPC((cmd) =>
      cmd === "storage_info"
        ? {
            session_bytes: 3_200_000_000,
            history_bytes: 2_100_000_000,
            sessions: [SESSION],
            recovery_bytes: SESSION.size_bytes,
          }
        : null,
    );
    await openRecoveryStorage();
    const { target, app } = mountDialog();
    expect(q(target, "recovery-storage")?.textContent).toBe(
      "Session storage: 3.2 GB (history 2.1 GB)",
    );
    expect(q(target, "recovery-close")).not.toBeNull();
    unmount(app);
    target.remove();
  });
});

describe("recovery formatting", () => {
  it("formats the recording length as m:ss.s", () => {
    expect(formatDuration(120_000, 48_000)).toBe("0:02.5");
    expect(formatDuration(48_000 * 151, 48_000)).toBe("2:31.0");
    expect(formatDuration(48_000 * 60 - 1, 48_000)).toBe("1:00.0");
  });

  it("formats sizes in decimal units", () => {
    expect(formatBytes(3_200_000_000)).toBe("3.2 GB");
    expect(formatBytes(150_000_000)).toBe("150 MB");
  });
});
