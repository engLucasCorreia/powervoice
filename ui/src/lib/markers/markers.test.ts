import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { MarkerDto } from "../ipc/bindings";
import { clearActionHandlers, dispatchAction } from "../shortcuts";
import { clearNotices } from "../state/notices.svelte";
import { resetSelectionForTest, selectAllOf } from "../state/selection.svelte";
import { resetTransportForTest } from "../state/transport.svelte";
import { docDto, transportStateDto } from "../test/fixtures";
import {
  addMarker,
  deleteSelectedMarker,
  goToNextMarker,
  goToPreviousMarker,
  initMarkers,
  jumpToMarker,
  markersState,
  renameMarker,
  resetMarkersForTest,
  selectMarker,
  setMarkerRange,
} from "./markers.svelte";

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

function marker(id: number, pos: number, len = 0, name = `m${id}`): MarkerDto {
  return { id, pos_samples: pos, len_samples: len, name };
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
