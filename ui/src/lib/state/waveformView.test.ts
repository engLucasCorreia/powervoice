import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  audioKeyFor,
  clearPendingRestore,
  consumePendingRestore,
  resetWaveformViewForTest,
  schedulePersistWaveformView,
  setPendingRestore,
  waveformViewApi,
} from "./waveformView.svelte";

afterEach(() => {
  clearMocks();
  resetWaveformViewForTest();
  vi.useRealTimers();
});

describe("waveformViewApi (H-12, SPEC-018 §2.6.5's view.waveform)", () => {
  it("defaults to startSample 0 / samplesPerPixel 1", () => {
    const wv = waveformViewApi();
    expect(wv.startSample).toBe(0);
    expect(wv.samplesPerPixel).toBe(1);
  });

  it("is a get/set accessor pair over shared module state (bindable from a template)", () => {
    const wv = waveformViewApi();
    wv.startSample = 1_234;
    wv.samplesPerPixel = 37.25;

    // A second call returns an accessor over the SAME underlying state.
    const wv2 = waveformViewApi();
    expect(wv2.startSample).toBe(1_234);
    expect(wv2.samplesPerPixel).toBe(37.25);
  });
});

describe("H-12: sidecar_view_set_waveform persistence (debounced, SPEC-018 §2.6.5)", () => {
  it("debounces a sidecar_view_set_waveform call with the full snapshot", () => {
    vi.useFakeTimers();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "sidecar_view_set_waveform") {
        calls.push(args);
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    schedulePersistWaveformView(0, 1, null, 0);
    schedulePersistWaveformView(1_000, 2.5, { startSample: 100, endSample: 200 }, 150);
    expect(calls).toHaveLength(0); // debounced — nothing sent yet

    vi.advanceTimersByTime(300);
    expect(calls).toHaveLength(1); // only the trailing call survives the debounce
    expect(calls[0]).toEqual({
      waveform: {
        start_sample: 1_000,
        samples_per_pixel: 2.5,
        selection: { start_sample: 100, end_sample: 200 },
        cursor_samples: 150,
      },
    });
  });

  it("a null selection persists as null (no selection)", () => {
    vi.useFakeTimers();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return null;
    });

    schedulePersistWaveformView(0, 1, null, 42);
    vi.advanceTimersByTime(300);

    expect(calls).toHaveLength(1);
    expect((calls[0] as { waveform: { selection: unknown } }).waveform.selection).toBeNull();
  });
});

describe("H-12: pending restore (WaveformView's zoom-to-fit-vs-restore decision)", () => {
  it("consumePendingRestore returns the pending viewport only for the matching audio key, once", () => {
    setPendingRestore(audioKeyFor(48_000, 480_000), 1_234, 37.25);

    expect(consumePendingRestore(audioKeyFor(44_100, 480_000))).toBeNull(); // wrong key
    const restore = consumePendingRestore(audioKeyFor(48_000, 480_000));
    expect(restore).toEqual({
      audioKey: audioKeyFor(48_000, 480_000),
      startSample: 1_234,
      samplesPerPixel: 37.25,
    });

    // One-shot: consuming again (e.g. a later resize of the same document) returns nothing, so a
    // restore doesn't keep fighting the user's own zoom/scroll.
    expect(consumePendingRestore(audioKeyFor(48_000, 480_000))).toBeNull();
  });

  it("clearPendingRestore drops a pending restore without applying it", () => {
    setPendingRestore(audioKeyFor(48_000, 480_000), 1_234, 37.25);
    clearPendingRestore();
    expect(consumePendingRestore(audioKeyFor(48_000, 480_000))).toBeNull();
  });

  it("audioKeyFor combines rate and length (a Save As to a different rate/length is a new key)", () => {
    expect(audioKeyFor(48_000, 480_000)).toBe("48000:480000");
    expect(audioKeyFor(48_000, 480_000)).not.toBe(audioKeyFor(44_100, 480_000));
  });
});
