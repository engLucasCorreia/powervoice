import type {
  IpcError,
  RecoverableSessionDto,
  RecoveredTakeActionDto,
  StorageInfoDto,
} from "../ipc/bindings";
import { recoveryDiscard, recoveryList, recoveryRecover, storageInfo } from "../ipc/commands";
import { applyRecoveredDocument, withUnsavedChangesGuard } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";

/**
 * Crash recovery store (T-301, SPEC-004 §2.7/§2.8): the start-up "PowerVoice didn't shut down
 * properly" dialog and the same list under File → Recovery & Storage… (the app has no Settings
 * window yet). Per session: Recover (with the interrupted take's choice, "Apply as recorded" by
 * default) or Discard (after a confirmation). Dialog-wide: Decide later.
 */

export type RecoveryMode = "startup" | "storage";

let mode = $state<RecoveryMode | null>(null);
let sessions = $state<RecoverableSessionDto[]>([]);
let storage = $state<StorageInfoDto | null>(null);
let takeActions = $state<Record<string, RecoveredTakeActionDto>>({});
let unrecoverable = $state<string[]>([]);
let pendingDiscard = $state<RecoverableSessionDto | null>(null);
let busy = $state(false);

/** Read-only accessor for the dialog. */
export function recoveryState(): {
  readonly mode: RecoveryMode | null;
  readonly sessions: RecoverableSessionDto[];
  readonly storage: StorageInfoDto | null;
  readonly pendingDiscard: RecoverableSessionDto | null;
  readonly busy: boolean;
} {
  return {
    get mode() {
      return mode;
    },
    get sessions() {
      return sessions;
    },
    get storage() {
      return storage;
    },
    get pendingDiscard() {
      return pendingDiscard;
    },
    get busy() {
      return busy;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

/** At start-up, before any document opens: shows the dialog when sessions are recoverable. */
export async function initRecovery(): Promise<void> {
  try {
    sessions = await recoveryList();
    if (sessions.length > 0) {
      mode = "startup";
    }
  } catch (err) {
    report(err);
  }
}

/** "Decide later": keeps everything; the next start asks again. */
export function decideLater(): void {
  mode = null;
  pendingDiscard = null;
}

async function refreshStorage(): Promise<void> {
  try {
    const info = await storageInfo();
    storage = info;
    sessions = info.sessions;
  } catch (err) {
    report(err);
  }
}

/** File → Recovery & Storage…: session storage figures plus the recoverable sessions. */
export async function openRecoveryStorage(): Promise<void> {
  mode = "storage";
  await refreshStorage();
}

export function closeRecovery(): void {
  mode = null;
  pendingDiscard = null;
}

export function takeActionFor(id: string): RecoveredTakeActionDto {
  return takeActions[id] ?? "apply";
}

export function setTakeAction(id: string, action: RecoveredTakeActionDto): void {
  takeActions = { ...takeActions, [id]: action };
}

export function isUnrecoverable(id: string): boolean {
  return unrecoverable.includes(id);
}

/**
 * Recover: opens the session as the document (modified, "(recovered)", bound to its original
 * path). Runs the unsaved-changes prompt first when a modified document is open. A session with
 * nothing intact is marked so only Discard stays available.
 */
export async function recover(id: string): Promise<boolean> {
  if (busy) {
    return false;
  }
  busy = true;
  let recovered = false;
  try {
    await withUnsavedChangesGuard(async () => {
      try {
        const result = await recoveryRecover(id, takeActionFor(id));
        applyRecoveredDocument(result.document);
        sessions = sessions.filter((s) => s.id !== id);
        mode = null;
        recovered = true;
      } catch (err) {
        if (isIpcError(err) && err.key === "error.recovery.nothing_intact") {
          unrecoverable = [...unrecoverable, id];
        }
        report(err);
      }
    });
  } finally {
    busy = false;
  }
  return recovered;
}

/** Discard: asks "Permanently delete the unsaved changes to ‹name›?" first. */
export function requestDiscard(session: RecoverableSessionDto): void {
  pendingDiscard = session;
}

export function cancelDiscard(): void {
  pendingDiscard = null;
}

export async function confirmDiscard(): Promise<void> {
  const target = pendingDiscard;
  pendingDiscard = null;
  if (!target) {
    return;
  }
  try {
    sessions = await recoveryDiscard(target.id);
    if (mode === "storage") {
      await refreshStorage();
    } else if (sessions.length === 0) {
      mode = null;
    }
  } catch (err) {
    report(err);
  }
}

/** Test/teardown helper. */
export function resetRecoveryForTest(): void {
  mode = null;
  sessions = [];
  storage = null;
  takeActions = {};
  unrecoverable = [];
  pendingDiscard = null;
  busy = false;
}
