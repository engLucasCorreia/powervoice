import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  EventName,
  IpcError,
  ModuleDescriptorDto,
  ModulePresetImportedDto,
  ParamChangedDto,
  PresetEntryDto,
  PresetRefDto,
  RackLatencyDto,
  RackStateDto,
} from "../ipc/bindings";
import {
  modulePresetDelete,
  modulePresetExport,
  modulePresetImport,
  modulePresetLoad,
  modulePresetRename,
  modulePresetSave,
  modulePresetsList,
  moduleResetDefault,
  moduleTelemetrySubscribe,
  paramSetNormalized,
  paramSetPlain,
  paramSetText,
  rackAb,
  rackAdd,
  rackBypass,
  rackEditorClose,
  rackEditorCloseAll,
  rackEditorOpen,
  rackGet,
  rackListModules,
  rackMove,
  rackPresetDelete,
  rackPresetExport,
  rackPresetImport,
  rackPresetLoad,
  rackPresetRename,
  rackPresetSave,
  rackPresetsList,
  rackRemove,
  rackRestart,
} from "../ipc/commands";
import { t } from "../i18n";
import { decodeVxmt } from "../ipc/moduleTelemetry";
import { toArrayBuffer } from "../ipc/telemetry";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";

/**
 * Rack store (S3-01, SPEC-012 §2.1–§2.6): the rack panel's slot list and the module registry,
 * kept in sync with the engine through `rack_get`/mutating commands and the `rack_changed` /
 * `param_changed` / `rack_latency` events. Parameter drags are coalesced here to at most one
 * `param_set_normalized` call per animation frame (§2.4 "no lost values": the latest value
 * always wins).
 */

const EMPTY: RackStateDto = { slots: [], ab: false, latency_samples: 0 };

let state = $state<RackStateDto>({ ...EMPTY });
let modules = $state<ModuleDescriptorDto[]>([]);
let loading = $state(true);
/** True once a command has failed with `error.rack_unavailable` (no output device open yet). */
let unavailable = $state(false);
/** S3-06: the last rack slot whose panel had focus (SPEC-014 §2.3 "last-focused NR slot" — the
 * backend validates it actually has a `NoiseProfile` extension, so this is tracked generically). */
let lastFocusedSlot: number | null = null;
/** H-03: the latest module-telemetry values per slot uid (`VXMT`), each in its slot's
 * `telemetry` channel order. Replaced wholesale per frame (60 Hz), so not deeply reactive. */
let meters = $state.raw<Record<number, readonly number[]>>({});

/** Read-only accessor for components. */
export function rackState(): {
  readonly state: RackStateDto;
  readonly modules: ModuleDescriptorDto[];
  readonly loading: boolean;
  readonly unavailable: boolean;
} {
  return {
    get state() {
      return state;
    },
    get modules() {
      return modules;
    },
    get loading() {
      return loading;
    },
    get unavailable() {
      return unavailable;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (!isIpcError(err)) {
    return;
  }
  if (err.key === "error.rack_unavailable") {
    unavailable = true;
  }
  pushNotice(noticeFromIpcError(err));
}

function applyState(next: RackStateDto): void {
  state = next;
  unavailable = false;
}

/** Merges one `param_changed` echo in place, without replacing slot/param object identity where
 * unaffected — keeps an open text field elsewhere in the panel untouched. */
function applyParamChanged(msg: ParamChangedDto): void {
  const slot = state.slots[msg.slot];
  if (!slot) {
    return;
  }
  const values = slot.values.map((v) =>
    v.id === msg.id
      ? { id: msg.id, value: msg.value, normalized: msg.normalized, text: msg.text }
      : v,
  );
  const slots = state.slots.slice();
  slots[msg.slot] = { ...slot, values };
  state = { ...state, slots };
  unavailable = false;
}

async function run(command: () => Promise<RackStateDto>): Promise<void> {
  try {
    applyState(await command());
  } catch (err) {
    report(err);
  }
}

export const addModule = (moduleId: string, index: number): Promise<void> =>
  run(() => rackAdd(moduleId, index));
export const removeSlot = (index: number): Promise<void> => run(() => rackRemove(index));
export const moveSlot = (from: number, to: number): Promise<void> => run(() => rackMove(from, to));
export const setBypass = (index: number, on: boolean): Promise<void> =>
  run(() => rackBypass(index, on));
export const setAb = (on: boolean): Promise<void> => run(() => rackAb(on));
export const restartSlot = (index: number): Promise<void> => run(() => rackRestart(index));

// --- Plugin windows (T-901) ---------------------------------------------------------------------

/** Opens slot `index`'s plugin window ("‹Plugin› — PowerVoice"); a failure is a notice. */
export const openPluginWindow = (index: number, pluginName: string): Promise<void> =>
  run(() => rackEditorOpen(index, t("rack.slot.window.title", { plugin: pluginName })));
/** Closes slot `index`'s plugin window. */
export const closePluginWindow = (index: number): Promise<void> => run(() => rackEditorClose(index));
/** Closes every plugin window. */
export const closeAllPluginWindows = (): Promise<void> => run(() => rackEditorCloseAll());

// --- Module & rack presets (T-406, SPEC-012 §2.7) ---------------------------------------------

/** Runs a preset-management call (list/save/rename/delete: not a `RackStateDto`), reporting and
 * swallowing an `IpcError` (returns `null`) so a preset menu/dialog can show the failure inline
 * or fall back gracefully instead of throwing. */
async function runPreset<T>(command: () => Promise<T>): Promise<T | null> {
  try {
    return await command();
  } catch (err) {
    report(err);
    return null;
  }
}

/** Factory presets first, then user-saved ones — for slot `index`'s preset menu. `null` on
 * failure (already reported). */
export function listModulePresets(moduleId: string): Promise<PresetEntryDto[] | null> {
  return runPreset(() => modulePresetsList(moduleId));
}

/** Saves slot `index`'s current state as a new user preset. `null` on failure (e.g. a duplicate
 * name without `overwrite`) — already reported. */
export function saveModulePreset(
  index: number,
  name: string,
  includeNoisePrint: boolean,
  overwrite = false,
): Promise<PresetEntryDto | null> {
  return runPreset(() => modulePresetSave(index, name, includeNoisePrint, overwrite));
}

/** Loads a module preset into slot `index` (SPEC-012 §2.7). */
export const loadModulePreset = (
  index: number,
  moduleId: string,
  preset: PresetRefDto,
): Promise<void> => run(() => modulePresetLoad(index, moduleId, preset));

/** Resets slot `index`'s parameters to their schema defaults (a committed blob is kept). */
export const resetSlotToDefault = (index: number): Promise<void> =>
  run(() => moduleResetDefault(index));

/** Renames a user module preset. `null` on failure (already reported). */
export function renameModulePreset(
  moduleId: string,
  oldName: string,
  newName: string,
): Promise<PresetEntryDto | null> {
  return runPreset(() => modulePresetRename(moduleId, oldName, newName));
}

/** Deletes a user module preset. `false` on failure (already reported). */
export async function deleteModulePreset(moduleId: string, name: string): Promise<boolean> {
  return (await runPreset(() => modulePresetDelete(moduleId, name).then(() => true))) ?? false;
}

/** The outcome of an attempted save/import that might collide with an existing preset name
 * (H-22 "Overwrite-confirm on save"): `conflict` means a preset of that name already exists and
 * nothing was written — the caller re-issues the same call with `overwrite: true` (or, for
 * import, calls {@link importModulePreset}/{@link importRackPreset} with `overwrite: true`) once
 * the user confirms "Replace preset ‹name›?". `name` is only present when the caller couldn't
 * already know it (an imported file's own name) — a typed "Save as…" name is already known to
 * the caller. */
export type PresetWriteOutcome<T> =
  | { status: "ok"; value: T }
  | { status: "conflict"; name?: string }
  | { status: "failed" };

function conflictName(err: IpcError): string | undefined {
  return err.params.name;
}

async function runConflictAware<T>(command: () => Promise<T>): Promise<PresetWriteOutcome<T>> {
  try {
    return { status: "ok", value: await command() };
  } catch (err) {
    if (isIpcError(err) && err.key === "error.preset_already_exists") {
      return { status: "conflict", name: conflictName(err) };
    }
    report(err);
    return { status: "failed" };
  }
}

/** Attempts to save slot `index`'s current state as a new user preset without overwriting
 * (SPEC-012 §2.7 "Save as…"): `conflict` when `name` is already taken — re-save with
 * `saveModulePreset(index, name, includeNoisePrint, true)` once the user confirms. */
export function trySaveModulePreset(
  index: number,
  name: string,
  includeNoisePrint: boolean,
): Promise<PresetWriteOutcome<PresetEntryDto>> {
  return runConflictAware(() => modulePresetSave(index, name, includeNoisePrint, false));
}

/** Attempts to save the live rack as a new user rack preset without overwriting: `conflict` when
 * `name` is already taken — re-save with `saveRackPreset(name, true)` once confirmed. */
export function trySaveRackPreset(name: string): Promise<PresetWriteOutcome<PresetEntryDto>> {
  return runConflictAware(() => rackPresetSave(name, false));
}

/** Exports user module preset `name` (of `moduleId`) to `path` (H-22 "Export…"). `false` on
 * failure (already reported). */
export async function exportModulePreset(moduleId: string, name: string, path: string): Promise<boolean> {
  return (await runPreset(() => modulePresetExport(moduleId, name, path).then(() => true))) ?? false;
}

/** Attempts to import a module preset file at `path` without overwriting: `conflict` (with the
 * file's own preset `name`) when that name is already taken — re-import with
 * `importModulePreset(path, true)` once confirmed. */
export function tryImportModulePreset(path: string): Promise<PresetWriteOutcome<ModulePresetImportedDto>> {
  return runConflictAware(() => modulePresetImport(path, false));
}

/** Imports a module preset file at `path`, replacing an existing same-named preset when
 * `overwrite` is set. `null` on failure (already reported). */
export function importModulePreset(path: string, overwrite = true): Promise<ModulePresetImportedDto | null> {
  return runPreset(() => modulePresetImport(path, overwrite));
}

/** Exports user rack preset `name` to `path` (H-22 "Export…"). `false` on failure (already
 * reported). */
export async function exportRackPreset(name: string, path: string): Promise<boolean> {
  return (await runPreset(() => rackPresetExport(name, path).then(() => true))) ?? false;
}

/** Attempts to import a rack preset file at `path` without overwriting: `conflict` (with the
 * file's own preset `name`) when that name is already taken — re-import with
 * `importRackPreset(path, true)` once confirmed. */
export function tryImportRackPreset(path: string): Promise<PresetWriteOutcome<PresetEntryDto>> {
  return runConflictAware(() => rackPresetImport(path, false));
}

/** Imports a rack preset file at `path`, replacing an existing same-named preset when `overwrite`
 * is set. `null` on failure (already reported). */
export function importRackPreset(path: string, overwrite = true): Promise<PresetEntryDto | null> {
  return runPreset(() => rackPresetImport(path, overwrite));
}

/** Factory rack presets first, then user-saved ones — Effects → Rack Presets. `null` on failure
 * (already reported). */
export function listRackPresets(): Promise<PresetEntryDto[] | null> {
  return runPreset(() => rackPresetsList());
}

/** Saves the live rack as a new user rack preset. `null` on failure (already reported). */
export function saveRackPreset(name: string, overwrite = false): Promise<PresetEntryDto | null> {
  return runPreset(() => rackPresetSave(name, overwrite));
}

/** Loads a rack preset, replacing the live rack. */
export const loadRackPreset = (preset: PresetRefDto): Promise<void> =>
  run(() => rackPresetLoad(preset));

/** Renames a user rack preset. `null` on failure (already reported). */
export function renameRackPreset(oldName: string, newName: string): Promise<PresetEntryDto | null> {
  return runPreset(() => rackPresetRename(oldName, newName));
}

/** Deletes a user rack preset. `false` on failure (already reported). */
export async function deleteRackPreset(name: string): Promise<boolean> {
  return (await runPreset(() => rackPresetDelete(name).then(() => true))) ?? false;
}

/** A rack slot's panel gained focus (a click or a keyboard focus inside it). */
export function noteSlotFocused(index: number): void {
  lastFocusedSlot = index;
}

/** The last-focused slot's index, or `null` (nothing focused yet this session). */
export function lastFocusedSlotIndex(): number | null {
  return lastFocusedSlot;
}

/** H-03: slot `uid`'s telemetry values from the latest `VXMT` frame (in its `telemetry` channel
 * order), or `undefined` before any frame carried it. */
export function slotTelemetry(uid: number): readonly number[] | undefined {
  return meters[uid];
}

/** Handles one module-telemetry channel message (`VXMT`, SPEC-016 §4.12). */
export function onModuleTelemetry(message: unknown): void {
  const buf = toArrayBuffer(message);
  const frame = buf ? decodeVxmt(buf) : null;
  if (!frame) {
    return;
  }
  const next: Record<number, readonly number[]> = {};
  for (const record of frame.records) {
    next[record.slotUid] = record.values;
  }
  // H-43: a frame that repeats the current values is not written, so the slot meters don't
  // re-render.
  if (!sameMeters(meters, next)) {
    meters = next;
  }
}

function sameMeters(a: Record<number, readonly number[]>, b: Record<number, readonly number[]>): boolean {
  const keys = Object.keys(b);
  if (Object.keys(a).length !== keys.length) {
    return false;
  }
  for (const key of keys) {
    const x = a[Number(key)];
    const y = b[Number(key)]!;
    if (!x || x.length !== y.length || x.some((v, i) => v !== y[i])) {
      return false;
    }
  }
  return true;
}

/**
 * Sets a parameter from typed text (Rust parses it, SPEC-012 §2.6); returns whether it was
 * accepted. `notifyOnError: false` (the inline text-entry field) shows its own error outline
 * instead of a toast — see `ParamControl.svelte`.
 */
/**
 * Sets a parameter from a plain Hz/dB/Q value immediately (S3-07, SPEC-015 §2.6.6): double-click
 * (toggle a band), Alt+click, and the header toggles — one-shot gestures, not a drag. Node drags
 * use {@link setParamPlainDragged} instead, coalesced to one call per animation frame.
 */
export const setParamPlain = (slot: number, id: number, value: number): Promise<void> =>
  run(() => paramSetPlain(slot, id, value));

export async function setParamText(
  slot: number,
  id: number,
  text: string,
  opts: { notifyOnError?: boolean } = {},
): Promise<boolean> {
  try {
    applyState(await paramSetText(slot, id, text));
    return true;
  } catch (err) {
    if (opts.notifyOnError !== false) {
      report(err);
    }
    return false;
  }
}

// --- Parameter drags: at most one IPC call per animation frame, latest wins (SPEC-012 §2.4) ---

const requestFrame: (cb: () => void) => number =
  typeof requestAnimationFrame === "function"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => setTimeout(cb, 16) as unknown as number;
const cancelFrame: (id: number) => void =
  typeof cancelAnimationFrame === "function"
    ? (id) => cancelAnimationFrame(id)
    : (id) => clearTimeout(id);

interface PendingDrag {
  value: number;
  frame: number;
}

const pendingDrags = new Map<string, PendingDrag>();

function dragKey(slot: number, id: number): string {
  return `${slot}:${id}`;
}

function flushDrag(key: string, slot: number, id: number): void {
  const pending = pendingDrags.get(key);
  if (!pending) {
    return;
  }
  pendingDrags.delete(key);
  paramSetNormalized(slot, id, pending.value)
    .then(applyState)
    .catch(report);
}

/**
 * Slider drag: a normalized `[0, 1]` position. Coalesced to at most one `param_set_normalized`
 * call per animation frame — call this as often as pointer events arrive; only the latest value
 * before each frame is sent.
 */
export function setParamNormalized(slot: number, id: number, value: number): void {
  const key = dragKey(slot, id);
  const existing = pendingDrags.get(key);
  if (existing) {
    existing.value = value;
    return;
  }
  const frame = requestFrame(() => flushDrag(key, slot, id));
  pendingDrags.set(key, { value, frame });
}

/** Test helper: waits for any coalesced drag calls to be sent. */
export function flushPendingDrags(): void {
  for (const [key, pending] of pendingDrags) {
    cancelFrame(pending.frame);
    pendingDrags.delete(key);
    const [slotStr, idStr] = key.split(":");
    paramSetNormalized(Number(slotStr), Number(idStr), pending.value)
      .then(applyState)
      .catch(report);
  }
}

// --- Plain-value drags (S3-07, SPEC-015 §2.6.6): the EQ graph's node drags, same coalescing
// rule as the slider drags above but sending Hz/dB/Q values, not normalized positions.

const pendingPlainDrags = new Map<string, PendingDrag>();

function flushPlainDrag(key: string, slot: number, id: number): void {
  const pending = pendingPlainDrags.get(key);
  if (!pending) {
    return;
  }
  pendingPlainDrags.delete(key);
  paramSetPlain(slot, id, pending.value).then(applyState).catch(report);
}

/**
 * EQ node drag: a plain Hz/dB/Q value. Coalesced to at most one `param_set_plain` call per
 * animation frame per parameter (SPEC-015 §2.6.4 "Rate") — call this as often as pointer events
 * arrive; only the latest value before each frame is sent.
 */
export function setParamPlainDragged(slot: number, id: number, value: number): void {
  const key = dragKey(slot, id);
  const existing = pendingPlainDrags.get(key);
  if (existing) {
    existing.value = value;
    return;
  }
  const frame = requestFrame(() => flushPlainDrag(key, slot, id));
  pendingPlainDrags.set(key, { value, frame });
}

/** Test helper: waits for any coalesced plain-value drag calls to be sent. */
export function flushPendingPlainDrags(): void {
  for (const [key, pending] of pendingPlainDrags) {
    cancelFrame(pending.frame);
    pendingPlainDrags.delete(key);
    const [slotStr, idStr] = key.split(":");
    paramSetPlain(Number(slotStr), Number(idStr), pending.value)
      .then(applyState)
      .catch(report);
  }
}

// --- Loading and live updates ---------------------------------------------------------------

/** Loads the registry and the current rack, subscribes to `rack_changed`/`param_changed`/
 * `rack_latency`, and returns the teardown. */
export async function loadRack(): Promise<() => void> {
  loading = true;
  const cleanups: Array<() => void> = [];

  try {
    modules = await rackListModules();
  } catch (err) {
    report(err);
  }
  try {
    applyState(await rackGet());
  } catch (err) {
    report(err);
  } finally {
    loading = false;
  }

  try {
    const unlisten = await listen<RackStateDto>("rack_changed" satisfies EventName, (e) =>
      applyState(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Without events the state still follows command results.
  }
  try {
    const unlisten = await listen<ParamChangedDto>("param_changed" satisfies EventName, (e) =>
      applyParamChanged(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Same fallback.
  }
  try {
    const unlisten = await listen<RackLatencyDto>("rack_latency" satisfies EventName, (e) => {
      state = { ...state, latency_samples: e.payload.latency_samples };
    });
    cleanups.push(unlisten);
  } catch {
    // Same fallback.
  }
  try {
    await moduleTelemetrySubscribe(new Channel<ArrayBuffer>((message) => onModuleTelemetry(message)));
  } catch {
    // Without module telemetry the slot meters stay at rest.
  }

  return () => {
    for (const cleanup of cleanups) {
      try {
        cleanup();
      } catch {
        // A failed unlisten during teardown is harmless.
      }
    }
    for (const pending of pendingDrags.values()) {
      cancelFrame(pending.frame);
    }
    pendingDrags.clear();
    for (const pending of pendingPlainDrags.values()) {
      cancelFrame(pending.frame);
    }
    pendingPlainDrags.clear();
    meters = {};
  };
}

/** Test/teardown helper. */
export function resetRackForTest(): void {
  state = { ...EMPTY };
  modules = [];
  loading = true;
  unavailable = false;
  lastFocusedSlot = null;
  meters = {};
  for (const pending of pendingDrags.values()) {
    cancelFrame(pending.frame);
  }
  pendingDrags.clear();
  for (const pending of pendingPlainDrags.values()) {
    cancelFrame(pending.frame);
  }
  pendingPlainDrags.clear();
}
