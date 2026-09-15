import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { pluginsState, resetPluginsForTest } from "../plugins/plugins.svelte";
import { folderFixture, pluginFixtures, settle } from "../plugins/testing";
import { clearNotices } from "../state/notices.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import PreferencesDialog from "./PreferencesDialog.svelte";
import { openPreferences, preferencesState, resetPreferencesForTest } from "./preferences.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetSettingsStateForTest();
  resetPreferencesForTest();
  resetPluginsForTest();
});

const q = (root: ParentNode, id: string) => root.querySelector<HTMLElement>(`[data-testid="${id}"]`);

describe("Preferences → Plugins (T-809)", () => {
  it("summarises the plugins, edits the user's folders and opens the manager", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "plugins_list") return pluginFixtures();
      if (cmd === "plugins_folders") return folderFixture();
      return null;
    });
    openPreferences();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(PreferencesDialog, { target });
    await settle();

    const section = q(target, "preferences-plugins")!;
    // 3 blocklisted + 1 flagged need attention.
    expect(q(section, "preferences-plugins-summary")?.textContent?.trim()).toBe("8 plugins · 4 need attention");
    // Only the user's own folders here (the manager lists the standard ones too).
    expect(section.querySelectorAll('[data-testid="plugin-folder-standard"]')).toHaveLength(0);
    expect([...section.querySelectorAll('[data-testid="plugin-folder-custom"] .path')].map((e) => e.textContent?.trim())).toEqual([
      "/media/plugins",
    ]);
    expect(q(section, "plugin-folder-add")).not.toBeNull();

    q(section, "preferences-manage-plugins")!.click();
    flushSync();
    expect(preferencesState().open).toBe(false);
    expect(pluginsState().open).toBe(true);
    unmount(app);
    target.remove();
  });
});
