import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { Settings } from "../ipc/bindings";
import { clearNotices } from "./notices.svelte";
import { loadSettings, resetSettingsStateForTest, saveSettings, settingsState } from "./settings.svelte";
import { settingsFixture as makeSettings } from "../test/fixtures";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetSettingsStateForTest();
});

describe("settings store", () => {
  it("loads settings via settings_get", async () => {
    const fixture = makeSettings();
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await loadSettings();

    expect(settingsState().current).toEqual(fixture);
    expect(settingsState().loading).toBe(false);
    expect(settingsState().error).toBeNull();
  });

  it("saves a patch merged onto the current settings via settings_set", async () => {
    const fixture = makeSettings();
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return fixture;
      }
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await loadSettings();
    await saveSettings({ monitor_mode: "dry" });

    expect(lastSaved?.monitor_mode).toBe("dry");
    expect(lastSaved?.default_format).toEqual(fixture.default_format);
    expect(settingsState().current?.monitor_mode).toBe("dry");
  });

  it("does nothing when saving before settings are loaded", async () => {
    let called = false;
    mockIPC((cmd) => {
      called = true;
      throw new Error(`unexpected command: ${cmd}`);
    });

    await saveSettings({ monitor_mode: "dry" });

    expect(called).toBe(false);
    expect(settingsState().current).toBeNull();
  });

  it("surfaces a settings_get failure as an error and a notice, without throwing", async () => {
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        throw { code: "io", key: "error.io", params: { message: "disk full" } };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await expect(loadSettings()).resolves.toBeUndefined();
    expect(settingsState().error).toBe("error.io");
    expect(settingsState().current).toBeNull();
  });
});
