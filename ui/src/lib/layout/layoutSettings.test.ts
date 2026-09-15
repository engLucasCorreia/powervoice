import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "../ipc/bindings";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import {
  applyLayoutPrefs,
  DEFAULT_LAYOUT_PREFS,
  layoutState,
  resetLayoutForTest,
  setDockHeightPx,
  setDockTab,
  setMarkersCollapsed,
  setMarkersWidthPx,
  setRackCollapsed,
  setRackWidthPx,
} from "./layoutSettings.svelte";
import { settingsFixture } from "../test/fixtures";

afterEach(() => {
  clearMocks();
  vi.useRealTimers();
  resetLayoutForTest();
  resetSettingsStateForTest();
});

describe("layoutState defaults and applyLayoutPrefs", () => {
  it("starts at the factory defaults", () => {
    const s = layoutState();
    expect(s.markersWidthPx).toBe(DEFAULT_LAYOUT_PREFS.markers_width_px);
    expect(s.rackWidthPx).toBe(DEFAULT_LAYOUT_PREFS.rack_width_px);
    expect(s.dockHeightPx).toBe(DEFAULT_LAYOUT_PREFS.dock_height_px);
    expect(s.markersCollapsed).toBe(false);
    expect(s.rackCollapsed).toBe(false);
    expect(s.dockTab).toBe("meters");
  });

  it("seeds every field from Settings.layout", () => {
    applyLayoutPrefs({
      markers_width_px: 300,
      rack_width_px: 320,
      dock_height_px: 280,
      markers_collapsed: true,
      rack_collapsed: true,
      dock_tab: "loudness",
    });
    const s = layoutState();
    expect(s.markersWidthPx).toBe(300);
    expect(s.rackWidthPx).toBe(320);
    expect(s.dockHeightPx).toBe(280);
    expect(s.markersCollapsed).toBe(true);
    expect(s.rackCollapsed).toBe(true);
    expect(s.dockTab).toBe("loudness");
  });
});

describe("H-24 item 3: debounced settings_set persistence", () => {
  it("a splitter drag debounces one settings_set write with the full layout snapshot", async () => {
    vi.useFakeTimers();
    const settingsSetCalls: Settings[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        const settings = (args as { settings: Settings }).settings;
        settingsSetCalls.push(settings);
        return settings;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    setMarkersWidthPx(260);
    setMarkersWidthPx(270); // a live drag fires many updates
    setRackWidthPx(300);
    expect(settingsSetCalls).toHaveLength(0); // debounced — nothing sent yet

    await vi.advanceTimersByTimeAsync(300);
    expect(settingsSetCalls).toHaveLength(1); // only the trailing call survives the debounce
    expect(settingsSetCalls[0]!.layout).toEqual({
      markers_width_px: 270,
      rack_width_px: 300,
      dock_height_px: DEFAULT_LAYOUT_PREFS.dock_height_px,
      markers_collapsed: false,
      rack_collapsed: false,
      dock_tab: "meters",
    });
  });

  it("dock height, collapse toggles and the dock tab all persist too", async () => {
    vi.useFakeTimers();
    const settingsSetCalls: Settings[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      if (cmd === "settings_set") {
        const settings = (args as { settings: Settings }).settings;
        settingsSetCalls.push(settings);
        return settings;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    setDockHeightPx(320);
    await vi.advanceTimersByTimeAsync(300);
    setMarkersCollapsed(true);
    await vi.advanceTimersByTimeAsync(300);
    setRackCollapsed(true);
    await vi.advanceTimersByTimeAsync(300);
    setDockTab("loudness");
    await vi.advanceTimersByTimeAsync(300);

    expect(settingsSetCalls).toHaveLength(4);
    const last = settingsSetCalls[3]!.layout;
    expect(last.dock_height_px).toBe(320);
    expect(last.markers_collapsed).toBe(true);
    expect(last.rack_collapsed).toBe(true);
    expect(last.dock_tab).toBe("loudness");
  });
});
