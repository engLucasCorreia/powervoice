import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { initDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { initRecord, onInputTelemetry, resetRecordForTest } from "../state/record.svelte";
import { docDto, recordStateDto } from "../test/fixtures";
import { resetWaveformViewForTest, waveformViewApi } from "../state/waveformView.svelte";
import EditorView from "../layout/EditorView.svelte";

// H-114: the owner reported the live-recording waveform "looks dead" for about the first 10 s of
// a take. The data path (LIVE_PEAKS_SPB, LIVE_POLL_MS, the capture-writer's 20 ms drain) was
// already real-time — the actual cause was `WaveformView`'s H-07 live-zoom effect, which floored
// the view at `LIVE_MIN_WINDOW_SECONDS` (10 s): a brand new take was zoomed to fit a 10 s window
// from sample 0, so the first spoken word (well under 1 s in) filled only a few percent of the
// canvas width. These tests mount the real `EditorView` (exercising the same two-way
// `bind:startSample`/`bind:samplesPerPixel` binding H-83 fixed) and read the live-zoomed
// `samplesPerPixel` back through `waveformViewApi()`, so a regression to a large floor — or any
// other change that shrinks the visible fraction of an early take — fails loudly here rather than
// only being noticed by feel.

const RATE_HZ = 48_000;
const VIEWPORT_PX = 1_600;

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
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

function headerOnlyVxpk(): ArrayBuffer {
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

afterEach(() => {
  clearMocks();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
});

/** Fraction of the viewport the take currently occupies, given the live-zoomed `samplesPerPixel`
 * the effect under test just wrote. */
function visibleFraction(elapsedSamples: number): number {
  const spp = waveformViewApi().samplesPerPixel;
  return spp > 0 ? Math.min(1, elapsedSamples / spp / VIEWPORT_PX) : 0;
}

describe("H-07/H-114 live-zoom window while recording into a new, empty document", () => {
  it("shows a clearly visible fraction of the take within the first half second, not a sliver", async () => {
    stubSize(VIEWPORT_PX, 900);
    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0, sample_rate_hz: RATE_HZ });
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
    const app = mount(EditorView, { target });
    flushSync();
    await Promise.resolve();

    try {
      // A first word at 0.3-0.5 s in (a realistic pre-roll of near-silence before speech starts)
      // must already read as an obviously growing take, not a barely-visible sliver: at LIVE_
      // MIN_WINDOW_SECONDS = 10 this was 3-5 % of the viewport (the reported bug); it must now be
      // at least a fifth of it.
      onInputTelemetry(frame(VXTM_FLAGS.RECORDING, Math.round(0.5 * RATE_HZ)));
      flushSync();
      expect(visibleFraction(Math.round(0.5 * RATE_HZ))).toBeGreaterThanOrEqual(0.2);

      // By 1 s the take should already read as at least half-grown...
      onInputTelemetry(frame(VXTM_FLAGS.RECORDING, RATE_HZ));
      flushSync();
      expect(visibleFraction(RATE_HZ)).toBeGreaterThanOrEqual(0.4);

      // ...and the view must never stay floored so wide that a multi-second take still looks
      // barely started (the inverse regression: too large a floor).
      onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 3 * RATE_HZ));
      flushSync();
      expect(visibleFraction(3 * RATE_HZ)).toBeGreaterThanOrEqual(0.9);
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
    }
  });

  it("keeps startSample at 0 and samplesPerPixel monotonically non-decreasing as the take grows past the floor", async () => {
    stubSize(VIEWPORT_PX, 900);
    const recordingDoc = docDto({ name: null, path: null, len_samples: 0, audio_rev: 0, sample_rate_hz: RATE_HZ });
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
    const app = mount(EditorView, { target });
    flushSync();
    await Promise.resolve();

    try {
      let lastSpp = 0;
      for (const seconds of [0.1, 0.5, 1, 2, 3, 5, 10, 20]) {
        onInputTelemetry(frame(VXTM_FLAGS.RECORDING, Math.round(seconds * RATE_HZ)));
        flushSync();
        expect(waveformViewApi().startSample).toBe(0);
        expect(waveformViewApi().samplesPerPixel).toBeGreaterThanOrEqual(lastSpp);
        lastSpp = waveformViewApi().samplesPerPixel;
      }
    } finally {
      unmount(app);
      target.remove();
      stopRecord();
      stopDocument();
    }
  });
});
