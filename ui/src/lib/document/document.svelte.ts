import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type { BitDepth, DocumentDto, EventName, IpcError } from "../ipc/bindings";
import { documentOpen, documentSave, documentSaveAs } from "../ipc/commands";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { t } from "../i18n";
import { pushNotice } from "../state/notices.svelte";
import { applyRestoredSpectralView } from "../state/spectral.svelte";

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
  sidecar_dirty: false,
  spectral_view: null,
};

const WAV_FILTERS = [{ name: "WAV", extensions: ["wav"] }];
/**
 * File → Open's dialog filter (T-202, SPEC-005 §2.2): every format `vox_io::decode` accepts. Save
 * and Save As stay WAV-only ([`WAV_FILTERS`]; save format choice is T-201's scope).
 */
const OPEN_FILTERS = [
  {
    name: "Audio",
    extensions: ["wav", "flac", "mp3", "m4a", "ogg"],
  },
];

export type UnsavedDecision = "save" | "discard" | "cancel";

interface UnsavedPrompt {
  name: string;
  /** T-306 (SPEC-018 §2.4): only `sidecar_dirty` is set — the dialog adds "Effect settings
   * changed." */
  effectSettingsOnly: boolean;
  resolve: (decision: UnsavedDecision) => void;
}

export interface SaveAsPrompt {
  suggestedName: string;
  defaultBits: BitDepth;
}

/** T-306 (SPEC-018 §2.9/§2.11): a confirmation the user must answer before Open/Save proceeds. */
export interface ConfirmPrompt {
  kind: "already_open" | "changed_on_disk";
  name: string;
}

interface PendingConfirmPrompt extends ConfirmPrompt {
  resolve: (confirmed: boolean) => void;
}

let doc = $state<DocumentDto>({ ...EMPTY });
let unsavedPrompt = $state<UnsavedPrompt | null>(null);
let saveAsPrompt = $state<SaveAsPrompt | null>(null);
let confirmPrompt = $state<PendingConfirmPrompt | null>(null);

/** T-306: `dirty || sidecar_dirty` — the title's `*` and every unsaved-changes prompt fire on
 * either (SPEC-018 §2.4). */
export function isModified(info: DocumentDto): boolean {
  return info.dirty || info.sidecar_dirty;
}

/** Read-only accessor for components. */
export function documentState(): {
  readonly current: DocumentDto;
  readonly unsavedPrompt: { readonly name: string; readonly effectSettingsOnly: boolean } | null;
  readonly saveAsPrompt: SaveAsPrompt | null;
  readonly confirmPrompt: ConfirmPrompt | null;
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
    get confirmPrompt() {
      return confirmPrompt ? { kind: confirmPrompt.kind, name: confirmPrompt.name } : null;
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
 * open (ticket: title "‹name› — PowerVoice" with `*` when modified — T-306 extends "modified" to
 * `dirty || sidecar_dirty`, SPEC-018 §2.4). */
export function titleFor(info: DocumentDto): string {
  const name = displayName(info);
  if (!name) {
    return "PowerVoice";
  }
  return `${name}${isModified(info) ? " *" : ""} — PowerVoice`;
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

/**
 * `isOpen`: only a successful *open* restores the sidecar's spectral view (T-306, SPEC-018
 * §2.6.5) — every other `document_changed` (an edit, a save, ...) leaves the pane's current
 * settings alone, so a live tweak never gets clobbered by a stale value from before it was
 * pushed to the backend (`spectral.svelte.ts`'s own debounce).
 */
function applyDoc(next: DocumentDto, isOpen = false): void {
  doc = next;
  updateWindowTitle(next);
  if (isOpen && next.spectral_view) {
    applyRestoredSpectralView(next.spectral_view);
  }
}

async function run(command: () => Promise<DocumentDto>, isOpen = false): Promise<boolean> {
  try {
    applyDoc(await command(), isOpen);
    return true;
  } catch (err) {
    report(err);
    return false;
  }
}

function askConfirm(kind: ConfirmPrompt["kind"], name: string): Promise<boolean> {
  return new Promise((resolve) => {
    confirmPrompt = { kind, name, resolve };
  });
}

/** The `ConfirmDialog` component calls this with the user's choice. */
export function resolveConfirmPrompt(confirmed: boolean): void {
  const prompt = confirmPrompt;
  confirmPrompt = null;
  prompt?.resolve(confirmed);
}

/**
 * T-306 (SPEC-018 §2.11): opens `path`, showing "‹name› is already open in another window" and
 * retrying with the confirm flag if the user picks "Open Anyway". Any other failure (including a
 * cancelled confirmation) is reported as a notice, same as [`run`].
 */
export async function openDocument(path: string): Promise<boolean> {
  try {
    applyDoc(await documentOpen(path, false), true);
    return true;
  } catch (err) {
    if (isIpcError(err) && err.code === "needs_confirmation" && err.key === "dialog.already_open") {
      const name = err.params.name ?? "";
      if (!(await askConfirm("already_open", name))) {
        return false;
      }
      return run(() => documentOpen(path, true), true);
    }
    report(err);
    return false;
  }
}

/**
 * T-306 (SPEC-018 §2.9): saves in place, showing "‹name› was changed on disk" and retrying with
 * `overwrite: true` if the user picks "Overwrite".
 */
export async function saveDocument(): Promise<boolean> {
  try {
    applyDoc(await documentSave(false));
    return true;
  } catch (err) {
    if (
      isIpcError(err) &&
      err.code === "needs_confirmation" &&
      err.key === "dialog.changed_on_disk"
    ) {
      const name = err.params.name ?? "";
      if (!(await askConfirm("changed_on_disk", name))) {
        return false;
      }
      return run(() => documentSave(true));
    }
    report(err);
    return false;
  }
}

export const saveDocumentAs = (path: string, bits: BitDepth): Promise<boolean> =>
  run(() => documentSaveAs(path, bits));

/**
 * Save from the unsaved-changes prompt: in place, or — for a never-saved recording, which has no
 * path yet — through the native Save As dialog at the default bit depth. Resolves `true` only
 * once the document is saved (a cancelled dialog or a failed save keeps the prompt's action from
 * running).
 */
async function saveForPrompt(): Promise<boolean> {
  if (doc.path) {
    return (await saveDocument()) && !isModified(doc);
  }
  const path = await saveFileDialog({
    defaultPath: doc.name ?? "untitled.wav",
    filters: WAV_FILTERS,
  });
  if (typeof path !== "string") {
    return false;
  }
  return (await saveDocumentAs(path, "24")) && !isModified(doc);
}

function askUnsavedChanges(name: string, effectSettingsOnly: boolean): Promise<UnsavedDecision> {
  return new Promise((resolve) => {
    unsavedPrompt = { name, effectSettingsOnly, resolve };
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
  if (isModified(doc)) {
    const decision = await askUnsavedChanges(displayName(doc) ?? "", !doc.dirty && doc.sidecar_dirty);
    if (decision === "cancel") {
      return false;
    }
    if (decision === "save" && !(await saveForPrompt())) {
      return false;
    }
  }
  await action();
  return true;
}

/** File → Open… (Ctrl+O): the native dialog, guarded by unsaved changes. */
export async function requestOpen(): Promise<void> {
  await withUnsavedChangesGuard(async () => {
    const picked = await openFileDialog({ multiple: false, filters: OPEN_FILTERS });
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
      if (!isModified(doc)) {
        return;
      }
      event.preventDefault();
      const decision = await askUnsavedChanges(displayName(doc) ?? "", !doc.dirty && doc.sidecar_dirty);
      if (decision === "cancel") {
        return;
      }
      if (decision === "save" && !(await saveForPrompt())) {
        return;
      }
      // Needs `core:window:allow-destroy` (capabilities/default.json) — without it the call is
      // refused and the window can't be closed at all while the document is dirty.
      try {
        await getCurrentWindow().destroy();
      } catch (err) {
        report(err);
      }
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
  confirmPrompt = null;
}
