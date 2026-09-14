import { listen } from "@tauri-apps/api/event";
import type { DocumentDto, EventName, IpcError, MarkerDto, MarkerRangeKindDto } from "../ipc/bindings";
import {
  markerAdd,
  markerDelete,
  markerRename,
  markerSetRange,
  markersGet,
} from "../ipc/commands";
import { registerAction } from "../keymap";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { recordState } from "../state/record.svelte";
import { hasSelection, selectionState } from "../state/selection.svelte";
import {
  extrapolatedHeardPositionAt,
  extrapolatedPositionAt,
  seek,
  transportState,
} from "../state/transport.svelte";

/**
 * Markers store (S2-03, SPEC-009 essential subset): the marker list (`markers_get`, refetched on
 * every `document_changed` — marker commands emit it too, `document_commands.rs`'s `after_edit`),
 * the panel's single selection, add/rename/move-resize/delete, and the keymap actions `marker.add`
 * (M), `marker.delete_selected` (Ctrl+0) and `marker.next`/`marker.prev` (Ctrl+Alt+→/←).
 *
 * Drag-to-move on the waveform, Delete All/Filtered, filter/sort and marker `kind` are deferred
 * (ticket "Out" list) — the panel here is a flat, position-sorted list.
 */

/** SPEC-009 §2.7's "previous while playing" grace period, in samples-independent seconds. */
const NAV_PREV_GRACE_S = 0.5;

let markers = $state<MarkerDto[]>([]);
let selectedId = $state<number | null>(null);
/** H-21: see {@link isTakeMarker}. */
const takeMarkerIds = new Set<number>();

/** Read-only accessor for components (the Markers panel). */
export function markersState(): {
  readonly list: MarkerDto[];
  readonly selectedId: number | null;
} {
  return {
    get list() {
      return markers;
    },
    get selectedId() {
      return selectedId;
    },
  };
}

/** Row click (SPEC-009 §2.8's essential subset: select and jump; multi-select/range-select on
 * the time selection is deferred). */
export function selectMarker(id: number | null): void {
  selectedId = id;
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

async function refresh(): Promise<void> {
  try {
    // `?? []`: defensive against a mocked/unimplemented backend returning `null` (e.g. Vitest's
    // `mockIPC` default), not just an empty list.
    markers = (await markersGet()) ?? [];
  } catch (err) {
    report(err);
  }
}

/** The reference position `p` for Add/Navigate (SPEC-009 §2.2/§2.7): the heard position while
 * playing (extrapolated at the key event's timestamp), else the cursor/playhead. */
function referencePosition(eventTimeStampMs?: number): number {
  if (transportState().state.playing) {
    return extrapolatedPositionAt(eventTimeStampMs ?? performance.now());
  }
  return transportState().playheadSamples;
}

/** M / the panel's + button (SPEC-009 §2.2): a point at the heard position while playing (a
 * selection is ignored then — "M always adds a point during playback"), else a region over a
 * non-empty selection or a point at the cursor. Key repeat is ignored. */
export async function addMarker(event?: KeyboardEvent): Promise<void> {
  if (event?.repeat) {
    return;
  }
  const playing = transportState().state.playing;
  // H-21 (SPEC-022 §2.9, SPEC-002 §2.2): during a take or record operation M always adds a point
  // at the heard position under the key press (pre-/post-roll), or `at + k` in the record window —
  // the telemetry position then, extrapolated without the clamp to the (old) document length.
  const takeRunning = recordState().state.recording;
  let pos: number;
  let len = 0;
  if (takeRunning) {
    pos = extrapolatedHeardPositionAt(event?.timeStamp ?? performance.now());
  } else if (!playing && hasSelection()) {
    const sel = selectionState().current!;
    pos = sel.startSample;
    len = sel.endSample - sel.startSample;
  } else {
    pos = referencePosition(event?.timeStamp);
  }
  try {
    const marker = await markerAdd(Math.max(0, Math.round(pos)), Math.round(len));
    if (takeRunning) {
      takeMarkerIds.add(marker.id);
    }
    markers = [...markers, marker].sort(
      (a, b) => a.pos_samples - b.pos_samples || a.id - b.id,
    );
    selectedId = marker.id;
  } catch (err) {
    report(err);
  }
}

/**
 * H-21: ids of markers added during a take/operation — already in the committed document's
 * coordinates, so an Insert operation's waveform (which shifts the existing audio after `at`
 * right by the take length while it grows) must not shift them. Ids are never reused.
 */
export function isTakeMarker(id: number): boolean {
  return takeMarkerIds.has(id);
}

/** `/`, F2, double-click (panel component calls this on commit). Empty/unchanged names are the
 * panel's own concern; the backend also rejects an empty normalized name. */
export async function renameMarker(id: number, name: string): Promise<void> {
  try {
    await markerRename(id, name);
    await refresh();
  } catch (err) {
    report(err);
  }
}

/** The panel's typed Start/End/Duration cells (SPEC-009 §2.5's panel-typed subset; dragging is
 * deferred). */
export async function setMarkerRange(
  id: number,
  posSamples: number,
  lenSamples: number,
  kind: MarkerRangeKindDto,
): Promise<void> {
  try {
    await markerSetRange(id, posSamples, lenSamples, kind);
    await refresh();
  } catch (err) {
    report(err);
  }
}

/** Ctrl+0 / the panel's Delete button: deletes the single panel selection (SPEC-009 §2.6's
 * essential subset — multi-select and Delete All/Filtered are deferred). A no-op with none
 * selected. */
export async function deleteSelectedMarker(): Promise<void> {
  if (selectedId === null) {
    return;
  }
  const id = selectedId;
  try {
    await markerDelete([id]);
    markers = markers.filter((m) => m.id !== id);
    if (selectedId === id) {
      selectedId = null;
    }
  } catch (err) {
    report(err);
  }
}

/** A row click, or a flag click on the waveform: selects and jumps (SPEC-009 §2.8's essential
 * subset — a region's range isn't turned into a time selection here, deferred). */
export function jumpToMarker(id: number): void {
  const marker = markers.find((m) => m.id === id);
  if (!marker) {
    return;
  }
  selectedId = id;
  void seek(marker.pos_samples);
}

/** Ctrl+Alt+→ (SPEC-009 §2.7): the marker with the smallest position past the reference position;
 * a no-op with none. */
export function goToNextMarker(event?: KeyboardEvent): void {
  const p = referencePosition(event?.timeStamp);
  const next = markers.filter((m) => m.pos_samples > p).at(0);
  if (next) {
    jumpToMarker(next.id);
  }
}

/** Ctrl+Alt+← (SPEC-009 §2.7): the marker with the largest position before the reference
 * position, minus a grace period while playing (repeated presses walk backwards instead of
 * re-landing on the same marker); a no-op with none. */
export function goToPreviousMarker(event?: KeyboardEvent): void {
  const p = referencePosition(event?.timeStamp);
  const rateHz = transportState().state.doc_rate_hz;
  const grace = transportState().state.playing && rateHz > 0 ? NAV_PREV_GRACE_S * rateHz : 0;
  const candidates = markers.filter((m) => m.pos_samples < p - grace);
  const prev = candidates.at(-1);
  if (prev) {
    jumpToMarker(prev.id);
  }
}

/**
 * Wires the store: keymap actions (`marker.add`, `marker.delete_selected`, `marker.next`,
 * `marker.prev`) and refetches the marker list on every `document_changed` (open/save, audio
 * edits shifting markers, undo/redo, and every marker_* command — S2-03 essential subset: no
 * dedicated `markers_changed` event). Returns the teardown.
 */
export async function initMarkers(): Promise<() => void> {
  const cleanups: Array<() => void> = [
    registerAction("marker.add", () => void addMarker()),
    registerAction("marker.delete_selected", () => void deleteSelectedMarker()),
    registerAction("marker.next", () => goToNextMarker()),
    registerAction("marker.prev", () => goToPreviousMarker()),
  ];
  try {
    const unlisten = await listen<DocumentDto>("document_changed" satisfies EventName, () => {
      void refresh();
    });
    cleanups.push(unlisten);
  } catch {
    // Without the event the panel only follows the calls this module itself made.
  }
  await refresh();
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
export function resetMarkersForTest(): void {
  markers = [];
  selectedId = null;
  takeMarkerIds.clear();
}
