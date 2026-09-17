import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { MarkerDto } from "../ipc/bindings";
import { clearActionHandlers, dispatchAction } from "../shortcuts";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { resetSelectionForTest, selectAllOf } from "../state/selection.svelte";
import { resetTransportForTest } from "../state/transport.svelte";
import { docDto, transportStateDto } from "../test/fixtures";
import {
  activateMarker,
  addMarker,
  deleteAllMarkers,
  deleteFilteredMarkers,
  deleteSelectedMarker,
  goToFirstDropout,
  goToNextMarker,
  goToPreviousMarker,
  initMarkers,
  jumpToMarker,
  markersState,
  renameMarker,
  resetMarkersForTest,
  selectMarker,
  setMarkerFilterText,
  setMarkerRange,
  setMarkerSort,
  setMarkerTypeFilter,
} from "./markers.svelte";
import { selectionState } from "../state/selection.svelte";

/**
 * Markers store tests (S2-03, SPEC-009 essential subset, Vitest + mockIPC — the S2-03 ticket's
 * "no auto-rename, key repeat ignored, jump, undo-friendly commands" ACs at the UI layer; the
 * heard-position-while-playing math itself is `transport.test.ts`'s job).
 */

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetMarkersForTest();
  resetSelectionForTest();
  resetTransportForTest();
});

const DOC_CHANGED = docDto({ path: "/tmp/take.wav" });

function marker(id: number, pos: number, len = 0, name = `m${id}`, kind: MarkerDto["kind"] = "user"): MarkerDto {
  return { id, pos_samples: pos, len_samples: len, name, kind };
}

describe("markers store (S2-03)", () => {
  it("M with no selection adds a point at the cursor (stopped)", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_add") {
        calls.push(args);
        return marker(1, 0);
      }
      if (cmd === "markers_get") return [];
      return null;
    });
    await addMarker();
    expect(calls).toEqual([{ posSamples: 0, lenSamples: 0 }]);
    expect(markersState().selectedId).toBe(1);
    expect(markersState().list).toEqual([marker(1, 0)]);
  });

  it("M with a non-empty selection (stopped) adds a region", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_add") {
        calls.push(args);
        return marker(2, 1_000, 4_000);
      }
      return null;
    });
    selectAllOf(10_000); // any non-empty selection
    await addMarker();
    expect(calls).toEqual([{ posSamples: 0, lenSamples: 10_000 }]);
  });

  it("holding M (repeat=true) adds nothing", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_add") {
        calls.push(args);
        return marker(1, 0);
      }
      return null;
    });
    await addMarker({ repeat: true } as KeyboardEvent);
    expect(calls).toHaveLength(0);
  });

  it("renameMarker calls marker_rename and refreshes the list", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_rename") {
        calls.push(args);
        return null;
      }
      if (cmd === "markers_get") return [marker(1, 0, 0, "Intro")];
      return null;
    });
    await renameMarker(1, "Intro");
    expect(calls).toEqual([{ id: 1, name: "Intro" }]);
    expect(markersState().list[0]?.name).toBe("Intro");
  });

  it("setMarkerRange forwards the move/resize kind", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_set_range") {
        calls.push(args);
        return null;
      }
      if (cmd === "markers_get") return [];
      return null;
    });
    await setMarkerRange(1, 100, 0, "move");
    expect(calls).toEqual([{ id: 1, posSamples: 100, lenSamples: 0, kind: "move" }]);
  });

  it("deleteSelectedMarker is a no-op with nothing selected, else deletes and clears selection", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await deleteSelectedMarker();
    expect(calls).toHaveLength(0);

    mockIPC((cmd, args) => {
      if (cmd === "marker_add") return marker(5, 0);
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await addMarker();
    expect(markersState().selectedId).toBe(5);
    await deleteSelectedMarker();
    expect(calls).toEqual([{ ids: [5] }]);
    expect(markersState().selectedId).toBeNull();
    expect(markersState().list).toEqual([]);
  });

  it("jumpToMarker selects the row and seeks to its position", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "marker_add") return marker(1, 12_345);
      if (cmd === "transport_seek") {
        seeks.push(args);
        return transportStateDto({ playhead_samples: 12_345, doc_len_samples: 480_000, can_play: true });
      }
      return null;
    });
    await addMarker();
    selectMarker(null);
    jumpToMarker(1);
    expect(markersState().selectedId).toBe(1);
    expect(seeks).toEqual([{ positionSamples: 12_345 }]);
  });

  it("activateMarker on a region sets the time selection to its exact range (H-57, SPEC-009 §2.8)", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [marker(1, 100_000, 20_000, "R")];
      if (cmd === "transport_seek") {
        seeks.push(args);
        return transportStateDto({ playhead_samples: 100_000, doc_len_samples: 480_000, can_play: true });
      }
      return null;
    });
    const stop = await initMarkers();
    selectAllOf(480_000); // a pre-existing selection must be replaced, not merged
    activateMarker(1);
    expect(markersState().selectedId).toBe(1);
    expect(selectionState().current).toEqual({ startSample: 100_000, endSample: 120_000 });
    expect(seeks).toEqual([{ positionSamples: 100_000 }]);
    stop();
  });

  it("activateMarker on a point clears the time selection", async () => {
    mockIPC((cmd) => {
      if (cmd === "markers_get") return [marker(2, 40_000, 0, "P")];
      if (cmd === "transport_seek") {
        return transportStateDto({ playhead_samples: 40_000, doc_len_samples: 480_000, can_play: true });
      }
      return null;
    });
    const stop = await initMarkers();
    selectAllOf(480_000);
    activateMarker(2);
    expect(selectionState().current).toBeNull();
    stop();
  });

  it("goToFirstDropout jumps to the earliest dropout marker, selection untouched (H-67, SPEC-002 AC-7)", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") {
        return [
          marker(1, 50_000, 0, "User", "user"),
          marker(3, 200_000, 480, "Dropout 10 ms", "dropout"),
          marker(2, 144_000, 480, "Dropout 10 ms", "dropout"),
        ];
      }
      if (cmd === "transport_seek") {
        seeks.push(args);
        return transportStateDto({ playhead_samples: 144_000, doc_len_samples: 480_000, can_play: true });
      }
      return null;
    });
    const stop = await initMarkers();
    selectAllOf(480_000); // a pre-existing time selection must survive navigation
    goToFirstDropout();
    expect(markersState().selectedId).toBe(2);
    expect(seeks).toEqual([{ positionSamples: 144_000 }]);
    expect(selectionState().current).toEqual({ startSample: 0, endSample: 480_000 });
    stop();
  });

  it("goToFirstDropout is a no-op with no dropout markers", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [marker(1, 50_000, 0, "User", "user")];
      if (cmd === "transport_seek") {
        seeks.push(args);
        return null;
      }
      return null;
    });
    const stop = await initMarkers();
    goToFirstDropout();
    expect(seeks).toHaveLength(0);
    expect(markersState().selectedId).toBeNull();
    stop();
  });

  it("goToNextMarker/goToPreviousMarker pick the nearest marker past/before the cursor, stopped", async () => {
    mockIPC((cmd) => {
      if (cmd === "markers_get") {
        return [marker(1, 1_000), marker(2, 5_000), marker(3, 9_000)];
      }
      if (cmd === "transport_seek") {
        return transportStateDto({ doc_len_samples: 480_000, can_play: true });
      }
      return null;
    });
    const stop = await initMarkers(); // playheadSamples starts at 0 (resetTransportForTest)

    goToNextMarker();
    expect(markersState().selectedId).toBe(1); // nearest past 0

    // No further transport_state update lands, so playheadSamples stays 0 for the next call too —
    // the marker store itself doesn't move the playhead; it only asks the transport to seek.
    goToPreviousMarker();
    expect(markersState().selectedId).toBe(1); // nothing before 0: unchanged

    stop();
  });

  it("registers the keymap actions marker.add/delete_selected/next/prev", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "markers_get") return [];
      if (cmd === "marker_add") return marker(1, 0);
      return null;
    });
    const stop = await initMarkers();
    calls.length = 0;

    const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

    dispatchAction("marker.add");
    await flush();
    expect(calls).toContain("marker_add");

    dispatchAction("marker.delete_selected");
    await flush();
    expect(calls).toContain("marker_delete");

    dispatchAction("marker.next");
    dispatchAction("marker.prev");

    stop();
  });

  it("refetches the marker list on every document_changed", async () => {
    let markersGetCalls = 0;
    mockIPC(
      (cmd) => {
        if (cmd === "markers_get") {
          markersGetCalls += 1;
          return [marker(1, 0)];
        }
        return null;
      },
      { shouldMockEvents: true },
    );
    const stop = await initMarkers();
    expect(markersGetCalls).toBe(1); // initMarkers' own initial refresh

    await emit("document_changed", DOC_CHANGED);
    // The event handler's refresh() call is async; let its microtask settle.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(markersGetCalls).toBe(2);

    stop();
  });
});

// H-64 (SPEC-009 §2.8): panel filter/sort state, kept independent of the raw `list` (waveform
// rendering, drag magnet targets and navigation must never see the filtered/sorted view).
describe("markers store filter/sort (H-64, SPEC-009 §2.8)", () => {
  async function loadMarkers(list: MarkerDto[]): Promise<void> {
    mockIPC((cmd) => (cmd === "markers_get" ? list : null));
    await initMarkers();
  }

  it("visibleList applies the text and type filters, list stays the full set", async () => {
    await loadMarkers([
      marker(1, 100, 0, "Take 1"),
      marker(2, 200, 50, "Take 2"),
      marker(3, 300, 0, "Intro"),
      marker(4, 400, 0, "Dropout 10 ms", "dropout"),
    ]);

    setMarkerFilterText("take");
    expect(markersState().visibleList.map((m) => m.id)).toEqual([1, 2]);
    expect(markersState().list).toHaveLength(4); // unaffected

    // Both filters apply together (AND): "take" + Dropouts matches neither "Take" marker.
    setMarkerTypeFilter("dropouts");
    expect(markersState().visibleList).toEqual([]);

    // The type filter alone (Dropouts) matches marker 4.
    setMarkerFilterText("");
    expect(markersState().visibleList.map((m) => m.id)).toEqual([4]);
  });

  it("isFiltered reflects either filter being active", () => {
    expect(markersState().isFiltered).toBe(false);
    setMarkerFilterText("x");
    expect(markersState().isFiltered).toBe(true);
    setMarkerFilterText("");
    expect(markersState().isFiltered).toBe(false);
    setMarkerTypeFilter("regions");
    expect(markersState().isFiltered).toBe(true);
  });

  it("setMarkerSort toggles direction on the same column, resets to ascending on a new one", async () => {
    await loadMarkers([marker(1, 300), marker(2, 100), marker(3, 200)]);
    expect(markersState().sortColumn).toBe("start");
    expect(markersState().sortDirection).toBe("asc");
    expect(markersState().visibleList.map((m) => m.id)).toEqual([2, 3, 1]);

    setMarkerSort("start");
    expect(markersState().sortDirection).toBe("desc");
    expect(markersState().visibleList.map((m) => m.id)).toEqual([1, 3, 2]);

    setMarkerSort("name");
    expect(markersState().sortColumn).toBe("name");
    expect(markersState().sortDirection).toBe("asc"); // a new column always starts ascending
  });
});

// H-64 (SPEC-009 §2.6): Delete All / Delete Filtered Markers.
describe("markers store delete all/filtered (H-64, SPEC-009 §2.6, AC-10)", () => {
  it("deleteAllMarkers is a no-op with none, else deletes every marker as one command and clears the panel selection", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [];
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await deleteAllMarkers();
    expect(calls).toHaveLength(0);

    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [marker(1, 0), marker(2, 100), marker(3, 200)];
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await initMarkers();
    selectMarker(2);
    await deleteAllMarkers();
    expect(calls).toEqual([{ ids: [1, 2, 3] }]);
    expect(markersState().list).toEqual([]);
    expect(markersState().selectedId).toBeNull();
  });

  it("deleteFilteredMarkers deletes exactly the filtered rows (AC-10: dropouts filtered, then the rest)", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") {
        return [
          marker(1, 0, 0, "A"),
          marker(2, 100, 0, "B", "dropout"),
          marker(3, 200, 0, "C", "dropout"),
        ];
      }
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await initMarkers();

    setMarkerTypeFilter("dropouts");
    await deleteFilteredMarkers();
    expect(calls).toEqual([{ ids: [2, 3] }]);
    expect(markersState().list.map((m) => m.id)).toEqual([1]);

    // Filtered to nothing: a no-op.
    setMarkerTypeFilter("dropouts");
    await deleteFilteredMarkers();
    expect(calls).toEqual([{ ids: [2, 3] }]); // unchanged
  });

  it("a delete of more than one marker posts notice.markers_deleted with an Undo action; a single delete stays silent", async () => {
    mockIPC((cmd) => {
      if (cmd === "markers_get") return [marker(1, 0), marker(2, 100)];
      if (cmd === "marker_delete") return null;
      return null;
    });
    await initMarkers();
    await deleteAllMarkers();
    expect(noticesState().toasts).toHaveLength(1);
    expect(noticesState().toasts[0]?.key).toBe("notice.markers_deleted");
    expect(noticesState().toasts[0]?.params).toEqual({ count: "2" });
    expect(noticesState().toasts[0]?.action).toEqual({
      label_key: "notice.action.undo",
      id: "undo_marker_delete",
    });

    clearNotices();
    mockIPC((cmd) => {
      if (cmd === "markers_get") return [marker(1, 0)];
      if (cmd === "marker_delete") return null;
      return null;
    });
    await initMarkers();
    await deleteAllMarkers();
    expect(noticesState().toasts).toHaveLength(0);
  });

  it("registers the marker.delete_all keymap action", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return cmd === "markers_get" ? [marker(1, 0)] : null;
    });
    const stop = await initMarkers();
    calls.length = 0;

    dispatchAction("marker.delete_all");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toContain("marker_delete");

    stop();
  });
});
