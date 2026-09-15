import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { pluginsState, resetPluginsForTest } from "../plugins/plugins.svelte";
import { settle } from "../plugins/testing";
import EffectsMenu from "./EffectsMenu.svelte";

afterEach(() => {
  clearMocks();
  resetMenuBarForTest();
  resetPluginsForTest();
});

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EffectsMenu, { target });
  flushSync();
  target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-effects"]')!.click();
  flushSync();
  return { target, app };
}

describe("Effects menu → plugins (T-809)", () => {
  it("Manage Plugins… opens the plugin manager", async () => {
    mockIPC(() => null);
    const { target, app } = mountMenu();
    const item = target.querySelector<HTMLButtonElement>('[data-testid="menu-manage-plugins"]')!;
    expect(item.textContent).toContain("Manage Plugins…");
    item.click();
    await settle();
    expect(pluginsState().open).toBe(true);
    unmount(app);
    target.remove();
  });

  it("Install Module… opens the native picker filtered to plugin files", async () => {
    const opened: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|open") {
        opened.push((args as { options: unknown }).options);
        return null;
      }
      return null;
    });
    const { target, app } = mountMenu();
    const item = target.querySelector<HTMLButtonElement>('[data-testid="menu-install-module"]')!;
    expect(item.textContent).toContain("Install Module…");
    item.click();
    await settle();
    expect(opened).toHaveLength(1);
    expect((opened[0] as { filters: { extensions: string[] }[] }).filters[0]!.extensions).toEqual(["clap", "vst3"]);
    expect(pluginsState().install.phase).toBe("idle");
    unmount(app);
    target.remove();
  });
});
