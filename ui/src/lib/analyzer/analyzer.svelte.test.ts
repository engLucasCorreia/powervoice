import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { Settings } from "../ipc/bindings";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import {
  analyzerState,
  applyAnalyzerPrefs,
  resetAnalyzerForTest,
  setAnalyzerPeakHold,
  setAnalyzerResponse,
  setAnalyzerVisible,
} from "./analyzer.svelte";

function settingsFixture(overrides: Partial<Settings> = {}): Settings {
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
    multichannel_policy: "ask",
    renderer_preference: "auto",
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
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  resetAnalyzerForTest();
  resetSettingsStateForTest();
});

describe("analyzer.svelte (H-16, SPEC-007 §2.9 persistence)", () => {
  it("applyAnalyzerPrefs seeds visibility/response/peak-hold from Settings", () => {
    applyAnalyzerPrefs({ visible: false, response: "slow", peakHold: false });
    const s = analyzerState();
    expect(s.visible).toBe(false);
    expect(s.response).toBe("slow");
    expect(s.peakHold).toBe(false);
  });

  it("setAnalyzerVisible updates state and persists via settings_set", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        calls.push((args as { settings: Settings }).settings);
        return (args as { settings: Settings }).settings;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    setAnalyzerVisible(false);
    expect(analyzerState().visible).toBe(false);
    await Promise.resolve();
    expect(calls).toHaveLength(1);
    expect((calls[0] as Settings).analyzer_visible).toBe(false);
  });

  it("setAnalyzerPeakHold updates state (readable via the get/set accessor) and persists", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        calls.push((args as { settings: Settings }).settings);
        return (args as { settings: Settings }).settings;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    const s = analyzerState();
    s.peakHold = false; // exercises the accessor's `set`, as `AnalyzerPanel`'s `bind:checked` does
    expect(analyzerState().peakHold).toBe(false);
    await Promise.resolve();
    expect(calls).toHaveLength(1);
    expect((calls[0] as Settings).analyzer_peak_hold).toBe(false);
  });

  it("setAnalyzerResponse updates state and persists (subscription update best-effort without one)", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        calls.push((args as { settings: Settings }).settings);
        return (args as { settings: Settings }).settings;
      }
      return null;
    });
    await loadSettings();

    await setAnalyzerResponse("fast");
    expect(analyzerState().response).toBe("fast");
    expect(calls).toHaveLength(1);
    expect((calls[0] as Settings).analyzer_response).toBe("fast");
  });
});
