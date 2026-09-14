import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "../ipc/bindings";
import { clearActionHandlers, dispatchAction } from "../keymap";
import {
  applyRestoredSpectralView,
  applySpectralDefaults,
  initSpectral,
  resetSpectralForTest,
  spectralState,
} from "./spectral.svelte";
import { loadSettings, resetSettingsStateForTest } from "./settings.svelte";

function settingsFixture(): Settings {
  return {
    version: 1,
    device: {
      host: "pipewire",
      input_device: null,
      input_channel: 1,
      output_device: null,
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    default_format: { sample_rate_hz: 48_000, bit_depth: "24" },
    monitor_mode: "off",
    monitor_hint_shown: false,
    telemetry_rate_hz: 60,
    memory_budget_mib: 2048,
    normalize_dialog: { value: -1, unit: "db" },
    recent_files: [],
    spectral_defaults: {
      freq_scale: "log",
      colormap: "inferno",
      display_floor_db: -120,
      display_ceil_db: 0,
      fft_size: null,
    },
    analyzer_visible: true,
    analyzer_response: "medium",
    analyzer_peak_hold: true,
    multichannel_policy: "ask",
    renderer_preference: "auto",
  };
}

afterEach(() => {
  clearActionHandlers();
  resetSpectralForTest();
  resetSettingsStateForTest();
  clearMocks();
  vi.useRealTimers();
});

describe("spectralState (T-207, SPEC-007 §2.1)", () => {
  it("defaults to hidden, 50% split, log scale, inferno, -120/0 dB, Auto FFT", () => {
    const s = spectralState();
    expect(s.visible).toBe(false);
    expect(s.splitRatio).toBe(50);
    expect(s.freqScale).toBe("log");
    expect(s.colormap).toBe("inferno");
    expect(s.floorDb).toBe(-120);
    expect(s.ceilDb).toBe(0);
    expect(s.fftSize).toBeNull();
  });

  it("toggle() flips visibility", () => {
    const s = spectralState();
    s.toggle();
    expect(spectralState().visible).toBe(true);
    s.toggle();
    expect(spectralState().visible).toBe(false);
  });

  it("Shift+D (spectral.toggle action) toggles visibility once wired by initSpectral", () => {
    const teardown = initSpectral();
    expect(spectralState().visible).toBe(false);
    dispatchAction("spectral.toggle");
    expect(spectralState().visible).toBe(true);
    dispatchAction("spectral.toggle");
    expect(spectralState().visible).toBe(false);
    teardown();
  });

  it("setSplitRatio clamps to [0, 100]; a pane dragged to either edge stays reachable", () => {
    const s = spectralState();
    s.setSplitRatio(-10);
    expect(spectralState().splitRatio).toBe(0);
    s.setSplitRatio(150);
    expect(spectralState().splitRatio).toBe(100);
    s.setSplitRatio(37.5);
    expect(spectralState().splitRatio).toBe(37.5);
  });

  it("setFreqScale / setColormap / setFftSize set the value directly", () => {
    const s = spectralState();
    s.setFreqScale("linear");
    expect(spectralState().freqScale).toBe("linear");
    s.setColormap("viridis");
    expect(spectralState().colormap).toBe("viridis");
    s.setFftSize(4096);
    expect(spectralState().fftSize).toBe(4096);
    s.setFftSize(null);
    expect(spectralState().fftSize).toBeNull();
  });

  it("setFloorDb/setCeilDb clamp to their ranges and keep at least a 20 dB span", () => {
    const s = spectralState();
    s.setFloorDb(-1000);
    expect(spectralState().floorDb).toBe(-150); // clamped to the range floor
    s.setFloorDb(-40);
    s.setCeilDb(-45); // would leave a 5 dB span against -40 -> pushed apart to 20
    const after = spectralState();
    expect(after.floorDb).toBe(-40);
    expect(after.ceilDb).toBe(-20);
    expect(after.ceilDb - after.floorDb).toBe(20);
    s.setCeilDb(1000);
    expect(spectralState().ceilDb).toBe(6); // clamped to the range ceiling
  });
});

describe("T-306: sidecar persistence (SPEC-018 §2.6.5)", () => {
  it("a setter debounces a sidecar_view_set_spectral call with the full snapshot", () => {
    vi.useFakeTimers();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "sidecar_view_set_spectral") {
        calls.push(args);
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const s = spectralState();
    s.setVisible(true);
    s.setSplitRatio(62.5);
    expect(calls).toHaveLength(0); // debounced — nothing sent yet

    vi.advanceTimersByTime(300);
    expect(calls).toHaveLength(1); // only the trailing call survives the debounce
    expect(calls[0]).toEqual({
      spectral: {
        visible: true,
        split_ratio: 62.5,
        fft_size: null,
        freq_scale: "log",
        display_floor_db: -120,
        display_ceil_db: 0,
        colormap: "inferno",
      },
    });
  });

  it("applyRestoredSpectralView sets the state without scheduling a persist call", () => {
    vi.useFakeTimers();
    let called = false;
    mockIPC((cmd) => {
      if (cmd === "sidecar_view_set_spectral") {
        called = true;
      }
      return null;
    });

    applyRestoredSpectralView({
      visible: true,
      split_ratio: 62.5,
      fft_size: 4096,
      freq_scale: "linear",
      display_floor_db: -100,
      display_ceil_db: -10,
      colormap: "viridis",
    });

    const s = spectralState();
    expect(s.visible).toBe(true);
    expect(s.splitRatio).toBe(62.5);
    expect(s.fftSize).toBe(4096);
    expect(s.freqScale).toBe("linear");
    expect(s.colormap).toBe("viridis");
    expect(s.floorDb).toBe(-100);
    expect(s.ceilDb).toBe(-10);

    vi.advanceTimersByTime(1000);
    expect(called).toBe(false);
  });

  it("applyRestoredSpectralView ignores an unrecognized enum value, keeping the current one", () => {
    applyRestoredSpectralView({
      visible: true,
      split_ratio: 30,
      fft_size: null,
      freq_scale: "log-ish", // unrecognized
      display_floor_db: -120,
      display_ceil_db: 0,
      colormap: "sunset", // unrecognized
    });
    const s = spectralState();
    expect(s.freqScale).toBe("log");
    expect(s.colormap).toBe("inferno");
  });
});

describe("H-12 (A-014): app-wide spectral display defaults", () => {
  it("a display setter also debounces a settings_set write to spectral_defaults", async () => {
    vi.useFakeTimers();
    const settingsSetCalls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        settingsSetCalls.push((args as { settings: Settings }).settings);
        return (args as { settings: Settings }).settings;
      }
      if (cmd === "sidecar_view_set_spectral") {
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    spectralState().setColormap("viridis");
    spectralState().setFloorDb(-100);
    expect(settingsSetCalls).toHaveLength(0); // debounced

    await vi.advanceTimersByTimeAsync(300);
    expect(settingsSetCalls).toHaveLength(1); // only the trailing call survives the debounce
    expect((settingsSetCalls[0] as Settings).spectral_defaults).toEqual({
      freq_scale: "log",
      colormap: "viridis",
      display_floor_db: -100,
      display_ceil_db: 0, // unchanged: -100..0 is already a 100 dB span
      fft_size: null,
    });
  });

  it("visibility/split-ratio changes never write to settings_set (per document only, SPEC-018 §2.6.5)", async () => {
    vi.useFakeTimers();
    let settingsSetCalled = false;
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        settingsSetCalled = true;
        return settingsFixture();
      }
      return null;
    });
    await loadSettings();

    spectralState().toggle();
    spectralState().setSplitRatio(75);
    await vi.advanceTimersByTimeAsync(1000);

    expect(settingsSetCalled).toBe(false);
  });

  it("applySpectralDefaults sets only the display settings, leaving visibility/split ratio alone", () => {
    spectralState().setVisible(true);
    spectralState().setSplitRatio(75);

    applySpectralDefaults({
      freq_scale: "linear",
      colormap: "viridis",
      display_floor_db: -100,
      display_ceil_db: -10,
      fft_size: 4096,
    });

    const s = spectralState();
    expect(s.visible).toBe(true); // untouched
    expect(s.splitRatio).toBe(75); // untouched
    expect(s.freqScale).toBe("linear");
    expect(s.colormap).toBe("viridis");
    expect(s.floorDb).toBe(-100);
    expect(s.ceilDb).toBe(-10);
    expect(s.fftSize).toBe(4096);
  });

  it("applySpectralDefaults ignores an unrecognized enum value, keeping the current one", () => {
    applySpectralDefaults({
      freq_scale: "log-ish", // unrecognized
      colormap: "sunset", // unrecognized
      display_floor_db: -120,
      display_ceil_db: 0,
      fft_size: null,
    });
    const s = spectralState();
    expect(s.freqScale).toBe("log");
    expect(s.colormap).toBe("inferno");
  });
});
