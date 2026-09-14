import { listen } from "@tauri-apps/api/event";
import type { EventName, RecentFileDto } from "../ipc/bindings";
import { recentFilesClear, recentFilesGet, recentFilesRemove } from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { openDocument, withUnsavedChangesGuard } from "./document.svelte";

/**
 * Recent files store (T-306, SPEC-018 §2.12): `File → Open Recent`. Loads the list once
 * (`initRecentFiles`) and keeps it fresh via `recent_files_changed` — `document_open`/
 * `document_save_as` already touch the list server-side, so this store only ever reads it.
 */

let entries = $state<RecentFileDto[]>([]);

export function recentFilesState(): { readonly entries: readonly RecentFileDto[] } {
  return {
    get entries() {
      return entries;
    },
  };
}

function isIpcError(value: unknown): value is { code: string; key: string } {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err as Parameters<typeof noticeFromIpcError>[0]));
  }
}

/** Refetches the list (call when the File menu opens — existence is re-checked then, SPEC-018
 * §2.12). */
export async function refreshRecentFiles(): Promise<void> {
  try {
    entries = await recentFilesGet();
  } catch (err) {
    report(err);
  }
}

/** Picking an entry: the normal Open flow (unsaved-changes prompt included). A missing entry is
 * the caller's job to catch before calling this (`RecentFilesMenu` checks `exists` first). */
export async function openRecentFile(path: string): Promise<void> {
  await withUnsavedChangesGuard(async () => {
    await openDocument(path);
  });
}

/** The File menu's "Remove" action on one entry (missing or not). */
export async function removeRecentFile(path: string): Promise<void> {
  try {
    entries = await recentFilesRemove(path);
  } catch (err) {
    report(err);
  }
}

/** "Clear Recent Files" — no confirmation (SPEC-018 §2.12: it deletes no user data). */
export async function clearRecentFiles(): Promise<void> {
  try {
    await recentFilesClear();
    entries = [];
  } catch (err) {
    report(err);
  }
}

/** Wires the `recent_files_changed` event and loads the initial list. Returns the teardown. */
export async function initRecentFiles(): Promise<() => void> {
  await refreshRecentFiles();
  try {
    const unlisten = await listen<RecentFileDto[]>(
      "recent_files_changed" satisfies EventName,
      (e) => {
        entries = e.payload;
      },
    );
    return unlisten;
  } catch {
    // Not running inside a real Tauri window — the list still follows explicit refetches.
    return () => {};
  }
}

/** Test/teardown helper. */
export function resetRecentFilesForTest(): void {
  entries = [];
}
