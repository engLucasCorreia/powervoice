import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type {
  BitDepth,
  DocumentDto,
  DocumentProbeDto,
  DownmixChoiceDto,
  EventName,
  ImportStartedDto,
  IpcError,
  JobProgressDto,
  SaveContainerDto,
  SaveDitherPref,
} from "../ipc/bindings";
import {
  documentClose,
  documentOpen,
  documentOpenCancel,
  documentSave,
  documentSaveAs,
} from "../ipc/commands";
import { registerAction } from "../shortcuts";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { t } from "../i18n";
import { pushNotice } from "../state/notices.svelte";
import { saveSettings, settingsState } from "../state/settings.svelte";
import { setSelectionFromResult } from "../state/selection.svelte";
import { applyRestoredSpectralView } from "../state/spectral.svelte";
import { seek } from "../state/transport.svelte";
import {
  audioKeyFor,
  clearPendingRestore,
  setPendingRestore,
  setTimeRulerFormat,
  setVerticalZoom,
} from "../state/waveformView.svelte";
import { DEFAULT_VERTICAL_ZOOM } from "../waveform/verticalZoom";

/**
 * Document store (S1-03): the open document's facts (`document_changed` event + command
 * results), the native Open/Save As dialogs (`tauri-plugin-dialog`, ADR-007 Amendment 2), the
 * simple unsaved-changes prompt (SPEC-004 §2.8) before Open/quit replaces the document, and the
 * window title. Registers the File keymap actions (Ctrl+O, Ctrl+S, Ctrl+Shift+S).
 *
 * T-209 additions: the import job's progress/cancel (SPEC-005 §2.3), the multichannel
 * channel-choice dialog (§2.4), the Save As format row (WAV/FLAC, §2.6/§2.7) and the clip prompt
 * (§2.8) — every flow the backend now runs through `document_open`/`document_save`/
 * `document_save_as`'s job/confirmation contracts.
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
  waveform_view: null,
  recovered: false,
};

const WAV_FILTERS = [{ name: "WAV", extensions: ["wav"] }];
const FLAC_FILTERS = [{ name: "FLAC", extensions: ["flac"] }];
/**
 * File → Open's dialog filter (T-202, SPEC-005 §2.2): every format `vox_io::decode` accepts.
 */
const OPEN_FILTERS = [
  {
    name: "Audio",
    extensions: ["wav", "flac", "mp3", "m4a", "ogg"],
  },
];

/** SPEC-005 §2.6: sources Save can never write back to — Save acts as Save As instead, WAV
 * 24-bit and `‹name›.wav` preselected. */
const LOSSY_EXTENSIONS = new Set(["mp3", "m4a", "ogg"]);

function extensionOf(path: string): string {
  return path.slice(path.lastIndexOf(".") + 1).toLowerCase();
}

function isLossySourcePath(path: string): boolean {
  return LOSSY_EXTENSIONS.has(extensionOf(path));
}

function withExtension(path: string, ext: string): string {
  const dot = path.lastIndexOf(".");
  const base = dot > path.lastIndexOf("/") && dot > 0 ? path.slice(0, dot) : path;
  return `${base}.${ext}`;
}

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
  defaultContainer: SaveContainerDto;
  defaultBits: BitDepth;
  /** H-20 (SPEC-005 §3 `save_dither`): the remembered Settings preference. */
  defaultDither: SaveDitherPref;
}

/** T-306/H-20 (SPEC-018 §2.9/§2.11, SPEC-005 §2.4): a confirmation the user must answer before
 * Open/Save proceeds. */
export interface ConfirmPrompt {
  kind: "already_open" | "changed_on_disk" | "multichannel_source";
  name: string;
}

interface PendingConfirmPrompt extends ConfirmPrompt {
  resolve: (confirmed: boolean) => void;
}

/** T-209 (SPEC-005 §2.4): "Open stereo file" — the dialog's data (from `dialog.channel_choice`'s
 * `probe` param) plus the resolve callback `ChannelChoiceDialog` calls with the user's answer. */
export interface ChannelChoicePrompt {
  probe: DocumentProbeDto;
}

interface PendingChannelChoicePrompt extends ChannelChoicePrompt {
  resolve: (answer: { choice: DownmixChoiceDto; remember: boolean } | null) => void;
}

/** T-209 (SPEC-005 §2.8): the clip prompt's data (from `dialog.overs`'s `count`/`peak_dbfs`
 * params). */
export interface ClipPrompt {
  count: number;
  peakDbfs: number;
}

export type ClipDecision = "clip" | "float" | "cancel";

interface PendingClipPrompt extends ClipPrompt {
  resolve: (decision: ClipDecision) => void;
}

/** T-209/H-20 (SPEC-005 §2.3): the import job's progress, plus (H-20) the document shell shown
 * immediately from `import_started` — file name, rate and (when the container states one)
 * length — well before the import completes and `document_changed` swaps in the real document. */
export interface ImportJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
  name: string;
  sampleRateHz: number;
  lenSamples: number | null;
}

let doc = $state<DocumentDto>({ ...EMPTY });
let unsavedPrompt = $state<UnsavedPrompt | null>(null);
let saveAsPrompt = $state<SaveAsPrompt | null>(null);
let confirmPrompt = $state<PendingConfirmPrompt | null>(null);
let channelChoicePrompt = $state<PendingChannelChoicePrompt | null>(null);
let clipPrompt = $state<PendingClipPrompt | null>(null);
let importJob = $state<ImportJobState | null>(null);
let unlistenImportProgress: (() => void) | null = null;

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
  readonly channelChoicePrompt: ChannelChoicePrompt | null;
  readonly clipPrompt: ClipPrompt | null;
  readonly importJob: ImportJobState | null;
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
    get channelChoicePrompt() {
      return channelChoicePrompt ? { probe: channelChoicePrompt.probe } : null;
    },
    get clipPrompt() {
      return clipPrompt ? { count: clipPrompt.count, peakDbfs: clipPrompt.peakDbfs } : null;
    },
    get importJob() {
      return importJob;
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
  // T-301 (SPEC-004 §2.7): "(recovered)" until the first save.
  const shown = info.recovered ? t("document.recovered_title", { name }) : name;
  return `${shown}${isModified(info) ? " *" : ""} — PowerVoice`;
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
 * `isOpen`: only a successful *open* restores the sidecar's spectral/waveform view (T-306/H-12,
 * SPEC-018 §2.6.5) — every other `document_changed` (an edit, a save, ...) leaves the panes'
 * current view alone, so a live tweak never gets clobbered by a stale value from before it was
 * pushed to the backend (`spectral.svelte.ts`/`waveformView.svelte.ts`'s own debounce).
 *
 * H-12: the waveform viewport (`start_sample`/`samples_per_pixel`) can't be validated/clamped
 * here — SPEC-018 §2.6.5's "an out-of-range `samples_per_pixel` -> zoom full" needs the
 * viewport's pixel width, which only `WaveformView` knows — so it's recorded as a *pending*
 * restore (`setPendingRestore`) for `WaveformView`'s own zoom-to-fit effect to consume once that
 * width is known. Selection, cursor, (T-206) `time_ruler_format` and (H-35) `vertical_zoom` need
 * no viewport to validate (already clamped/defaulted on the Rust side) and are applied
 * immediately.
 */
function applyDoc(next: DocumentDto, isOpen = false): void {
  doc = next;
  updateWindowTitle(next);
  if (!isOpen) {
    return;
  }
  if (next.spectral_view) {
    applyRestoredSpectralView(next.spectral_view);
  }
  const view = next.waveform_view;
  if (view) {
    setPendingRestore(
      audioKeyFor(next.sample_rate_hz, next.len_samples),
      view.start_sample,
      view.samples_per_pixel,
    );
    setSelectionFromResult(view.selection ? [view.selection.start_sample, view.selection.end_sample] : null);
    void seek(view.cursor_samples);
    setTimeRulerFormat(view.time_ruler_format);
    setVerticalZoom(view.vertical_zoom);
  } else {
    clearPendingRestore();
    setTimeRulerFormat("timecode");
    setVerticalZoom(DEFAULT_VERTICAL_ZOOM);
  }
}

/** T-301: a document recovery opened (restores its spectral view like an open). */
export function applyRecoveredDocument(next: DocumentDto): void {
  applyDoc(next, true);
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

/** T-209 (SPEC-005 §2.4): shows the channel-choice dialog; resolves `null` on Cancel. */
function askChannelChoice(
  probe: DocumentProbeDto,
): Promise<{ choice: DownmixChoiceDto; remember: boolean } | null> {
  return new Promise((resolve) => {
    channelChoicePrompt = { probe, resolve };
  });
}

/** The `ChannelChoiceDialog` component calls this with the user's choice (or `null` on Cancel).
 * `remember` persists the policy in Settings → Files (`multichannel_policy`) before resolving. */
export function resolveChannelChoicePrompt(
  answer: { choice: DownmixChoiceDto; remember: boolean } | null,
): void {
  const prompt = channelChoicePrompt;
  channelChoicePrompt = null;
  if (answer?.remember) {
    const policy = answer.choice.kind === "average" ? "always_mix" : "always_first_channel";
    void saveSettings({ multichannel_policy: policy });
  }
  prompt?.resolve(answer);
}

/** T-209 (SPEC-005 §2.8): shows the clip prompt. */
function askClipPrompt(count: number, peakDbfs: number): Promise<ClipDecision> {
  return new Promise((resolve) => {
    clipPrompt = { count, peakDbfs, resolve };
  });
}

/** The `ClipPromptDialog` component calls this with the user's choice. */
export function resolveClipPrompt(decision: ClipDecision): void {
  const prompt = clipPrompt;
  clipPrompt = null;
  prompt?.resolve(decision);
}

function isChannelChoiceError(err: unknown): err is IpcError & { params: { probe: string } } {
  return (
    isIpcError(err) && err.code === "needs_confirmation" && err.key === "dialog.channel_choice"
  );
}

function isOversError(err: unknown): err is IpcError & { params: Record<string, string> } {
  return isIpcError(err) && err.code === "needs_confirmation" && err.key === "dialog.overs";
}

/** H-20 (SPEC-005 §2.4): `dialog.multichannel_source`'s `name` param. */
function isMultichannelSourceError(err: unknown): err is IpcError & { params: { name: string } } {
  return (
    isIpcError(err) && err.code === "needs_confirmation" && err.key === "dialog.multichannel_source"
  );
}

/** T-209/H-20: ensures the `job_progress` (kind `import`) and `import_started` listeners are
 * attached, so `importJob` tracks an import started by this or another call (mirrors
 * `state/normalize.svelte.ts`'s `ensureListening`). */
async function ensureImportProgressListening(): Promise<void> {
  if (unlistenImportProgress) {
    return;
  }
  try {
    const unlistenProgress = await listen<JobProgressDto>(
      "job_progress" satisfies EventName,
      (event) => applyImportJobProgress(event.payload),
    );
    const unlistenStarted = await listen<ImportStartedDto>(
      "import_started" satisfies EventName,
      (event) => applyImportStarted(event.payload),
    );
    unlistenImportProgress = () => {
      unlistenProgress();
      unlistenStarted();
    };
  } catch {
    // Not running inside a real Tauri window (e.g. Vitest) — tests drive the store functions
    // directly instead.
  }
}

/**
 * H-20 (SPEC-005 §2.3): "the editor immediately shows the document shell" — applies
 * `import_started`, before the (potentially slow) decode loop even starts. The OS window title
 * reflects it too ("Opening ‹name›… — PowerVoice"); a cancelled/failed import restores the
 * previous document's title (`applyImportJobProgress`), and a successful one is overwritten by
 * the `document_open` command's own `document_changed` at commit.
 */
export function applyImportStarted(payload: ImportStartedDto): void {
  importJob = {
    jobId: payload.job_id,
    fraction: 0,
    state: "running",
    name: payload.name,
    sampleRateHz: payload.sample_rate_hz,
    lenSamples: payload.len_samples,
  };
  try {
    void getCurrentWindow()
      .setTitle(`${t("document.opening", { name: payload.name })} — PowerVoice`)
      .catch(() => {});
  } catch {
    // Not running inside a Tauri window (e.g. Vitest) — nothing to update.
  }
}

/** Applies one `job_progress` event to the store (kind `import` only) — a pure function so it's
 * directly testable (mirrors `state/normalize.svelte.ts`'s `applyNormalizeJobProgress`). Preserves
 * the document shell fields `import_started` set; a cancelled/failed job restores the window title
 * to the (unchanged) current document's (SPEC-005 §2.3 "Cancel": no partial document is ever left
 * open, so the title must not keep showing the abandoned import). */
export function applyImportJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "import") {
    return;
  }
  importJob =
    importJob && importJob.jobId === payload.job_id
      ? { ...importJob, fraction: payload.fraction, state: payload.state }
      : {
          jobId: payload.job_id,
          fraction: payload.fraction,
          state: payload.state,
          name: "",
          sampleRateHz: 0,
          lenSamples: null,
        };
  if (payload.state === "cancelled" || payload.state === "failed") {
    updateWindowTitle(doc);
  }
}

/** Cancels the running import job (`document_open_cancel`, best-effort). */
export function cancelImportJob(): void {
  if (importJob && importJob.state === "running") {
    void documentOpenCancel(importJob.jobId).catch(report);
  }
}

/** Dismisses a finished import job's progress panel (Done/Cancelled/Failed). */
export function dismissImportJob(): void {
  importJob = null;
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
    if (isChannelChoiceError(err)) {
      const probe = JSON.parse(err.params.probe) as DocumentProbeDto;
      const answer = await askChannelChoice(probe);
      if (!answer) {
        return false;
      }
      return run(() => documentOpen(path, false, answer.choice), true);
    }
    report(err);
    return false;
  }
}

/**
 * T-306 (SPEC-018 §2.9): saves in place, showing "‹name› was changed on disk" and retrying with
 * `overwrite: true` if the user picks "Overwrite". T-209 (SPEC-005 §2.8): also shows the clip
 * prompt on `dialog.overs` — "Clip and save" retries with `confirmClip: true`; "Save as 32-bit
 * float instead" saves the bound path as WAV 32-bit float instead (never clips); "Cancel" leaves
 * the document untouched. H-20 (SPEC-005 §2.4): also shows "saving replaces the stereo source
 * with mono" on `dialog.multichannel_source` — Save retries with `confirmMultichannel: true`.
 */
export async function saveDocument(): Promise<boolean> {
  let overwrite = false;
  let confirmClip = false;
  let confirmMultichannel = false;
  for (;;) {
    try {
      applyDoc(await documentSave(overwrite, confirmClip, confirmMultichannel));
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
        overwrite = true;
        continue;
      }
      if (isOversError(err)) {
        const decision = await askClipPrompt(
          Number(err.params.count ?? "0"),
          Number(err.params.peak_dbfs ?? "0"),
        );
        if (decision === "cancel") {
          return false;
        }
        if (decision === "float") {
          return saveDocumentAs(withExtension(doc.path ?? "untitled.wav", "wav"), "wav", "32f");
        }
        confirmClip = true;
        continue;
      }
      if (isMultichannelSourceError(err)) {
        if (!(await askConfirm("multichannel_source", err.params.name))) {
          return false;
        }
        confirmMultichannel = true;
        continue;
      }
      report(err);
      return false;
    }
  }
}

export const saveDocumentAs = (
  path: string,
  container: SaveContainerDto,
  bits: BitDepth,
  dither: SaveDitherPref = "tpdf",
  confirmClip = false,
  confirmMultichannel = false,
): Promise<boolean> =>
  run(() => documentSaveAs(path, container, bits, dither, confirmClip, confirmMultichannel));

/**
 * T-209/H-20 (SPEC-005 §2.4/§2.8): drives a `document_save_as` call through the clip prompt and
 * the multichannel-source warning, like {@link saveDocument} does for plain Save — used by the
 * Save As dialog's native picker ({@link confirmSaveAsPrompt}), which can't just call
 * {@link saveDocumentAs} directly since it also needs to react to those confirmations.
 */
async function saveAsWithClipHandling(
  path: string,
  container: SaveContainerDto,
  bits: BitDepth,
  dither: SaveDitherPref,
): Promise<boolean> {
  let confirmClip = false;
  let confirmMultichannel = false;
  for (;;) {
    try {
      applyDoc(await documentSaveAs(path, container, bits, dither, confirmClip, confirmMultichannel));
      return true;
    } catch (err) {
      if (isOversError(err)) {
        const decision = await askClipPrompt(
          Number(err.params.count ?? "0"),
          Number(err.params.peak_dbfs ?? "0"),
        );
        if (decision === "cancel") {
          return false;
        }
        if (decision === "float") {
          return saveDocumentAs(withExtension(path, "wav"), "wav", "32f", dither);
        }
        confirmClip = true;
        continue;
      }
      if (isMultichannelSourceError(err)) {
        if (!(await askConfirm("multichannel_source", err.params.name))) {
          return false;
        }
        confirmMultichannel = true;
        continue;
      }
      report(err);
      return false;
    }
  }
}

/**
 * Save from the unsaved-changes prompt: in place, or — for a never-saved recording (no path yet)
 * or a compressed source (SPEC-005 §2.6: Save acts as Save As) — through the native Save As
 * dialog at WAV 24-bit. Resolves `true` only once the document is saved (a cancelled dialog or a
 * failed save keeps the prompt's action from running).
 */
async function saveForPrompt(): Promise<boolean> {
  if (doc.path && !isLossySourcePath(doc.path)) {
    return (await saveDocument()) && !isModified(doc);
  }
  const suggested = doc.path
    ? withExtension(doc.name ?? "untitled.wav", "wav")
    : (doc.name ?? "untitled.wav");
  const path = await saveFileDialog({
    defaultPath: suggested,
    filters: WAV_FILTERS,
  });
  if (typeof path !== "string") {
    return false;
  }
  return (await saveDocumentAs(path, "wav", "24", "tpdf")) && !isModified(doc);
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

/**
 * Opens the Save As format/bit-depth prompt (`SaveAsDialog`); the dialog then runs the native
 * picker. T-209: preselects the document's current container (WAV stays WAV, FLAC stays FLAC) —
 * a compressed source (SPEC-005 §2.6) has no container of its own to preselect, so it defaults to
 * WAV with `‹name›.wav` (`requestSave`/`saveForPrompt` already route it here for that reason).
 */
export function openSaveAsPrompt(): void {
  const path = doc.path;
  const lossy = path !== null && isLossySourcePath(path);
  const isFlac = path !== null && !lossy && extensionOf(path) === "flac";
  saveAsPrompt = {
    suggestedName: lossy
      ? withExtension(doc.name ?? "untitled.wav", "wav")
      : (doc.name ?? "untitled.wav"),
    defaultContainer: isFlac ? "flac" : "wav",
    defaultBits: "24",
    // H-20 (SPEC-005 §3 `save_dither`): the dialog's remembered preference.
    defaultDither: settingsState().current?.save_dither ?? "tpdf",
  };
}

export function cancelSaveAsPrompt(): void {
  saveAsPrompt = null;
}

/** Confirms the Save As prompt: shows the native save dialog, then saves in `container` at
 * `bits`/`dither` if a path was chosen (running the clip and multichannel-source prompts like
 * plain Save, SPEC-005 §2.4/§2.8). Called by `SaveAsDialog` once the user picked a format/bit
 * depth/dither. Also remembers `dither` in Settings (SPEC-005 §3: "remembered as a preference"). */
export async function confirmSaveAsPrompt(
  container: SaveContainerDto,
  bits: BitDepth,
  dither: SaveDitherPref,
): Promise<void> {
  const suggested = saveAsPrompt?.suggestedName ?? "untitled.wav";
  saveAsPrompt = null;
  const ext = container === "flac" ? "flac" : "wav";
  const path = await saveFileDialog({
    defaultPath: withExtension(suggested, ext),
    filters: container === "flac" ? FLAC_FILTERS : WAV_FILTERS,
  });
  if (typeof path === "string") {
    if (dither !== settingsState().current?.save_dither) {
      void saveSettings({ save_dither: dither });
    }
    await saveAsWithClipHandling(path, container, bits, dither);
  }
}

/** File → Save (Ctrl+S): saves in place, or opens Save As when there's no bound path yet or the
 * document was opened from a compressed source (SPEC-005 §2.6: Save acts as Save As). */
export async function requestSave(): Promise<void> {
  if (!doc.path || isLossySourcePath(doc.path)) {
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
 * File → Close (H-19): closes the current document (after the unsaved-changes guard) without
 * quitting the app — the same `documentClose` call the quit-time flow already uses (SPEC-004
 * §2.8), just reachable from the menu too. `document_changed` (already wired below) resets the
 * UI to the "no document" state once the backend confirms the close.
 */
export async function requestClose(): Promise<void> {
  await withUnsavedChangesGuard(async () => {
    try {
      await documentClose();
    } catch (err) {
      report(err);
    }
  });
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
  await ensureImportProgressListening();
  cleanups.push(() => {
    try {
      unlistenImportProgress?.();
    } catch {
      // A failed unlisten during teardown is harmless (mirrors this function's other cleanups).
    }
    unlistenImportProgress = null;
  });
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
      // T-301 (SPEC-004 §2.8): Save or Don't Save ends the session — its directory is deleted,
      // so a discarded document is never offered for recovery at the next start.
      try {
        await documentClose();
      } catch (err) {
        report(err);
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
  channelChoicePrompt = null;
  clipPrompt = null;
  importJob = null;
  try {
    unlistenImportProgress?.();
  } catch {
    // A failed unlisten after the mock IPC layer was already torn down is harmless.
  }
  unlistenImportProgress = null;
}
