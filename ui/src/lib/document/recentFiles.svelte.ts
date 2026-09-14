import { listen } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import type { EventName, RecentFileDto } from "../ipc/bindings";
import { recentFilesClear, recentFilesGet, recentFilesRemove } from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { openDocument, withUnsavedChangesGuard } from "./document.svelte";

/**
 * Recent files store (T-306, SPEC-018 §2.12): `File → Open Recent`. Loads the list once
 * (`initRecentFiles`) and keeps it fresh via `recent_files_changed` — `document_open`/
 * `document_save_as` already touch the list server-side, so this store only ever reads it.
 *
 * H-15: picking a *missing* entry (`exists === false`) no longer falls through to the normal
 * open flow's error toast — it shows the dedicated `RecentMissingDialog`
 * (Locate…/Remove from List/Cancel, SPEC-018 §2.12/§4.7 `dialog.recent_missing.*`).
 */

/** Every format `File → Open`/Locate accepts (T-202, SPEC-005 §2.2), mirrored from
 * `document.svelte.ts`'s private `OPEN_FILTERS` (kept separate: that module isn't recent-files'
 * to reach into, and the array is one line). */
const OPEN_FILTERS = [{ name: "Audio", extensions: ["wav", "flac", "mp3", "m4a", "ogg"] }];

export type RecentMissingDecision = "locate" | "remove" | "cancel";

/** The missing-file dialog's data. */
export interface RecentMissingPrompt {
  path: string;
  name: string;
}

interface PendingRecentMissingPrompt extends RecentMissingPrompt {
  resolve: (decision: RecentMissingDecision) => void;
}

let entries = $state<RecentFileDto[]>([]);
let missingPrompt = $state<PendingRecentMissingPrompt | null>(null);

export function recentFilesState(): {
  readonly entries: readonly RecentFileDto[];
  readonly missingPrompt: RecentMissingPrompt | null;
} {
  return {
    get entries() {
      return entries;
    },
    get missingPrompt() {
      return missingPrompt ? { path: missingPrompt.path, name: missingPrompt.name } : null;
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

/** Picking an entry known to exist: the normal Open flow (unsaved-changes prompt included).
 * Returns whether it actually proceeded (a cancelled unsaved-changes prompt returns `false`). A
 * missing entry goes through {@link pickRecentFile} instead. */
export async function openRecentFile(path: string): Promise<boolean> {
  return withUnsavedChangesGuard(async () => {
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

function askRecentMissing(path: string, name: string): Promise<RecentMissingDecision> {
  return new Promise((resolve) => {
    missingPrompt = { path, name, resolve };
  });
}

/** The `RecentMissingDialog` component calls this with the user's choice. */
export function resolveRecentMissingPrompt(decision: RecentMissingDecision): void {
  const prompt = missingPrompt;
  missingPrompt = null;
  prompt?.resolve(decision);
}

/** "Locate…": the native Open picker, then opens the picked file and drops the old (missing)
 * entry — the old entry's slot in the list is effectively re-pointed to the new location, since
 * a successful open adds the new path at the top (SPEC-018 §2.12). Nothing changes if the picker
 * is cancelled or the guarded open doesn't go through (unsaved changes, cancelled). */
async function locateRecentFile(oldPath: string): Promise<void> {
  const picked = await openFileDialog({ multiple: false, filters: OPEN_FILTERS });
  if (typeof picked !== "string") {
    return;
  }
  if (await openRecentFile(picked)) {
    await removeRecentFile(oldPath);
  }
}

function fileNameOf(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

/** `File → Open Recent`'s entry click (H-15, SPEC-018 §2.12): a missing entry shows the
 * dedicated dialog instead of failing through the normal open flow's error toast. */
export async function pickRecentFile(path: string, exists: boolean | null): Promise<void> {
  if (exists === false) {
    const decision = await askRecentMissing(path, fileNameOf(path));
    if (decision === "remove") {
      await removeRecentFile(path);
    } else if (decision === "locate") {
      await locateRecentFile(path);
    }
    return;
  }
  await openRecentFile(path);
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
  missingPrompt = null;
}
