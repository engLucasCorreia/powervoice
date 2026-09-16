import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { clearActionHandlers, registerAction } from "../shortcuts";
import type { ActionId } from "../shortcuts/actions";
import { initDocument, openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { resetEditForTest } from "../state/edit.svelte";
import { resetInsertSilenceForTest } from "../state/insertSilence.svelte";
import { applyRecordStateForTest, initRecord, onInputTelemetry, resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, selectionState, setSelectionFromResult } from "../state/selection.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { resetTransportForTest, transportState } from "../state/transport.svelte";
import { docDto, recordStateDto, settingsFixture } from "../test/fixtures";
import { initMarkers, resetMarkersForTest } from "../markers/markers.svelte";
import type { MarkerDto } from "../ipc/bindings";
import { RAW_SPP } from "./coords";
import { VXPK_FLAGS } from "./vxpk";
import WaveformView from "./WaveformView.svelte";
import {
  resetWaveformViewForTest,
  setAmplitudeRulerMode,
  verticalZoomState,
} from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
  resetSelectionForTest();
  resetSettingsStateForTest();
  resetTransportForTest();
});

function headerOnlyVxpk(): ArrayBuffer {
  // Header-only VXPK (no buckets) is enough for these smoke tests.
  const buf = new ArrayBuffer(48);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x50);
  dv.setUint8(3, 0x4b);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 48, true);
  return buf;
}

/** A `VXPK` frame carrying `buckets` (min, max) at `samplesPerBucket` (H-10 item 6), `PARTIAL`. */
function vxpkWithBuckets(samplesPerBucket: number, buckets: Array<[number, number]>): ArrayBuffer {
  return vxpkFrame(samplesPerBucket, buckets, true);
}

/** H-71: like {@link vxpkWithBuckets}, but without the `PARTIAL` flag — a response the import
 * job's own peaks give once the covering range has fully committed (SPEC-006 AC-13). */
function vxpkWithBucketsNotPartial(
  samplesPerBucket: number,
  buckets: Array<[number, number]>,
): ArrayBuffer {
  return vxpkFrame(samplesPerBucket, buckets, false);
}

function vxpkFrame(
  samplesPerBucket: number,
  buckets: Array<[number, number]>,
  partial: boolean,
): ArrayBuffer {
  const headerLen = 48;
  const buf = new ArrayBuffer(headerLen + buckets.length * 8);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x56);
  dv.setUint8(1, 0x58);
  dv.setUint8(2, 0x50);
  dv.setUint8(3, 0x4b);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, headerLen, true);
  dv.setUint32(12, partial ? 1 << 1 : 0, true); // PARTIAL
  dv.setUint32(32, samplesPerBucket, true);
  dv.setUint32(36, buckets.length, true);
  dv.setUint32(40, 48_000, true);
  let offset = headerLen;
  for (const [mn, mx] of buckets) {
    dv.setFloat32(offset, mn, true);
    dv.setFloat32(offset + 4, mx, true);
    offset += 8;
  }
  return buf;
}

function frame(flags: number, playheadSample = 0): TelemetryFrame {
  return {
    seq: 0,
    flags,
    playheadSample,
    playheadTimeNs: 0,
    rate: 0,
    outPeakDbfs: Number.NEGATIVE_INFINITY,
    outRmsDbfs: Number.NEGATIVE_INFINITY,
    inPeakDbfs: Number.NEGATIVE_INFINITY,
    inRmsDbfs: Number.NEGATIVE_INFINITY,
    audioRev: 0,
    droppedRtEvents: 0,
  };
}

describe("WaveformView (S1-03)", () => {
  it("shows the empty state with no document open", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="waveform-empty"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-canvas"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("renders the canvas once a document is open", async () => {
    const fixture = docDto();
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        // Header-only VXPK (no buckets) is enough for this smoke test.
        const buf = new ArrayBuffer(48);
        const dv = new DataView(buf);
        dv.setUint8(0, 0x56);
        dv.setUint8(1, 0x58);
        dv.setUint8(2, 0x50);
        dv.setUint8(3, 0x4b);
        dv.setUint16(4, 1, true);
        dv.setUint16(6, 48, true);
        return buf;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    // H-12: the ruler/scrollbar moved to `EditorView` (shared with the spectral pane) — this
    // view only owns the canvas now (see `EditorView.test.ts` for the ruler/scrollbar tests).
    expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-empty"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  // Regression: the canvas container only renders once a document is open, so the size observer
  // must attach then — not at mount (no document yet), which left the viewport 0 and the view blank.
  it("measures the viewport and requests peaks when a document opens after mount", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const peakRequests: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        return docDto();
      }
      if (cmd === "peaks_get") {
        peakRequests.push(args);
        const buf = new ArrayBuffer(48);
        const dv = new DataView(buf);
        dv.setUint8(0, 0x56);
        dv.setUint8(1, 0x58);
        dv.setUint8(2, 0x50);
        dv.setUint8(3, 0x4b);
        dv.setUint16(4, 1, true);
        dv.setUint16(6, 48, true);
        return buf;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    expect(peakRequests).toHaveLength(0);

    try {
      await openDocument("/home/user/take.wav");
      flushSync();
      await Promise.resolve();
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      expect(peakRequests.length).toBeGreaterThan(0);
    } finally {
      unmount(app);
      target.remove();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
    }
  });

  // H-07: the take lives only in the take files and is committed to the document at Stop, so the
  // document is empty (len_samples 0) while recording. The view must still show the canvas and
  // draw the growing take from record_peaks_get, polling instead of the normal peaks_get path.
  it("polls record_peaks_get and renders the canvas while recording, without a committed document", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const heightDescriptor = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight",
    );
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    const livePeakRequests: unknown[] = [];
    // S1-04: the take isn't committed until Stop, so the document is empty while recording.
    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0 });
    const recordingState = recordStateDto({ armed: true, input_open: true, recording: true, monitoring: true });
    mockIPC(
      (cmd, args) => {
        if (cmd === "record_get") {
          return recordingState;
        }
        if (cmd === "record_peaks_get") {
          livePeakRequests.push(args);
          return headerOnlyVxpk();
        }
        if (cmd === "peaks_get") {
          throw new Error("peaks_get must not be called while recording (S1-04: empty document)");
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    const stopRecord = initRecord();
    await emit("document_changed", recordingDoc);
    flushSync();
    await Promise.resolve();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await Promise.resolve();

    try {
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      expect(target.querySelector('[data-testid="waveform-empty"]')).toBeNull();
      expect(livePeakRequests.length).toBeGreaterThan(0);
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });

  // H-71 (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 7): while `document_open`'s import job
  // runs, the document has no committed audio of its own yet either (mirrors H-07's live-take
  // test above) — the view must show the canvas (not the empty state) and poll
  // `import_peaks_get`, never the normal `peaks_get`.
  it("polls import_peaks_get and renders the canvas while an import job is running", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const heightDescriptor = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight",
    );
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    const importPeaksRequests: unknown[] = [];
    mockIPC(
      (cmd, args) => {
        if (cmd === "import_peaks_get") {
          importPeaksRequests.push(args);
          return headerOnlyVxpk();
        }
        if (cmd === "peaks_get") {
          throw new Error("peaks_get must not be called while an import job is running");
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    await emit("import_started", {
      job_id: 1,
      name: "big.wav",
      sample_rate_hz: 48_000,
      len_samples: 48_000 * 3_600,
    });
    flushSync();
    await Promise.resolve();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await Promise.resolve();

    try {
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
      expect(target.querySelector('[data-testid="waveform-empty"]')).toBeNull();
      expect(importPeaksRequests.length).toBeGreaterThan(0);
    } finally {
      unmount(app);
      target.remove();
      stopDocument();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });

  // SPEC-006 AC-13: partial (`NaN`) buckets render as pending immediately, and a `job_progress`
  // tick re-requests and redraws the same range with real values, without a manual scroll/zoom —
  // and once the job is no longer running, the view stops polling `import_peaks_get` at all.
  it("AC-13: re-requests real peaks on job_progress and stops polling once the import ends", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const heightDescriptor = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight",
    );
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    let respondPartial = true;
    let importPeaksCalls = 0;
    let peaksGetCalls = 0;
    mockIPC(
      (cmd) => {
        if (cmd === "import_peaks_get") {
          importPeaksCalls++;
          return respondPartial
            ? vxpkWithBuckets(64, [
                [-0.5, 0.5],
                [Number.NaN, Number.NaN],
              ])
            : vxpkWithBucketsNotPartial(64, [
                [-0.5, 0.5],
                [-0.25, 0.25],
              ]);
        }
        if (cmd === "peaks_get") {
          peaksGetCalls++;
          return headerOnlyVxpk();
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    await emit("import_started", {
      job_id: 1,
      name: "big.wav",
      sample_rate_hz: 48_000,
      len_samples: 48_000 * 60,
    });
    flushSync();
    await Promise.resolve();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await Promise.resolve();
    await Promise.resolve();

    try {
      const afterStart = importPeaksCalls;
      expect(afterStart).toBeGreaterThan(0);
      expect(peaksGetCalls).toBe(0);

      // The import has committed more: the same range now reads real, non-PARTIAL values.
      respondPartial = false;
      await emit("job_progress", { job_id: 1, kind: "import", state: "running", fraction: 0.5 });
      flushSync();
      await Promise.resolve();
      await Promise.resolve();
      expect(importPeaksCalls).toBeGreaterThan(afterStart);

      // The import finished (document_changed swaps in the real document): the view stops
      // polling import_peaks_get and resumes the normal peaks_get path for the now-open document.
      const finishedDoc = docDto({ name: "big.wav", len_samples: 48_000 * 60, audio_rev: 1 });
      await emit("job_progress", { job_id: 1, kind: "import", state: "done", fraction: 1 });
      await emit("document_changed", finishedDoc);
      flushSync();
      await Promise.resolve();
      await Promise.resolve();
      const afterDone = importPeaksCalls;
      expect(peaksGetCalls).toBeGreaterThan(0);

      // No further import_peaks_get polling once the job is no longer running, even though the
      // view keeps redrawing (the document is open and its own peaks are cached already).
      flushSync();
      await Promise.resolve();
      expect(importPeaksCalls).toBe(afterDone);
    } finally {
      unmount(app);
      target.remove();
      stopDocument();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });

  // H-28 item 1: a take recording into an empty document must not blow Svelte's effect-update
  // depth (`effect_update_depth_exceeded`). This was seen from the follow effects pre-H-27; H-27
  // unified record-follow/playback-follow behind one `ViewportWriter` (`viewportFollow.ts`), and
  // this reproduces the exact scenario (a burst of telemetry ticks growing the take, each one
  // re-running the zoom-to-fit effect and the follow effect in the same flush) on current main to
  // confirm it no longer reproduces.
  it("does not throw effect_update_depth_exceeded while a take records into an empty document", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 800,
    });
    const heightDescriptor = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "clientHeight",
    );
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 200,
    });

    // S1-04: the take isn't committed until Stop, so the document is empty while recording.
    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0 });
    const recordingState = recordStateDto({ armed: true, input_open: true, recording: true, monitoring: true });
    mockIPC(
      (cmd) => {
        if (cmd === "record_get") {
          return recordingState;
        }
        if (cmd === "record_peaks_get") {
          return headerOnlyVxpk();
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    const stopDocument = await initDocument();
    const stopRecord = initRecord();
    await emit("document_changed", recordingDoc);
    flushSync();
    await Promise.resolve();

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    await Promise.resolve();

    try {
      // A burst of telemetry ticks growing the take — each one updates `rec.elapsedSamples`,
      // which drives the H-07 zoom-to-fit effect (writes `startSample`/`samplesPerPixel`), which
      // in turn re-runs the shared follow effect (reads both). If either effect fed back into
      // itself without the `ViewportWriter`/reference-stable-`INITIAL_*_STATE` guards, this loop
      // would throw `effect_update_depth_exceeded` on some tick.
      expect(() => {
        for (let i = 1; i <= 200; i++) {
          onInputTelemetry(frame(VXTM_FLAGS.RECORDING, i * 4_800));
          flushSync();
        }
      }).not.toThrow();
      expect(target.querySelector('[data-testid="waveform-canvas"]')).not.toBeNull();
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });

  // H-10 item 6: `LivePeaks` doubles its bucket size once a multi-hour take decimates
  // (crates/engine/src/capture.rs). The view must size its *next* request from the response's
  // own `samplesPerBucket`, not the fixed starting constant — otherwise, once the backend has
  // decimated, a stale (smaller) assumed bucket size would under-cover the take's current span.
  it("sizes the next live-peaks request from the response's own bucket size, not a fixed 256", async () => {
    const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
    const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 200 });

    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0 });
    const recordingState = recordStateDto({ armed: true, input_open: true, recording: true, monitoring: true });
    const requests: Array<{ startBucket: number; count: number }> = [];
    mockIPC(
      (cmd, args) => {
        if (cmd === "record_get") {
          return recordingState;
        }
        if (cmd === "record_peaks_get") {
          requests.push(args as { startBucket: number; count: number });
          // A decimated take: bucket size doubled from the client's starting 256 to 512.
          return vxpkWithBuckets(512, [[-0.5, 0.5]]);
        }
        return null;
      },
      { shouldMockEvents: true },
    );

    vi.useFakeTimers();
    try {
      const stopDocument = await initDocument();
      const stopRecord = initRecord();
      await emit("document_changed", recordingDoc);
      await vi.advanceTimersByTimeAsync(0);
      flushSync();

      // 100 000 samples elapsed: the first request still assumes the starting 256 spb.
      onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 100_000), 0);
      flushSync();

      const target = document.createElement("div");
      document.body.appendChild(target);
      const app = mount(WaveformView, { target });
      flushSync();
      await vi.advanceTimersByTimeAsync(0);

      try {
        expect(requests.length).toBeGreaterThan(0);
        expect(requests[0]?.count).toBe(Math.ceil(100_000 / 256) + 2);

        // The next poll (100 ms later) must use the *decimated* 512 spb the response just
        // reported — not the stale starting constant.
        await vi.advanceTimersByTimeAsync(100);
        const next = requests.at(-1);
        expect(next?.count).toBe(Math.ceil(100_000 / 512) + 2);
      } finally {
        unmount(app);
        target.remove();
        stopRecord();
        stopDocument();
      }
    } finally {
      vi.useRealTimers();
      if (widthDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
      }
      if (heightDescriptor) {
        Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
      }
    }
  });
});

// S2-01, SPEC-006 §2.9: click-drag creates a selection at exact document samples; a plain click
// (no movement) clears it instead. The viewport is zoomed to fit exactly (8 000 samples over
// 800 px = 10 samples/px, `startSample = 0`), so `sample = round(clientX * 10)` and jsdom's
// zeroed `getBoundingClientRect` makes `clientX` itself the in-canvas pixel.
describe("WaveformView selection (S2-01)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => px,
    });
  }

  afterEach(() => {
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  async function openFixture(lenSamples: number): Promise<void> {
    const fixture = docDto({ len_samples: lenSamples });
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        const buf = new ArrayBuffer(48);
        const dv = new DataView(buf);
        dv.setUint8(0, 0x56);
        dv.setUint8(1, 0x58);
        dv.setUint8(2, 0x50);
        dv.setUint8(3, 0x4b);
        dv.setUint16(4, 1, true);
        dv.setUint16(6, 48, true);
        return buf;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
  }

  it("a click-drag creates a selection at exact document samples", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 10, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 50, bubbles: true }),
    );
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 100, endSample: 500 });

    unmount(app);
    target.remove();
  });

  it("a plain click (no drag) clears the selection", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    // First, an actual drag to have something to clear.
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).not.toBeNull();

    // Then a plain click (mousedown/up at the same point, no movement) clears it.
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 20, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 20, bubbles: true }));
    flushSync();
    expect(selectionState().current).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Ctrl+A selects the entire document", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    // Ctrl+A -> "waveform.select_all" is exercised in `keymap.test.ts`; here we only check
    // `WaveformView`'s registered handler, so no `attachKeymap()` listener is needed.
    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.select_all");
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 0, endSample: 8_000 });

    unmount(app);
    target.remove();
  });

  it("Esc clears the selection", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).not.toBeNull();

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.deselect");
    flushSync();
    expect(selectionState().current).toBeNull();

    unmount(app);
    target.remove();
  });
});

// H-57, SPEC-009 §2.5: dragging a marker's flag on the waveform. Same fixture geometry as the
// selection describe block above (8 000 samples / 800 px = 10 samples/px, startSample 0), so
// `clientX` is the in-canvas pixel and `sample = round(clientX * 10)`.
describe("WaveformView marker drag (H-57, SPEC-009 §2.5)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => px,
    });
  }

  afterEach(() => {
    resetMarkersForTest();
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  function marker(id: number, pos: number, len = 0): MarkerDto {
    return { id, pos_samples: pos, len_samples: len, name: `m${id}`, kind: "user" };
  }

  async function openFixtureWithMarkers(lenSamples: number, markers: MarkerDto[]): Promise<() => void> {
    const fixture = docDto({ len_samples: lenSamples });
    const setRangeCalls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        const buf = new ArrayBuffer(48);
        const dv = new DataView(buf);
        dv.setUint8(0, 0x56);
        dv.setUint8(1, 0x58);
        dv.setUint8(2, 0x50);
        dv.setUint8(3, 0x4b);
        dv.setUint16(4, 1, true);
        dv.setUint16(6, 48, true);
        return buf;
      }
      if (cmd === "markers_get") {
        return markers;
      }
      if (cmd === "marker_set_range") {
        setRangeCalls.push(args);
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    await initMarkers();
    return () => setRangeCalls;
  }

  it("dragging a point marker's flag past the threshold commits one marker_set_range (move)", async () => {
    stubWidth(800);
    const getCalls = await openFixtureWithMarkers(8_000, [marker(1, 1_000)]);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    // The flag sits at px 100 (1 000 samples / 10 samples-per-px). Drag it to px 150 (1 500).
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 100, clientY: 0, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 150, clientY: 0, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 150, clientY: 0, bubbles: true }));
    flushSync();

    expect(getCalls()).toEqual([{ id: 1, posSamples: 1_500, lenSamples: 0, kind: "move" }]);

    unmount(app);
    target.remove();
  });

  it("releasing before the drag threshold is a click: no marker_set_range, and it activates the marker", async () => {
    stubWidth(800);
    const getCalls = await openFixtureWithMarkers(8_000, [marker(1, 1_000, 500)]);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    selectAllOfDocument();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 100, clientY: 0, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 101, clientY: 0, bubbles: true }));
    flushSync();

    expect(getCalls()).toEqual([]);
    // Activation: a region marker's range replaces the pre-existing selection.
    expect(selectionState().current).toEqual({ startSample: 1_000, endSample: 1_500 });

    unmount(app);
    target.remove();
  });

  it("Esc mid-drag cancels: no marker_set_range is issued", async () => {
    stubWidth(800);
    const getCalls = await openFixtureWithMarkers(8_000, [marker(1, 1_000)]);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 100, clientY: 0, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 150, clientY: 0, bubbles: true }));
    flushSync();

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.deselect");
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 150, clientY: 0, bubbles: true }));
    flushSync();

    expect(getCalls()).toEqual([]);

    unmount(app);
    target.remove();
  });

  function selectAllOfDocument(): void {
    setSelectionFromResult([0, 8_000]);
  }
});

// T-701/A-020: keyboard nudge (Left/Right Arrow) and extend (Shift+Left/Right Arrow).
describe("WaveformView keyboard nudge/extend (T-701/A-020)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => px,
    });
  }

  afterEach(() => {
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  /** A raw `VXPK` frame (mirrors `zeroCrossing.test.ts`'s own helper — kept local since that
   * file's helper isn't exported). */
  function buildRawVxpk(opts: { audioRev: number; startSample: number; samples: number[] }): ArrayBuffer {
    const buf = new ArrayBuffer(48 + opts.samples.length * 4);
    const dv = new DataView(buf);
    dv.setUint8(0, 0x56);
    dv.setUint8(1, 0x58);
    dv.setUint8(2, 0x50);
    dv.setUint8(3, 0x4b);
    dv.setUint16(4, 1, true);
    dv.setUint16(6, 48, true);
    dv.setUint32(8, 1, true);
    dv.setUint32(12, VXPK_FLAGS.RAW, true);
    dv.setUint32(16, opts.audioRev >>> 0, true);
    dv.setUint32(24, opts.startSample >>> 0, true);
    dv.setUint32(32, 1, true);
    dv.setUint32(36, opts.samples.length, true);
    dv.setUint32(40, 48_000, true);
    opts.samples.forEach((v, i) => dv.setFloat32(48 + i * 4, v, true));
    return buf;
  }

  /** `snapToZeroCrossing` off by default (no settings loaded, matching the S2-01 fixtures above);
   * `rawSamples`, when given, answers the RAW (`spp === RAW_SPP`) request with a real crossing —
   * everything else (the view's own bucketed peaks) gets the generic header-only response. */
  async function openFixture(lenSamples: number, rawSamples?: number[]): Promise<void> {
    const fixture = docDto({ len_samples: lenSamples });
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "settings_get") {
        return settingsFixture({ snap_to_zero_crossing: rawSamples !== undefined });
      }
      if (cmd === "transport_seek") {
        const positionSamples = (args as { positionSamples: number }).positionSamples;
        return { ...transportState().state, playhead_samples: positionSamples, playing: false };
      }
      if (cmd === "peaks_get") {
        const request = (args as { request: { spp: number } }).request;
        if (rawSamples && request.spp === RAW_SPP) {
          return buildRawVxpk({ audioRev: fixture.audio_rev, startSample: 0, samples: rawSamples });
        }
        return headerOnlyVxpk();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    if (rawSamples !== undefined) {
      await loadSettings();
    }
  }

  async function mountView(): Promise<{ app: object; target: HTMLElement }> {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    return { app, target };
  }

  it("Left/Right Arrow nudges the whole selection, length unchanged, no snap", async () => {
    stubWidth(800);
    await openFixture(8_000); // samplesPerPixel = 10 (zoom-full fit)
    const { app, target } = await mountView();
    const container = target.querySelector('[data-testid="waveform-canvas"]')!.parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 100, endSample: 500 });

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("selection.nudge_right");
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 110, endSample: 510 });

    dispatchAction("selection.nudge_left");
    dispatchAction("selection.nudge_left");
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 90, endSample: 490 });

    unmount(app);
    target.remove();
  });

  it("Left/Right Arrow nudges the cursor (seeks) when there is no selection", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const { app, target } = await mountView();
    expect(selectionState().current).toBeNull();
    // `document.svelte.ts` fires its own (fire-and-forget) restore-cursor seek on open — let that
    // settle before capturing the baseline, so this test's own nudges aren't racing it.
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    const before = transportState().playheadSamples;

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("selection.nudge_right");
    await vi.waitFor(() => {
      flushSync();
      expect(transportState().playheadSamples).toBe(before + 10);
    });

    dispatchAction("selection.nudge_left");
    await vi.waitFor(() => {
      flushSync();
      expect(transportState().playheadSamples).toBe(before);
    });

    unmount(app);
    target.remove();
  });

  it("Shift+Right Arrow extends a fresh selection from the cursor (no selection yet)", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const { app, target } = await mountView();
    expect(selectionState().current).toBeNull();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    const cursor = transportState().playheadSamples;

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("selection.extend_right");
    flushSync();
    // cursor .. cursor + one step (10 samples).
    expect(selectionState().current).toEqual({ startSample: cursor, endSample: cursor + 10 });

    unmount(app);
    target.remove();
  });

  it("Shift+Right Arrow grows the end edge, Shift+Left Arrow grows the start edge (no snap)", async () => {
    stubWidth(800);
    await openFixture(8_000);
    const { app, target } = await mountView();
    const container = target.querySelector('[data-testid="waveform-canvas"]')!.parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 100, endSample: 500 });

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("selection.extend_right");
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 100, endSample: 510 });

    dispatchAction("selection.extend_left");
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 90, endSample: 510 });

    unmount(app);
    target.remove();
  });

  it("Shift+Right Arrow snaps the extended edge to the nearest zero crossing when the setting is on (T-206)", async () => {
    stubWidth(800);
    // Filler at 0.5 everywhere (never itself a crossing), with a real sign change at
    // 507 -> 508 — closer to the unsnapped edge (510) than to the selection's other edge (100), so
    // the result is unambiguous.
    const samples = new Array(1_023).fill(0.5);
    samples[507] = 0.2;
    samples[508] = -0.2;
    await openFixture(8_000, samples);
    const { app, target } = await mountView();
    const container = target.querySelector('[data-testid="waveform-canvas"]')!.parentElement as HTMLElement;
    container.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointermove", { clientX: 50, bubbles: true }));
    container.dispatchEvent(new PointerEvent("pointerup", { clientX: 50, bubbles: true }));
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 100, endSample: 500 });

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("selection.extend_right"); // unsnapped target: 500 + 10 = 510
    // Snapped to the 507/508 crossing (508 is the nearer side found first), not the raw 510 — and
    // the *other* (fixed) edge, 100, is untouched.
    await vi.waitFor(() => {
      flushSync();
      expect(selectionState().current).toEqual({ startSample: 100, endSample: 508 });
    });

    unmount(app);
    target.remove();
  });
});

// H-35 (SPEC-006 §2.4/§2.6): Zoom to Selection, Zoom Full, vertical (amplitude) zoom.
describe("WaveformView zoom commands (H-35)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => px });
  }

  afterEach(() => {
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  async function openFixture(lenSamples: number): Promise<void> {
    const fixture = docDto({ len_samples: lenSamples });
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        return headerOnlyVxpk();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
  }

  /** Mounts with a captured, externally-readable `startSample`/`samplesPerPixel` (Svelte 5's
   * documented `mount()` pattern for reading a `$bindable` prop from outside a parent template —
   * a plain getter/setter pair, no runes needed). */
  function mountBoundView(): {
    app: object;
    target: HTMLElement;
    viewport: () => { startSample: number; samplesPerPixel: number };
  } {
    const target = document.createElement("div");
    document.body.appendChild(target);
    let startSample = 0;
    let samplesPerPixel = 1;
    const app = mount(WaveformView, {
      target,
      props: {
        get startSample() {
          return startSample;
        },
        set startSample(v: number) {
          startSample = v;
        },
        get samplesPerPixel() {
          return samplesPerPixel;
        },
        set samplesPerPixel(v: number) {
          samplesPerPixel = v;
        },
      },
    });
    flushSync();
    return { app, target, viewport: () => ({ startSample, samplesPerPixel }) };
  }

  it("Zoom to Selection fits the selection exactly, per SPEC-006 §2.6's formula", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target, viewport } = mountBoundView();
    expect(viewport()).toEqual({ startSample: 0, samplesPerPixel: 125 }); // zoom-full at open

    setSelectionFromResult([10_000, 14_000]);
    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.zoom_to_selection");
    flushSync();

    expect(viewport()).toEqual({ startSample: 10_000, samplesPerPixel: 5 }); // 4_000 / 800

    unmount(app);
    target.remove();
  });

  it("Zoom to Selection is a no-op with no selection (SPEC-006 §2.6)", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target, viewport } = mountBoundView();
    const before = viewport();

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.zoom_to_selection");
    flushSync();

    expect(viewport()).toEqual(before);

    unmount(app);
    target.remove();
  });

  it("Zoom Full fits the whole document, startSample 0", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target, viewport } = mountBoundView();

    setSelectionFromResult([10_000, 14_000]);
    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.zoom_to_selection");
    flushSync();
    expect(viewport().samplesPerPixel).toBe(5); // zoomed into the selection first

    dispatchAction("waveform.zoom_full");
    flushSync();
    expect(viewport()).toEqual({ startSample: 0, samplesPerPixel: 125 });

    unmount(app);
    target.remove();
  });

  it("Alt+= / Alt+- / Alt+0 zoom vertical amplitude in/out/reset (SPEC-006 §2.4)", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target } = mountBoundView();
    expect(verticalZoomState().current).toBe(1);

    const { dispatchAction } = await import("../shortcuts");
    dispatchAction("waveform.zoom_in_vertical");
    flushSync();
    expect(verticalZoomState().current).toBe(2);

    dispatchAction("waveform.zoom_in_vertical");
    flushSync();
    expect(verticalZoomState().current).toBe(4);

    dispatchAction("waveform.zoom_out_vertical");
    flushSync();
    expect(verticalZoomState().current).toBe(2);

    dispatchAction("waveform.zoom_reset_vertical");
    flushSync();
    expect(verticalZoomState().current).toBe(1);

    unmount(app);
    target.remove();
  });

  it("Alt+wheel zooms vertical amplitude, centred on the ruler center line", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target } = mountBoundView();

    const container = target.querySelector('[data-testid="waveform-canvas"]')!.parentElement as HTMLElement;
    container.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, altKey: true, bubbles: true, cancelable: true }));
    flushSync();
    expect(verticalZoomState().current).toBe(2);

    container.dispatchEvent(new WheelEvent("wheel", { deltaY: 100, altKey: true, bubbles: true, cancelable: true }));
    flushSync();
    expect(verticalZoomState().current).toBe(1);

    unmount(app);
    target.remove();
  });

  // H-72 (SPEC-006 §2.4): the amplitude ruler actually switches between dBFS and percent labels
  // when `amplitudeRulerModeState` changes — the previous agent's `amplitudeTicksPercent` had no
  // caller in `WaveformView.svelte` at all.
  it("the amplitude ruler switches from dBFS to percent labels (H-72)", async () => {
    stubWidth(800);
    await openFixture(100_000);
    const { app, target } = mountBoundView();

    const ruler = target.querySelector('[data-testid="waveform-amp-ruler"]')!;
    expect(ruler.querySelector(".unit")?.textContent).toBe("dBFS");
    expect(ruler.textContent).toContain("0"); // a 0 dBFS tick, unlabeled with a % sign

    setAmplitudeRulerMode("percent");
    flushSync();

    expect(ruler.querySelector(".unit")).toBeNull(); // H-72: no separate unit box in percent mode
    expect(ruler.textContent).toContain("0%");
    expect(ruler.textContent).toContain("100%");

    unmount(app);
    target.remove();
  });
});

// H-66 (SPEC-008 §2.11): the waveform's right-click menu — same seven ops/enablement as
// EditMenu.svelte, opened at the pointer or (keyboard) under the canvas container, never
// disturbing the selection or a marker drag.
describe("WaveformView right-click menu (H-66)", () => {
  const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");

  function stubWidth(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => px,
    });
  }

  afterEach(() => {
    resetMarkersForTest();
    resetEditForTest();
    resetInsertSilenceForTest();
    if (widthDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    }
  });

  const OP_IDS = ["cut", "copy", "paste", "delete", "trim", "silence", "insert-silence"];

  function marker(id: number, pos: number, len = 0): MarkerDto {
    return { id, pos_samples: pos, len_samples: len, name: `m${id}`, kind: "user" };
  }

  async function openFixtureDocument(
    lenSamples: number,
    markers: MarkerDto[] = [],
  ): Promise<() => unknown[]> {
    const fixture = docDto({ len_samples: lenSamples });
    const setRangeCalls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        return fixture;
      }
      if (cmd === "peaks_get") {
        return headerOnlyVxpk();
      }
      if (cmd === "markers_get") {
        return markers;
      }
      if (cmd === "marker_set_range") {
        setRangeCalls.push(args);
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    await initMarkers();
    return () => setRangeCalls;
  }

  function mountView(): { target: HTMLElement; app: ReturnType<typeof mount>; container: HTMLElement } {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    const container = target.querySelector('[data-testid="waveform-canvas"]')!
      .parentElement as HTMLElement;
    return { target, app, container };
  }

  function openContextMenu(container: HTMLElement, clientX = 40, clientY = 20): void {
    container.dispatchEvent(
      new MouseEvent("contextmenu", { clientX, clientY, bubbles: true, cancelable: true }),
    );
    flushSync();
  }

  it("has no document, no canvas, nothing to right-click — the empty state shows instead", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(WaveformView, { target });
    flushSync();
    expect(target.querySelector('[data-testid="waveform-empty"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="waveform-context-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("opens the seven SPEC-008 §2.11 ops, same order/labels/shortcuts as the Edit menu", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    const { target, app, container } = mountView();

    openContextMenu(container);
    expect(target.querySelector('[data-testid="waveform-context-menu"]')).not.toBeNull();
    for (const id of OP_IDS) {
      expect(
        target.querySelector(`[data-testid="waveform-menu-${id}"]`),
        id,
      ).not.toBeNull();
    }
    expect(target.querySelector('[data-testid="waveform-menu-cut"] .shortcut')?.textContent).toBe(
      "Ctrl+X",
    );
    expect(target.querySelector('[data-testid="waveform-menu-trim"] .shortcut')?.textContent).toBe(
      "Ctrl+T",
    );
    // Silence/Insert Silence have no default binding (menu only, SPEC-008 §2.11 table).
    expect(target.querySelector('[data-testid="waveform-menu-silence"] .shortcut')).toBeNull();
    expect(target.querySelector('[data-testid="waveform-menu-insert-silence"] .shortcut')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Cut/Copy/Paste/Delete/Trim/Silence are disabled with no selection and an empty clipboard; Insert Silence is enabled", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    const { target, app, container } = mountView();

    openContextMenu(container);
    for (const id of ["cut", "copy", "paste", "delete", "trim", "silence"]) {
      expect(
        target.querySelector<HTMLButtonElement>(`[data-testid="waveform-menu-${id}"]`)?.disabled,
        id,
      ).toBe(true);
    }
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="waveform-menu-insert-silence"]')
        ?.disabled,
    ).toBe(false);

    unmount(app);
    target.remove();
  });

  it("Cut/Copy/Delete/Trim/Silence become enabled with a non-empty selection", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    setSelectionFromResult([0, 100]);
    const { target, app, container } = mountView();

    openContextMenu(container);
    for (const id of ["cut", "copy", "delete", "trim", "silence"]) {
      expect(
        target.querySelector<HTMLButtonElement>(`[data-testid="waveform-menu-${id}"]`)?.disabled,
        id,
      ).toBe(false);
    }
    // Paste still needs a non-empty clipboard, which a selection alone doesn't provide.
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="waveform-menu-paste"]')?.disabled,
    ).toBe(true);

    unmount(app);
    target.remove();
  });

  it("disables all seven while recording, even with a selection (SPEC-008 §2.2)", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    setSelectionFromResult([0, 100]);
    applyRecordStateForTest({ recording: true });
    const { target, app, container } = mountView();

    openContextMenu(container);
    for (const id of OP_IDS) {
      expect(
        target.querySelector<HTMLButtonElement>(`[data-testid="waveform-menu-${id}"]`)?.disabled,
        id,
      ).toBe(true);
    }

    unmount(app);
    target.remove();
  });

  it("every item dispatches the same keymap action as its Edit-menu equivalent", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    setSelectionFromResult([0, 100]);
    const { target, app, container } = mountView();

    const cases: Array<[string, ActionId]> = [
      ["waveform-menu-cut", "edit.cut"],
      ["waveform-menu-copy", "edit.copy"],
      ["waveform-menu-delete", "edit.delete"],
      ["waveform-menu-trim", "edit.trim"],
    ];
    for (const [testid, action] of cases) {
      const handler = vi.fn();
      const unregister = registerAction(action, handler);
      openContextMenu(container);
      target.querySelector<HTMLButtonElement>(`[data-testid="${testid}"]`)!.click();
      expect(handler, `${testid} -> ${action}`).toHaveBeenCalledOnce();
      unregister();
    }

    unmount(app);
    target.remove();
  });

  it("opens at the pointer's client coordinates on a mouse right-click", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    const { target, app, container } = mountView();

    openContextMenu(container, 123, 45);
    const menu = target.querySelector<HTMLElement>('[data-testid="waveform-context-menu"]');
    expect(menu).not.toBeNull();
    // Popover positions a point anchor's left/top to (roughly) the pointer (`placement.ts`
    // flip/shift only kicks in near the viewport edge) — not zero, i.e. not the empty-anchor
    // fallback and not left at the container's own origin.
    expect(menu?.style.left).not.toBe("0px");

    unmount(app);
    target.remove();
  });

  it("opens under the canvas container when the browser fires contextmenu from the keyboard (clientX/Y 0,0)", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    const { target, app, container } = mountView();

    openContextMenu(container, 0, 0);
    expect(target.querySelector('[data-testid="waveform-context-menu"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("Escape closes the menu", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    const { target, app, container } = mountView();

    openContextMenu(container);
    expect(target.querySelector('[data-testid="waveform-context-menu"]')).not.toBeNull();
    target
      .querySelector('[data-testid="waveform-context-menu"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="waveform-context-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("a right-click (and a right-button drag) leaves the selection untouched", async () => {
    stubWidth(800);
    await openFixtureDocument(8_000);
    setSelectionFromResult([0, 100]);
    const { app, container } = mountView();

    container.dispatchEvent(
      new PointerEvent("pointerdown", { button: 2, clientX: 10, clientY: 20, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointermove", { button: 2, clientX: 300, clientY: 20, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointerup", { button: 2, clientX: 300, clientY: 20, bubbles: true }),
    );
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 0, endSample: 100 });

    unmount(app);
  });

  it("right-clicking (and dragging) a marker's flag does not move it or start a drag (H-57)", async () => {
    stubWidth(800);
    // The flag sits at px 100 (1 000 samples / 10 samples-per-px, SPEC-006 zoom-full math).
    const getCalls = await openFixtureDocument(8_000, [marker(1, 1_000)]);
    const { app, container } = mountView();

    container.dispatchEvent(
      new PointerEvent("pointerdown", { button: 2, clientX: 100, clientY: 0, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointermove", { button: 2, clientX: 150, clientY: 0, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointerup", { button: 2, clientX: 150, clientY: 0, bubbles: true }),
    );
    flushSync();

    expect(getCalls()).toEqual([]);

    unmount(app);
  });
});
