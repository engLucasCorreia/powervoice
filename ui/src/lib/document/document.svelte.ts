import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type { BitDepth, DocumentDto, EventName, IpcError } from "../ipc/bindings";
import { documentOpen, documentSave, documentSaveAs } from "../ipc/commands";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { t } from "../i18n";
import { pushNotice } from "../state/notices.svelte";

/**
 * Document store (S1-03): the open document's facts (`document_changed` event + command
 * results), the native Open/Save As dialogs (`tauri-plugin-dialog`, ADR-007 Amendment 2), the
 * simple unsaved-changes prompt (SPEC-004 §2.8) before Open/quit replaces the document, and the
 * window title. Registers the File keymap actions (Ctrl+O, Ctrl+S, Ctrl+Shift+S).
 */

const EMPTY: DocumentDto = {
  name: null,
  path: null,
  sample_rate_hz: 0,
  len_samples: 0,
  dirty: false,
  audio_rev: 0,
};

const WAV_FILTERS = [{ name: "WAV", extensions: ["wav"] }];

export type UnsavedDecision = "save" | "discard" | "cancel";

interface UnsavedPrompt {
  name: string;
  resolve: (decision: UnsavedDecision) => void;
}

export interface SaveAsPrompt {
  suggestedName: string;
  defaultBits: BitDepth;
}

let doc = $state<DocumentDto>({ ...EMPTY });
let unsavedPrompt = $state<UnsavedPrompt | null>(null);
let saveAsPrompt = $state<SaveAsPrompt | null>(null);

/** Read-only accessor for components. */
export function documentState(): {
  readonly current: DocumentDto;
  readonly unsavedPrompt: { readonly name: string } | null;
  readonly saveAsPrompt: SaveAsPrompt | null;
} {
  return {
    get current() {
      return doc;
    },
    get unsavedPrompt() {
      return unsavedPrompt;
    },
    get saveAsPrompt() {
      return saveAsPrompt;
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

/** "name — PowerVoice", "name * — PowerVoice" when modified, or just "PowerVoice" with none
 * open (ticket: title "‹name› — PowerVoice" with `*` when modified). */
export function titleFor(info: DocumentDto): string {
  const name = displayName(info);
  if (!name) {
    return "PowerVoice";
  }
  return `${name}${info.dirty ? " *" : ""} — PowerVoice`;
}

/** A document is open (S1-04: a never-saved recording has no name or path, but a rate). */
export function hasDocument(info: DocumentDto): boolean {
  return info.sample_rate_hz > 0;
}

/** The document's display name: its file name, "Untitled" for a never-saved recording, `null`
 * when none is open. */
export function displayName(info: DocumentDto): string | null {
  return info.name ?? (hasDocument(info) ? t("document.untitled") : null);
}

function updateWindowTitle(info: DocumentDto): void {
  try {
    void getCurrentWindow()
      .setTitle(titleFor(info))
      .catch(() => {});
  } catch {
    // Not running inside a Tauri window (e.g. Vitest) — nothing to update.
  }
}

function applyDoc(next: DocumentDto): void {
  doc = next;
  updateWindowTitle(next);
}

async function run(command: () => Promise<DocumentDto>): Promise<boolean> {
  try {
    applyDoc(await command());
    return true;
  } catch (err) {
    report(err);
    return false;
  }
}

export const openDocument = (path: string): Promise<boolean> => run(() => documentOpen(path));
export const saveDocument = (): Promise<boolean> => run(documentSave);
export const saveDocumentAs = (path: string, bits: BitDepth): Promise<boolean> =>
  run(() => documentSaveAs(path, bits));

function askUnsavedChanges(name: string): Promise<UnsavedDecision> {
  return new Promise((resolve) => {
    unsavedPrompt = { name, resolve };
  });
}

/** The `UnsavedChangesDialog` component calls this with the user's choice. */
export function resolveUnsavedPrompt(decision: UnsavedDecision): void {
  const prompt = unsavedPrompt;
  unsavedPrompt = null;
  prompt?.resolve(decision);
}

/**
 * Runs the unsaved-changes prompt first if the current document is dirty (SPEC-004 §2.8, simple
 * version), then `action` — unless the user cancels, or chose Save and it failed, in which case
 * `action` never runs. Returns whether `action` ran. S1-04's New Recording uses it too.
 */
export async function withUnsavedChangesGuard(action: () => Promise<void>): Promise<boolean> {
  if (doc.dirty) {
    const decision = await askUnsavedChanges(displayName(doc) ?? "");
    if (decision === "cancel") {
      return false;
    }
    if (decision === "save") {
      const saved = await saveDocument();
      if (!saved || doc.dirty) {
        return false;
      }
    }
  }
  await action();
  return true;
}

/** File → Open… (Ctrl+O): the native dialog, guarded by unsaved changes. */
export async function requestOpen(): Promise<void> {
  await withUnsavedChangesGuard(async () => {
    const picked = await openFileDialog({ multiple: false, filters: WAV_FILTERS });
    if (typeof picked === "string") {
      await openDocument(picked);
    }
  });
}

/** Opens the Save As bit-depth prompt (`SaveAsDialog`); the dialog then runs the native picker. */
export function openSaveAsPrompt(): void {
  saveAsPrompt = {
    suggestedName: doc.name ?? "untitled.wav",
    defaultBits: "24",
  };
}

export function cancelSaveAsPrompt(): void {
  saveAsPrompt = null;
}

/** Confirms the Save As prompt: shows the native save dialog, then saves at `bits` if a path was
 * chosen. Called by `SaveAsDialog` once the user picked a bit depth. */
export async function confirmSaveAsPrompt(bits: BitDepth): Promise<void> {
  const suggested = saveAsPrompt?.suggestedName ?? "untitled.wav";
  saveAsPrompt = null;
  const path = await saveFileDialog({ defaultPath: suggested, filters: WAV_FILTERS });
  if (typeof path === "string") {
    await saveDocumentAs(path, bits);
  }
}

/** File → Save (Ctrl+S): saves in place, or opens Save As when there's no bound path yet. */
export async function requestSave(): Promise<void> {
  if (!doc.path) {
    openSaveAsPrompt();
    return;
  }
  await saveDocument();
}

/** File → Save As… (Ctrl+Shift+S): always opens the bit-depth prompt. */
export function requestSaveAs(): void {
  openSaveAsPrompt();
}

/**
 * Wires the store: keymap actions, `document_changed` events, and (best-effort — not exercised
 * by Vitest, no real Tauri window there) the unsaved-changes prompt on window close. Returns the
 * teardown.
 */
export async function initDocument(): Promise<() => void> {
  const cleanups: Array<() => void> = [
    registerAction("file.open", () => void requestOpen()),
    registerAction("file.save", () => void requestSave()),
    registerAction("file.save_as", requestSaveAs),
  ];
  try {
    const unlisten = await listen<DocumentDto>("document_changed" satisfies EventName, (e) =>
      applyDoc(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Without the event the document state still follows command results.
  }
  try {
    const unlisten = await getCurrentWindow().onCloseRequested(async (event) => {
      if (!doc.dirty) {
        return;
      }
      event.preventDefault();
      const decision = await askUnsavedChanges(displayName(doc) ?? "");
      if (decision === "cancel") {
        return;
      }
      if (decision === "save") {
        const saved = await saveDocument();
        if (!saved || doc.dirty) {
          return;
        }
      }
      await getCurrentWindow().destroy();
    });
    cleanups.push(unlisten);
  } catch {
    // Not running inside a real Tauri window.
  }
  return () => {
    for (const cleanup of cleanups) {
      try {
        cleanup();
      } catch {
        // A failed unlisten during teardown is harmless.
      }
    }
  };
}

/** Test/teardown helper. */
export function resetDocumentStateForTest(): void {
  doc = { ...EMPTY };
  unsavedPrompt = null;
  saveAsPrompt = null;
}
