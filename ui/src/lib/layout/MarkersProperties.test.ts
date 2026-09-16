import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import type { MarkerDto } from "../ipc/bindings";
import { clearActionHandlers } from "../shortcuts";
import { initMarkers, resetMarkersForTest } from "../markers/markers.svelte";
import { resetWaveformViewForTest, setTimeRulerFormat } from "../state/waveformView.svelte";
import { docDto } from "../test/fixtures";
import MarkersProperties from "./MarkersProperties.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetMarkersForTest();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("MarkersProperties marker times follow time_ruler_format (T-206, SPEC-006 §2.5)", () => {
  it("switches marker position/length text between timecode/samples/seconds", async () => {
    const marker: MarkerDto = { id: 1, pos_samples: 48_000, len_samples: 24_000, name: "m1", kind: "user" };
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return docDto({ sample_rate_hz: 48_000, len_samples: 480_000 });
      }
      if (cmd === "markers_get") {
        return [marker];
      }
      return null;
    });
    await openDocument("/home/user/take.wav");
    const teardownMarkers = await initMarkers();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(MarkersProperties, { target });
    await settle();

    const times = () => Array.from(target.querySelectorAll(".marker-time")).map((el) => el.textContent?.trim());
    const durations = () =>
      Array.from(target.querySelectorAll(".marker-duration")).map((el) => el.textContent?.trim());

    expect(times()).toEqual(["00:00:01.000"]); // default: timecode
    expect(durations()).toEqual(["00:00:00.500"]);

    setTimeRulerFormat("samples");
    await settle();
    expect(times()).toEqual(["48000"]);
    expect(durations()).toEqual(["24000"]);

    setTimeRulerFormat("seconds");
    await settle();
    expect(times()).toEqual(["1.000000"]);
    expect(durations()).toEqual(["0.500000"]);

    unmount(app);
    target.remove();
    teardownMarkers();
  });
});
