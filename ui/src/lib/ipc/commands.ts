import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  BitDepth,
  CommandName,
  DevicePrefsDto,
  DevicesDto,
  DocumentDto,
  EditResultDto,
  EditTargetDto,
  PeaksRequestDto,
  Settings,
  TransportStateDto,
} from "./bindings";

/**
 * Hand-written typed wrappers around `invoke` (ADR-003). Every command gets one wrapper here;
 * they use generated types (`./bindings`) only, never hand-copied shapes. The `satisfies
 * CommandName` on each command literal is what would catch a typo or a name Rust doesn't export.
 */

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info" satisfies CommandName);
}

/** T-104: current settings, from the in-memory cache (never touches disk on the Rust side). */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("settings_get" satisfies CommandName);
}

/** T-104: replaces the settings file (atomic write) and returns the canonical saved value. */
export async function setSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("settings_set" satisfies CommandName, { settings });
}

/** S1-01: the Audio Devices dialog's data (the engine's current device list). */
export async function devicesList(): Promise<DevicesDto> {
  return invoke<DevicesDto>("devices_list" satisfies CommandName);
}

/** S1-01: applies (and, when it worked, saves) a device selection. */
export async function devicesSelect(prefs: DevicePrefsDto): Promise<DevicesDto> {
  return invoke<DevicesDto>("devices_select" satisfies CommandName, { prefs });
}

/** S1-01: current transport state. */
export async function transportGet(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_get" satisfies CommandName);
}

export async function transportPlay(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_play" satisfies CommandName);
}

export async function transportPause(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_pause" satisfies CommandName);
}

export async function transportStop(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_stop" satisfies CommandName);
}

export async function transportPlayFromStart(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_play_from_start" satisfies CommandName);
}

export async function transportReturnToStart(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_return_to_start" satisfies CommandName);
}

export async function transportSeek(positionSamples: number): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_seek" satisfies CommandName, { positionSamples });
}

/** S1-01: binary `VXTM` telemetry frames at 60 Hz (ADR-003 §2) on `channel`. */
export async function telemetrySubscribe(channel: Channel<ArrayBuffer>): Promise<void> {
  return invoke<void>("telemetry_subscribe" satisfies CommandName, { channel });
}

/** S1-01: the engine's app clock in ns (clock sync, ADR-003 §3). */
export async function clockNowNs(): Promise<number> {
  return invoke<number>("clock_now_ns" satisfies CommandName);
}

/** S1-03: opens `path` as the document (SPEC-005 §2.3), replacing whatever was open. */
export async function documentOpen(path: string): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_open" satisfies CommandName, { path });
}

/** S1-03: saves the current revision back to its bound path and format (SPEC-005 §2.7). */
export async function documentSave(): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_save" satisfies CommandName);
}

/** S1-03: saves the current revision to `path` at `bits`, then binds the document to it. */
export async function documentSaveAs(path: string, bits: BitDepth): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_save_as" satisfies CommandName, { path, bits });
}

/**
 * S1-03: `count` buckets of `(min, max)` (or raw samples below the pyramid floor) as a binary
 * `VXPK` frame (ADR-003 §2) — never JSON floats (CLAUDE.md).
 */
export async function peaksGet(request: PeaksRequestDto): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("peaks_get" satisfies CommandName, { request });
}

/** S2-01: cuts `[startSamples, endSamples)` (SPEC-008 §2.1). */
export async function editCut(startSamples: number, endSamples: number): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_cut" satisfies CommandName, { startSamples, endSamples });
}

/** S2-01: copies `[startSamples, endSamples)` into the clipboard. Not an edit. */
export async function editCopy(startSamples: number, endSamples: number): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_copy" satisfies CommandName, { startSamples, endSamples });
}

/** S2-01: pastes the clipboard at `target` (a cursor or a selection to replace). */
export async function editPaste(target: EditTargetDto): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_paste" satisfies CommandName, { target });
}

/** S2-01: deletes `[startSamples, endSamples)`, closing the gap. */
export async function editDelete(startSamples: number, endSamples: number): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_delete" satisfies CommandName, { startSamples, endSamples });
}

/** S2-01: trims the document to `[startSamples, endSamples)` (Audition: Crop). */
export async function editTrim(startSamples: number, endSamples: number): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_trim" satisfies CommandName, { startSamples, endSamples });
}

/** S2-01: silences `[startSamples, endSamples)` with exact `+0.0` samples. */
export async function editSilence(startSamples: number, endSamples: number): Promise<EditResultDto> {
  return invoke<EditResultDto>("edit_silence" satisfies CommandName, { startSamples, endSamples });
}

/** S2-01: undoes the top history entry. */
export async function historyUndo(): Promise<EditResultDto> {
  return invoke<EditResultDto>("history_undo" satisfies CommandName);
}

/** S2-01: redoes the top history entry. */
export async function historyRedo(): Promise<EditResultDto> {
  return invoke<EditResultDto>("history_redo" satisfies CommandName);
}
