import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { FLAGGED_ID, folderFixture, pluginEntry, pluginFixtures } from "../test/fixtures";
import { setPlatformForTest } from "../ui/platform";
import PluginManagerDialog from "./PluginManagerDialog.svelte";
import { initPlugins, openPluginManager, pluginsState, resetPluginsForTest } from "./plugins.svelte";
import { settle } from "./testing";

interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

function mock(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}): Call[] {
  const calls: Call[] = [];
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      if (cmd in handlers) return handlers[cmd]!(a);
      if (cmd === "plugins_list") return pluginFixtures();
      if (cmd === "plugins_folders") return folderFixture();
      return null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

let cleanup: (() => void) | null = null;

async function open(focus?: string): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(PluginManagerDialog, { target });
  cleanup = () => {
    unmount(app);
    target.remove();
  };
  openPluginManager({ focus });
  await settle();
  return target;
}

afterEach(() => {
  cleanup?.();
  cleanup = null;
  clearMocks();
  clearNotices();
  resetPluginsForTest();
  setPlatformForTest(null);
});

const q = <T extends Element = HTMLElement>(root: ParentNode, id: string) => root.querySelector<T>(`[data-testid="${id}"]`);
const all = (root: ParentNode, id: string) => [...root.querySelectorAll<HTMLElement>(`[data-testid="${id}"]`)];
const rows = (root: ParentNode) => all(root, "plugin-row");
const rowNamed = (root: ParentNode, name: string) =>
  rows(root).find((r) => q(r, "plugin-name")?.textContent?.trim() === name)!;
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

describe("Plugin manager (T-809)", () => {
  it("stays closed until opened", () => {
    mock();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(PluginManagerDialog, { target });
    expect(q(target, "plugin-manager")).toBeNull();
    unmount(app);
    target.remove();
  });

  it("lists every plugin with its format, channels, params and status", async () => {
    mock();
    const root = await open();
    expect(rows(root)).toHaveLength(8);
    expect(rows(root).map((r) => r.dataset.status)).toEqual([
      "flagged", // Breath Control
      "ok", // De-esser
      "blocklisted", // glitchy-comp
      "blocklisted", // Hum Remover
      "ok", // Loudness Rider
      "blocklisted", // slow-limiter
      "ok", // Small Room
      "disabled", // Tape Saturator
    ]);
    const deesser = rowNamed(root, "De-esser");
    expect(text(q(deesser, "plugin-path"))).toBe("/home/u/.clap/acme-deesser.clap");
    expect(text(q(deesser, "plugin-format"))).toBe("CLAP");
    expect(text(q(deesser, "plugin-io"))).toBe("Mono");
    expect(text(q(deesser, "plugin-params"))).toBe("12");
    expect(text(q(deesser, "plugin-status"))).toBe("OK");
    expect(text(q(rowNamed(root, "Tape Saturator"), "plugin-format"))).toBe("VST3");
    expect(text(q(rowNamed(root, "Tape Saturator"), "plugin-status"))).toBe("Disabled");
    expect(text(q(rowNamed(root, "Small Room"), "plugin-format"))).toBe("LV2");
    expect(text(q(rowNamed(root, "Small Room"), "plugin-io"))).toBe("1 in · 2 out");
    expect(text(q(rowNamed(root, "Loudness Rider"), "plugin-format"))).toBe("JSFX");
    expect(text(q(rowNamed(root, "Loudness Rider"), "plugin-io"))).toBe("—");
    const flagged = rowNamed(root, "Breath Control");
    expect(text(q(flagged, "plugin-status"))).toBe("Flagged");
    expect(text(q(flagged, "plugin-status-detail"))).toBe("Crashed 3 times");
    expect(text(q(rowNamed(root, "glitchy-comp"), "plugin-status-detail"))).toBe("Crashed during scan");
    expect(text(q(rowNamed(root, "slow-limiter"), "plugin-status-detail"))).toBe("Timed out during scan");
    expect(text(q(rowNamed(root, "Hum Remover"), "plugin-status-detail"))).toBe("Blocked by you");
    expect(text(q(root, "plugins-counts"))).toBe("8 plugins · 1 disabled · 3 blocklisted · 1 flagged");
  });

  it("filters by the search field and offers to clear it", async () => {
    mock();
    const root = await open();
    const search = q<HTMLInputElement>(root, "plugins-search")!;
    type(search, "acme");
    expect(rows(root).map((r) => text(q(r, "plugin-name")))).toEqual(["De-esser", "Hum Remover"]);
    type(search, "timed out");
    expect(rows(root).map((r) => text(q(r, "plugin-name")))).toEqual(["slow-limiter"]);
    type(search, "zzz");
    expect(rows(root)).toHaveLength(0);
    expect(q(root, "plugins-no-match")).not.toBeNull();
    q(root, "plugins-search-clear")!.click();
    flushSync();
    expect(rows(root)).toHaveLength(8);
  });

  it("sorts by a column header, toggling the direction", async () => {
    mock();
    const root = await open();
    const nameHeader = q(root, "plugins-sort-name")!.closest("th")!;
    expect(nameHeader.getAttribute("aria-sort")).toBe("ascending");
    q(root, "plugins-sort-status")!.click();
    flushSync();
    expect(rows(root).slice(0, 4).map((r) => r.dataset.status)).toEqual(["blocklisted", "blocklisted", "blocklisted", "flagged"]);
    expect(q(root, "plugins-sort-status")!.closest("th")!.getAttribute("aria-sort")).toBe("ascending");
    expect(nameHeader.getAttribute("aria-sort")).toBe("none");
    q(root, "plugins-sort-status")!.click();
    flushSync();
    expect(q(root, "plugins-sort-status")!.closest("th")!.getAttribute("aria-sort")).toBe("descending");
    expect(rows(root)[0]!.dataset.status).toBe("ok");
    q(root, "plugins-sort-vendor")!.click();
    flushSync();
    expect(text(q(rows(root)[2]!, "plugin-name"))).toBe("De-esser");
  });

  it("switches a plugin off for Add module", async () => {
    const calls = mock();
    const root = await open();
    const toggle = q<HTMLButtonElement>(rowNamed(root, "De-esser"), "plugin-enabled")!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.getAttribute("aria-labelledby")).toBeTruthy();
    expect(q<HTMLButtonElement>(rowNamed(root, "Tape Saturator"), "plugin-enabled")!.getAttribute("aria-checked")).toBe(
      "false",
    );
    toggle.click();
    await settle();
    expect(calls.find((c) => c.cmd === "plugins_set_enabled")?.args).toEqual({
      moduleId: "clap:com.acme.deesser",
      enabled: false,
    });
    // Blocklisted rows have no switch, but an Unblock button.
    expect(q(rowNamed(root, "glitchy-comp"), "plugin-enabled")).toBeNull();
    q(rowNamed(root, "glitchy-comp"), "plugin-unblock")!.click();
    await settle();
    expect(calls.find((c) => c.cmd === "plugins_unblock")?.args).toEqual({ path: "/home/u/Downloads/glitchy-comp.clap" });
    expect(calls.find((c) => c.cmd === "plugins_rescan")?.args).toEqual({ full: false });
  });

  it("runs the row menu's actions: clear a crash warning, block, reveal, unblock", async () => {
    const calls = mock();
    const root = await open();
    const act = async (name: string, action: string) => {
      q(rowNamed(root, name), "plugin-actions")!.click();
      flushSync();
      const item = q<HTMLElement>(document, action);
      expect(item, `${name}: ${action}`).not.toBeNull();
      item!.click();
      await settle();
    };
    await act("Breath Control", "plugin-action-clear-flag");
    // "Breath Control" lives in the install folder (H-29), so its row offers "Uninstall…", not
    // "Block" — use a plugin found elsewhere for that.
    await act("Tape Saturator", "plugin-action-block");
    await act("Small Room", "plugin-action-reveal");
    await act("Hum Remover", "plugin-action-unblock");
    const actions = calls.filter((c) =>
      ["plugins_clear_flag", "plugins_block", "plugins_reveal", "plugins_unblock"].includes(c.cmd),
    );
    expect(actions).toEqual([
      { cmd: "plugins_clear_flag", args: { moduleId: FLAGGED_ID } },
      { cmd: "plugins_block", args: { path: "/usr/lib/vst3/Tape Saturator.vst3" } },
      { cmd: "plugins_reveal", args: { path: "/usr/lib/lv2/small-room.lv2" } },
      { cmd: "plugins_unblock", args: { path: "/media/plugins/hum-remover.clap" } },
    ]);
  });

  it("offers Uninstall… (not Block) for a plugin in the install folder, and opens the confirm prompt (H-29)", async () => {
    mock();
    const root = await open();
    // "De-esser" lives at /home/u/.clap/acme-deesser.clap — inside folderFixture().install.
    q(rowNamed(root, "De-esser"), "plugin-actions")!.click();
    flushSync();
    expect(q(document, "plugin-action-block")).toBeNull();
    const uninstall = q<HTMLElement>(document, "plugin-action-uninstall");
    expect(uninstall).not.toBeNull();
    uninstall!.click();
    // The confirm dialog itself (`UninstallPluginDialog`, mounted alongside this one in
    // App.svelte) is covered by its own test file — here we only check the row menu wires into
    // the shared store correctly.
    expect(pluginsState().uninstallPrompt?.entry.name).toBe("De-esser");
  });

  it("shows a duplicate-id loser as Shadowed by … (H-29)", async () => {
    const shadow = pluginEntry({
      id: "",
      name: "De-esser (old)",
      vendor: "Acme Audio",
      version: "2.0.0",
      path: "/usr/lib/clap/acme-deesser.clap",
      status: { kind: "shadowed", by: "/home/u/.clap/acme-deesser.clap" },
      ports: null,
      param_count: 0,
    });
    mock({ plugins_list: () => [...pluginFixtures(), shadow] });
    const root = await open();
    const row = rowNamed(root, "De-esser (old)");
    expect(text(q(row, "plugin-status"))).toBe("Shadowed");
    expect(text(q(row, "plugin-status-detail"))).toBe("Shadowed by acme-deesser.clap");
    // Never registered, so it can't be toggled or uninstalled — only Block, since it isn't
    // itself in the install folder here.
    q(row, "plugin-actions")!.click();
    flushSync();
    expect(q(document, "plugin-action-uninstall")).toBeNull();
    expect(q(document, "plugin-action-block")).not.toBeNull();
  });

  it("rescans quickly or fully from the Rescan menu", async () => {
    const calls = mock();
    const root = await open();
    for (const [item, full] of [
      ["plugins-rescan-quick", false],
      ["plugins-rescan-full", true],
    ] as const) {
      q(root, "plugins-rescan")!.click();
      flushSync();
      q(document, item)!.click();
      await settle();
      expect(calls.filter((c) => c.cmd === "plugins_rescan").at(-1)?.args).toEqual({ full });
    }
  });

  it("shows live scan progress from plugin_scan_progress", async () => {
    mock();
    const stop = await initPlugins();
    const root = await open();
    expect(q(root, "plugins-scan-progress")).toBeNull();
    await emit("plugin_scan_progress", { done: 7, total: 19, current_path: "/usr/lib/clap/studio.clap", summary: null });
    flushSync();
    const bar = q(root, "plugins-scan-progress")!;
    expect(bar.querySelector("progress")?.getAttribute("value")).toBe("7");
    expect(bar.querySelector("progress")?.getAttribute("max")).toBe("19");
    expect(text(q(root, "plugins-scan-count"))).toBe("Scanning 7 of 19");
    expect(text(bar)).toContain("studio.clap");
    await emit("plugin_scan_progress", {
      done: 19,
      total: 19,
      current_path: null,
      summary: { scanned: 3, cached: 16, failed: 0, blocklisted_now: 1, blocklisted: 3, effects: 6, newly_registered: ["a", "b"] },
    });
    await settle();
    expect(q(root, "plugins-scan-progress")).toBeNull();
    expect(text(q(root, "plugins-last-scan"))).toBe("Last scan · Effects: 6 · New: 2 · Newly blocklisted: 1");
    stop();
  });

  it("has loading, empty and error states", async () => {
    let answer: () => unknown = () => new Promise(() => {});
    mock({ plugins_list: () => answer() });
    const root = await open();
    expect(q(root, "plugins-loading")).not.toBeNull();
    cleanup?.();
    resetPluginsForTest();

    answer = () => [];
    const empty = await open();
    expect(q(empty, "plugins-empty")).not.toBeNull();
    cleanup?.();
    resetPluginsForTest();

    answer = () => {
      throw { code: "internal", key: "error.internal", params: {} };
    };
    const failed = await open();
    expect(q(failed, "plugins-error")).not.toBeNull();
    answer = () => pluginFixtures();
    q(failed, "plugins-retry")!.click();
    await settle();
    expect(rows(failed)).toHaveLength(8);
  });

  it("highlights the plugin it was opened on", async () => {
    mock();
    const root = await open(FLAGGED_ID);
    const focused = rows(root).filter((r) => r.classList.contains("focused"));
    expect(focused.map((r) => text(q(r, "plugin-name")))).toEqual(["Breath Control"]);
  });

  it("lists the scanned folders and edits the user's own on the Folders tab", async () => {
    const calls = mock({ "plugin:dialog|open": () => "/opt/more-plugins" });
    const root = await open();
    const foldersTab = root.querySelector<HTMLButtonElement>("#plugin-manager-tab-folders")!;
    foldersTab.click();
    await settle();
    expect(q(root, "plugin-manager-panel-folders")).not.toBeNull();
    expect(all(root, "plugin-folder-standard").map(text)).toEqual([
      "/home/u/.clap Installs here",
      "/usr/lib/clap Standard",
      "/home/u/.vst3 Installs here",
      "/usr/lib/vst3 Standard",
    ]);
    expect(all(root, "plugin-folder-custom").map((li) => text(li.querySelector(".path")))).toEqual(["/media/plugins"]);
    q(root, "plugin-folder-remove")!.click();
    await settle();
    expect(calls.find((c) => c.cmd === "plugins_remove_folder")?.args).toEqual({ path: "/media/plugins" });
    q(root, "plugin-folder-add")!.click();
    await settle();
    expect(calls.find((c) => c.cmd === "plugins_add_folder")?.args).toEqual({ path: "/opt/more-plugins" });
  });

  it("starts an install from its toolbar and closes on Escape", async () => {
    const calls = mock({ "plugin:dialog|open": () => null });
    const root = await open();
    q(root, "plugins-install")!.click();
    await settle();
    expect(calls.some((c) => c.cmd === "plugin:dialog|open")).toBe(true);
    q(root, "plugin-manager")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(pluginsState().open).toBe(false);
    expect(q(root, "plugin-manager")).toBeNull();
  });

  it("explains the Linux VST3-bundle picker quirk under the Install button, on Linux (H-34)", async () => {
    mock();
    setPlatformForTest("linux");
    const root = await open();
    expect(text(q(root, "plugins-install-linux-hint"))).toContain(".vst3");
  });

  it.each(["mac", "windows"] as const)("shows no Linux picker hint on %s (H-34)", async (platform) => {
    mock();
    setPlatformForTest(platform);
    const root = await open();
    expect(q(root, "plugins-install-linux-hint")).toBeNull();
  });
});
