import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  BakeJobStartedDto,
  AcxCheckReportDto,
  AcxCheckRequestDto,
  AnalyzerResponseDto,
  AppInfo,
  BitDepth,
  CommandName,
  DevicePrefsDto,
  DevicesDto,
  DocumentDto,
  DocumentProbeDto,
  DownmixChoiceDto,
  EditResultDto,
  EditTargetDto,
  ExportFormatsDto,
  ExportRequestDto,
  ExportStartedDto,
  LoudnessAnalyzeRequestDto,
  LoudnessAnalyzeStartedDto,
  MarkerDto,
  MarkerRangeKindDto,
  ModuleDescriptorDto,
  NormalizeJobStartedDto,
  NrCaptureStartedDto,
  PeaksRequestDto,
  PluginEntryDto,
  PluginFoldersDto,
  PluginInstallResultDto,
  PresetEntryDto,
  PresetRefDto,
  RackStateDto,
  RecentFileDto,
  RecoverableSessionDto,
  RecoverResultDto,
  RecoveredTakeActionDto,
  ResponseCurveDto,
  SaveContainerDto,
  SaveDitherPref,
  Settings,
  SpectralViewDto,
  SpectroRequestDto,
  StorageInfoDto,
  TransportStateDto,
  WaveformViewDto,
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

/** H-03: binary `VXMT` module-telemetry frames (the rack slots' meters, SPEC-016 §4.12) on `channel`. */
export async function moduleTelemetrySubscribe(channel: Channel<ArrayBuffer>): Promise<void> {
  return invoke<void>("module_telemetry_subscribe" satisfies CommandName, { channel });
}

/**
 * T-208: binary `VXSA` live-analyzer frames (SPEC-007 §2.9/§4.9) on `channel`, at `response`.
 * Returns the subscriber id used by {@link analyzerSetResponse}/{@link analyzerUnsubscribe}.
 */
export async function analyzerSubscribe(
  channel: Channel<ArrayBuffer>,
  response: AnalyzerResponseDto,
): Promise<number> {
  return invoke<number>("analyzer_subscribe" satisfies CommandName, { channel, response });
}

/** T-208: changes one subscriber's averaging response. */
export async function analyzerSetResponse(
  id: number,
  response: AnalyzerResponseDto,
): Promise<void> {
  return invoke<void>("analyzer_set_response" satisfies CommandName, { id, response });
}

/** T-208: removes a subscriber; the tap turns off once none remain. */
export async function analyzerUnsubscribe(id: number): Promise<void> {
  return invoke<void>("analyzer_unsubscribe" satisfies CommandName, { id });
}

/** S1-01: the engine's app clock in ns (clock sync, ADR-003 §3). */
export async function clockNowNs(): Promise<number> {
  return invoke<number>("clock_now_ns" satisfies CommandName);
}

/** S3-01: the modules available to add (the Add-module menu, SPEC-012 §2.1). */
export async function rackListModules(): Promise<ModuleDescriptorDto[]> {
  return invoke<ModuleDescriptorDto[]>("rack_list_modules" satisfies CommandName);
}

/** S3-01: the current rack state (initial load). */
export async function rackGet(): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_get" satisfies CommandName);
}

/** S3-01: adds a registered module at `index` (`0..=len`), live. */
export async function rackAdd(moduleId: string, index: number): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_add" satisfies CommandName, { moduleId, index });
}

/** S3-01: removes the slot at `slot` (index), live. */
export async function rackRemove(slot: number): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_remove" satisfies CommandName, { slot });
}

/** S3-01: moves the slot at `from` to `to` (drag-reorder), live. */
export async function rackMove(from: number, to: number): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_move" satisfies CommandName, { from, to });
}

/** S3-01: per-slot bypass toggle. */
export async function rackBypass(slot: number, on: boolean): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_bypass" satisfies CommandName, { slot, on });
}

/** S3-01: whole-rack A/B (listening only, SPEC-012 §2.3). */
export async function rackAb(on: boolean): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_ab" satisfies CommandName, { on });
}

/** S3-01: restarts a slot's instance from its committed state (Restart of a failed slot). */
export async function rackRestart(slot: number): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_restart" satisfies CommandName, { slot });
}

/** S3-01: sets a parameter from a normalized `[0, 1]` slider position (SPEC-012 §2.4, §2.6). */
export async function paramSetNormalized(
  slot: number,
  id: number,
  value: number,
): Promise<RackStateDto> {
  return invoke<RackStateDto>("param_set_normalized" satisfies CommandName, { slot, id, value });
}

/** S3-01: sets a parameter from typed text; Rust parses it (SPEC-012 §2.6). */
export async function paramSetText(
  slot: number,
  id: number,
  text: string,
): Promise<RackStateDto> {
  return invoke<RackStateDto>("param_set_text" satisfies CommandName, { slot, id, text });
}

/** S3-07: sets a parameter from a plain Hz/dB/Q value (SPEC-015 §2.6.6, the EQ graph's
 * drag/wheel/double-click gestures). */
export async function paramSetPlain(
  slot: number,
  id: number,
  value: number,
): Promise<RackStateDto> {
  return invoke<RackStateDto>("param_set_plain" satisfies CommandName, { slot, id, value });
}

/** S3-07: the EQ graph's response curve at `points` (Hz), lean-slice JSON (SPEC-015 §2.6.6; the
 * binary `VXRC` frame is hardening). */
export async function rackResponseCurve(
  slot: number,
  points: number[],
): Promise<ResponseCurveDto> {
  return invoke<ResponseCurveDto>("rack_response_curve" satisfies CommandName, { slot, points });
}

// --- Module & rack presets (T-406, SPEC-012 §2.7) --------------------------------------------

/** Factory presets first, then user-saved ones (alphabetical) — a rack slot's preset menu. */
export async function modulePresetsList(moduleId: string): Promise<PresetEntryDto[]> {
  return invoke<PresetEntryDto[]>("module_presets_list" satisfies CommandName, { moduleId });
}

/** Saves slot `slot`'s current state as a new user preset named `name`. `includeNoisePrint`
 * keeps a Noise Reduction slot's captured print in the saved state (off by default, SPEC-012
 * scope); `overwrite` replaces an existing preset of the same name instead of failing. */
export async function modulePresetSave(
  slot: number,
  name: string,
  includeNoisePrint: boolean,
  overwrite = false,
): Promise<PresetEntryDto> {
  return invoke<PresetEntryDto>("module_preset_save" satisfies CommandName, {
    slot,
    name,
    includeNoisePrint,
    overwrite,
  });
}

/** Loads a module preset into slot `slot` (a state with a blob replaces the instance; a
 * parameter-only state is smoothed, with no restart, SPEC-012 §2.7). */
export async function modulePresetLoad(
  slot: number,
  moduleId: string,
  preset: PresetRefDto,
): Promise<RackStateDto> {
  return invoke<RackStateDto>("module_preset_load" satisfies CommandName, {
    slot,
    moduleId,
    preset,
  });
}

/** Renames a user module preset. */
export async function modulePresetRename(
  moduleId: string,
  oldName: string,
  newName: string,
): Promise<PresetEntryDto> {
  return invoke<PresetEntryDto>("module_preset_rename" satisfies CommandName, {
    moduleId,
    oldName,
    newName,
  });
}

/** Deletes a user module preset. */
export async function modulePresetDelete(moduleId: string, name: string): Promise<void> {
  return invoke<void>("module_preset_delete" satisfies CommandName, { moduleId, name });
}

/** Resets every writable parameter of slot `slot` to its schema default (a committed blob, e.g.
 * a noise print, is kept). */
export async function moduleResetDefault(slot: number): Promise<RackStateDto> {
  return invoke<RackStateDto>("module_reset_default" satisfies CommandName, { slot });
}

/** Factory rack presets ("Podcast voice", "Audiobook (ACX)", "Gentle cleanup", ...) first, then
 * user-saved ones (alphabetical) — Effects → Rack Presets. */
export async function rackPresetsList(): Promise<PresetEntryDto[]> {
  return invoke<PresetEntryDto[]>("rack_presets_list" satisfies CommandName);
}

/** Saves the live rack as a new user rack preset named `name`. `overwrite` replaces an existing
 * preset of the same name instead of failing. */
export async function rackPresetSave(
  name: string,
  overwrite = false,
): Promise<PresetEntryDto> {
  return invoke<PresetEntryDto>("rack_preset_save" satisfies CommandName, { name, overwrite });
}

/** Loads a rack preset, replacing the live rack. An unknown module id becomes a placeholder slot
 * with a notice, never a load failure. */
export async function rackPresetLoad(preset: PresetRefDto): Promise<RackStateDto> {
  return invoke<RackStateDto>("rack_preset_load" satisfies CommandName, { preset });
}

/** Renames a user rack preset. */
export async function rackPresetRename(oldName: string, newName: string): Promise<PresetEntryDto> {
  return invoke<PresetEntryDto>("rack_preset_rename" satisfies CommandName, {
    oldName,
    newName,
  });
}

/** Deletes a user rack preset. */
export async function rackPresetDelete(name: string): Promise<void> {
  return invoke<void>("rack_preset_delete" satisfies CommandName, { name });
}

/**
 * Opens `path` as the document (SPEC-005 §2.2-2.4: any format `vox_io::decode` supports, not
 * just WAV — T-202), replacing whatever was open. Runs as a job (T-209): `job_progress` events
 * (`kind: "import"`) report progress until this resolves; `documentOpenCancel` cancels it.
 *
 * T-209 (SPEC-005 §2.4): multichannel input needs `channelChoice` unless the remembered
 * `multichannel_policy` resolves it — otherwise this rejects with `dialog.channel_choice`
 * (its `probe` param is `DocumentProbeDto` JSON) and the caller re-issues with the user's choice.
 *
 * T-306 (SPEC-018 §2.11): `confirmAlreadyOpen` bypasses the "already open in another instance"
 * warning — pass `true` only when re-issuing after the user picked "Open Anyway".
 */
export async function documentOpen(
  path: string,
  confirmAlreadyOpen = false,
  channelChoice: DownmixChoiceDto | null = null,
): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_open" satisfies CommandName, {
    path,
    confirmAlreadyOpen,
    channelChoice,
  });
}

/** T-209: cancels a running import job (SPEC-005 §2.3 "Cancel"). Best-effort. */
export async function documentOpenCancel(jobId: number): Promise<void> {
  return invoke<void>("document_open_cancel" satisfies CommandName, { jobId });
}

/**
 * T-202: probes `path` without importing it (SPEC-005 §2.3 step 1, §2.4) — container/codec/rate/
 * channels, and for multichannel input each channel's peak plus the identical-channels/silent-
 * channel-hint data the channel-choice dialog (T-209) needs.
 */
export async function documentProbe(path: string): Promise<DocumentProbeDto> {
  return invoke<DocumentProbeDto>("document_probe" satisfies CommandName, { path });
}

/**
 * S1-03: saves the current revision back to its bound path and format (SPEC-005 §2.7).
 *
 * T-306 (SPEC-018 §2.9): `overwrite` bypasses the changed-on-disk confirmation — pass `true` only
 * when re-issuing after the user picked "Overwrite".
 * T-209 (SPEC-005 §2.8): `confirmClip` bypasses the clip prompt — pass `true` only when
 * re-issuing after the user picked "Clip and save".
 * H-20 (SPEC-005 §2.4): `confirmMultichannel` bypasses the "saving replaces the stereo source
 * with mono" warning — pass `true` only when re-issuing after the user picked Save.
 */
export async function documentSave(
  overwrite = false,
  confirmClip = false,
  confirmMultichannel = false,
): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_save" satisfies CommandName, {
    overwrite,
    confirmClip,
    confirmMultichannel,
  });
}

/**
 * S1-03/T-209/H-20: saves the current revision to `path` in `container` at `bits`/`dither`, then
 * binds the document to it. `confirmClip`/`confirmMultichannel` — see {@link documentSave}'s doc
 * comment.
 */
export async function documentSaveAs(
  path: string,
  container: SaveContainerDto,
  bits: BitDepth,
  dither: SaveDitherPref,
  confirmClip = false,
  confirmMultichannel = false,
): Promise<DocumentDto> {
  return invoke<DocumentDto>("document_save_as" satisfies CommandName, {
    path,
    container,
    bits,
    dither,
    confirmClip,
    confirmMultichannel,
  });
}

/**
 * T-306 (SPEC-018 §2.6.5): records the spectral pane's current settings for the next save
 * (fire-and-forget — a view-only change never marks the document modified).
 */
export async function sidecarViewSetSpectral(spectral: SpectralViewDto): Promise<void> {
  return invoke<void>("sidecar_view_set_spectral" satisfies CommandName, { spectral });
}

/**
 * H-12 (SPEC-018 §2.6.5): records the shared waveform/spectral viewport plus the selection and
 * edit cursor for the next save (fire-and-forget, same contract as
 * {@link sidecarViewSetSpectral}).
 */
export async function sidecarViewSetWaveform(waveform: WaveformViewDto): Promise<void> {
  return invoke<void>("sidecar_view_set_waveform" satisfies CommandName, { waveform });
}

/** T-306 (SPEC-018 §2.12): the current `File → Open Recent` list, most-recent-first. */
export async function recentFilesGet(): Promise<RecentFileDto[]> {
  return invoke<RecentFileDto[]>("recent_files_get" satisfies CommandName);
}

/** T-306: removes one entry (a missing file's "Remove", or "Remove from List"). */
export async function recentFilesRemove(path: string): Promise<RecentFileDto[]> {
  return invoke<RecentFileDto[]>("recent_files_remove" satisfies CommandName, { path });
}

/** T-306: empties the list (no confirmation — it deletes no user data). */
export async function recentFilesClear(): Promise<void> {
  return invoke<void>("recent_files_clear" satisfies CommandName);
}

/**
 * S1-03: `count` buckets of `(min, max)` (or raw samples below the pyramid floor) as a binary
 * `VXPK` frame (ADR-003 §2) — never JSON floats (CLAUDE.md).
 */
export async function peaksGet(request: PeaksRequestDto): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("peaks_get" satisfies CommandName, { request });
}

/**
 * T-204: binds spectral view `viewId` to `channel`; its tiles stream back as binary `VXST`
 * frames (ADR-003 §2), one message per tile.
 */
export async function spectroAttach(viewId: number, channel: Channel<ArrayBuffer>): Promise<void> {
  return invoke<void>("spectro_attach" satisfies CommandName, { viewId, channel });
}

/** T-204: detaches spectral view `viewId` (its pending tiles are cancelled). */
export async function spectroDetach(viewId: number): Promise<void> {
  return invoke<void>("spectro_detach" satisfies CommandName, { viewId });
}

/** T-204: requests tiles for spectral view `viewId`; a newer request cancels the older one. */
export async function spectroRequest(viewId: number, request: SpectroRequestDto): Promise<void> {
  return invoke<void>("spectro_request" satisfies CommandName, { viewId, request });
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

/**
 * S2-02/H-09: starts a peak-normalize job for `[startSamples, endSamples)` (SPEC-010), targeting
 * either `targetDb` dBFS sample peak or `targetPct` % of full scale (exactly one non-null).
 * Callers resolve "no selection" to the whole file before calling. Progress arrives as
 * `job_progress` events (kind `normalize_peak`); the finished edit result as `normalize_result`.
 */
export async function editNormalizePeakStart(
  startSamples: number,
  endSamples: number,
  targetDb: number | null,
  targetPct: number | null,
): Promise<NormalizeJobStartedDto> {
  return invoke<NormalizeJobStartedDto>("edit_normalize_peak_start" satisfies CommandName, {
    startSamples,
    endSamples,
    targetDb,
    targetPct,
  });
}

/** S2-02/H-09: cancels a running peak-normalize job (best-effort; leaves the document untouched,
 * SPEC-010 §2.8). */
export async function editNormalizePeakCancel(jobId: number): Promise<void> {
  return invoke<void>("edit_normalize_peak_cancel" satisfies CommandName, { jobId });
}

/** T-602: starts a Bake rack job over `[startSamples, endSamples)` (the caller resolves "no
 * selection" to the whole file). Progress arrives as `job_progress` (kind `bake`); a commit as
 * `document_changed`/`history_state` plus a `notice.bake.done` toast. */
export async function editBakeStart(
  startSamples: number,
  endSamples: number,
): Promise<BakeJobStartedDto> {
  return invoke<BakeJobStartedDto>("edit_bake_start" satisfies CommandName, {
    startSamples,
    endSamples,
  });
}

/** T-602: cancels a running bake (best-effort; the document and the rack stay unchanged). */
export async function editBakeCancel(jobId: number): Promise<void> {
  return invoke<void>("edit_bake_cancel" satisfies CommandName, { jobId });
}

/** S2-01: undoes the top history entry. */
export async function historyUndo(): Promise<EditResultDto> {
  return invoke<EditResultDto>("history_undo" satisfies CommandName);
}

/** S2-01: redoes the top history entry. */
export async function historyRedo(): Promise<EditResultDto> {
  return invoke<EditResultDto>("history_redo" satisfies CommandName);
}

// --- S2-03: markers -----------------------------------------------------------------------

/** S2-03: the current marker list (SPEC-009 §2.1), in canonical order. */
export async function markersGet(): Promise<MarkerDto[]> {
  return invoke<MarkerDto[]>("markers_get" satisfies CommandName);
}

/** S2-03: adds a point (`lenSamples === 0`) or region marker (SPEC-009 §2.2). */
export async function markerAdd(posSamples: number, lenSamples: number): Promise<MarkerDto> {
  return invoke<MarkerDto>("marker_add" satisfies CommandName, { posSamples, lenSamples });
}

/** S2-03: renames marker `id` (SPEC-009 §2.4, normalized server-side). */
export async function markerRename(id: number, name: string): Promise<void> {
  return invoke<void>("marker_rename" satisfies CommandName, { id, name });
}

/** S2-03: moves (`kind: "move"`) or resizes (`kind: "resize"`) marker `id` to
 * `[posSamples, posSamples + lenSamples)` — the panel's typed Start/End/Duration edits
 * (SPEC-009 §2.5; dragging is deferred). */
export async function markerSetRange(
  id: number,
  posSamples: number,
  lenSamples: number,
  kind: MarkerRangeKindDto,
): Promise<void> {
  return invoke<void>("marker_set_range" satisfies CommandName, {
    id,
    posSamples,
    lenSamples,
    kind,
  });
}

/** S2-03: deletes the markers in `ids` as one undo entry (SPEC-009 §2.6). */
export async function markerDelete(ids: number[]): Promise<void> {
  return invoke<void>("marker_delete" satisfies CommandName, { ids });
}

/** S4-04: MP3 availability (WAV/FLAC are always available) for the export dialog. */
export async function exportFormats(): Promise<ExportFormatsDto> {
  return invoke<ExportFormatsDto>("export_formats" satisfies CommandName);
}

/** S4-04: starts an export job; progress/completion arrive as `job_progress` events. */
export async function exportStart(request: ExportRequestDto): Promise<ExportStartedDto> {
  return invoke<ExportStartedDto>("export_start" satisfies CommandName, { request });
}

/** S4-04: cancels a running export job (best-effort). */
export async function exportCancel(jobId: number): Promise<void> {
  return invoke<void>("export_cancel" satisfies CommandName, { jobId });
}

/**
 * S3-06: starts a Capture Noise Print job for `[startSamples, endSamples)` (SPEC-014 §2.3).
 * `hintSlot` is the last-focused NR slot, if any. Progress/completion arrive as `job_progress`
 * events (kind `nr_capture`); warnings and errors as `notice`s.
 */
export async function nrCaptureStart(
  hintSlot: number | null,
  startSamples: number,
  endSamples: number,
): Promise<NrCaptureStartedDto> {
  return invoke<NrCaptureStartedDto>("nr_capture_start" satisfies CommandName, {
    hintSlot,
    start: startSamples,
    end: endSamples,
  });
}

/** S3-06: cancels a running capture job (best-effort). */
export async function nrCaptureCancel(jobId: number): Promise<void> {
  return invoke<void>("nr_capture_cancel" satisfies CommandName, { jobId });
}

/**
 * S4-01/H-09: starts a LUFS-normalize job for `[startSamples, endSamples)` targeting `targetLufs`
 * integrated loudness (BS.1770/EBU R128). Callers resolve "no selection" to the whole file before
 * calling, same convention as `editNormalizePeakStart` (mirrors it; no % mode).
 */
export async function editNormalizeLufsStart(
  startSamples: number,
  endSamples: number,
  targetLufs: number,
): Promise<NormalizeJobStartedDto> {
  return invoke<NormalizeJobStartedDto>("edit_normalize_lufs_start" satisfies CommandName, {
    startSamples,
    endSamples,
    targetLufs,
  });
}

/** S4-01/H-09: cancels a running LUFS-normalize job (mirrors `editNormalizePeakCancel`). */
export async function editNormalizeLufsCancel(jobId: number): Promise<void> {
  return invoke<void>("edit_normalize_lufs_cancel" satisfies CommandName, { jobId });
}

/**
 * S4-01: starts a loudness analysis job (integrated/short-term/momentary loudness, LRA, sample
 * and true peak); progress arrives as `job_progress` events and the finished report as a
 * `loudness_report` event, both tagged with the returned `job_id`.
 */
export async function loudnessAnalyzeStart(
  request: LoudnessAnalyzeRequestDto,
): Promise<LoudnessAnalyzeStartedDto> {
  return invoke<LoudnessAnalyzeStartedDto>("loudness_analyze_start" satisfies CommandName, {
    request,
  });
}

/** S4-01: cancels a running loudness analysis job (best-effort). */
export async function loudnessAnalyzeCancel(jobId: number): Promise<void> {
  return invoke<void>("loudness_analyze_cancel" satisfies CommandName, { jobId });
}

/**
 * S4-03: checks the whole document (processed or source, per `request.source`) against the ACX /
 * Audible submission rules (RMS, sample peak, noise floor) and returns one report — synchronous,
 * unlike `loudnessAnalyzeStart`: no job id, no `job_progress`/cancel.
 */
export async function acxCheck(request: AcxCheckRequestDto): Promise<AcxCheckReportDto> {
  return invoke<AcxCheckReportDto>("acx_check" satisfies CommandName, { request });
}

/** T-301 (SPEC-004 §2.7): the recoverable sessions (also removes cleanly closed ones). */
export async function recoveryList(): Promise<RecoverableSessionDto[]> {
  return invoke<RecoverableSessionDto[]>("recovery_list" satisfies CommandName);
}

/** T-301: Recover session `id`, handling its interrupted take per `takeAction`. */
export async function recoveryRecover(
  id: string,
  takeAction: RecoveredTakeActionDto,
): Promise<RecoverResultDto> {
  return invoke<RecoverResultDto>("recovery_recover" satisfies CommandName, { id, takeAction });
}

/** T-301: permanently deletes recoverable session `id`; returns the remaining list. */
export async function recoveryDiscard(id: string): Promise<RecoverableSessionDto[]> {
  return invoke<RecoverableSessionDto[]>("recovery_discard" satisfies CommandName, { id });
}

/** T-301 (SPEC-004 §2.5): Recovery & Storage's figures. */
export async function storageInfo(): Promise<StorageInfoDto> {
  return invoke<StorageInfoDto>("storage_info" satisfies CommandName);
}

/** T-301 (SPEC-004 §2.8): closes the document for good after the quit prompt. */
export async function documentClose(): Promise<void> {
  return invoke<void>("document_close" satisfies CommandName);
}

// --- Plugins (T-804 commands, T-809 plugin manager + "Install module…") -----------------------

/** T-804/T-809: every known plugin — registered ones plus blocklisted files. */
export async function pluginsList(): Promise<PluginEntryDto[]> {
  return invoke<PluginEntryDto[]>("plugins_list" satisfies CommandName);
}

/** T-804: rescans (`full` ignores the cache); emits `plugin_scan_progress` as it goes. Returns
 * the number of audio effects registered afterwards. */
export async function pluginsRescan(full: boolean): Promise<number> {
  return invoke<number>("plugins_rescan" satisfies CommandName, { full });
}

/** T-804: shows/hides a module in Add Module. */
export async function pluginsSetEnabled(moduleId: string, enabled: boolean): Promise<void> {
  return invoke<void>("plugins_set_enabled" satisfies CommandName, { moduleId, enabled });
}

/** T-804: blocks a plugin file. */
export async function pluginsBlock(path: string): Promise<void> {
  return invoke<void>("plugins_block" satisfies CommandName, { path });
}

/** T-804: unblocks a plugin file; `true` if it was blocked. */
export async function pluginsUnblock(path: string): Promise<boolean> {
  return invoke<boolean>("plugins_unblock" satisfies CommandName, { path });
}

/** T-809: clears a flagged plugin's runtime crash count. */
export async function pluginsClearFlag(moduleId: string): Promise<void> {
  return invoke<void>("plugins_clear_flag" satisfies CommandName, { moduleId });
}

/** T-804: adds a custom scan folder (a background rescan follows). */
export async function pluginsAddFolder(path: string): Promise<void> {
  return invoke<void>("plugins_add_folder" satisfies CommandName, { path });
}

/** T-804: removes a custom scan folder (a background rescan follows, T-809). */
export async function pluginsRemoveFolder(path: string): Promise<void> {
  return invoke<void>("plugins_remove_folder" satisfies CommandName, { path });
}

/** T-809: the install folder, the standard scan folders and the custom ones. */
export async function pluginsFolders(): Promise<PluginFoldersDto> {
  return invoke<PluginFoldersDto>("plugins_folders" satisfies CommandName);
}

/** T-809: "Install module…" — copies the file into the per-user plugin folder and scans it. */
export async function pluginsInstall(path: string, replace: boolean): Promise<PluginInstallResultDto> {
  return invoke<PluginInstallResultDto>("plugins_install" satisfies CommandName, { path, replace });
}

/** T-809: shows a plugin file in the system file manager. */
export async function pluginsReveal(path: string): Promise<void> {
  return invoke<void>("plugins_reveal" satisfies CommandName, { path });
}
