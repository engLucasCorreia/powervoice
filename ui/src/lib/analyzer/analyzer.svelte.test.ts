import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { Settings } from "../ipc/bindings";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { settingsFixture } from "../test/fixtures";
import {
  analyzerState,
  applyAnalyzerPrefs,
  resetAnalyzerForTest,
  setAnalyzerPeakHold,
  setAnalyzerResponse,
  setAnalyzerVisible,
} from "./analyzer.svelte";

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
