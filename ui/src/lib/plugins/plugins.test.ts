import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { PluginInstallResultDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { setPlatformForTest } from "../ui/platform";
import {
  addPluginFolder,
  blockPlugin,
  cancelUninstall,
  clearPluginFlag,
  closeInstall,
  confirmReplace,
  confirmUninstall,
  initPlugins,
  openPluginManager,
  pluginCrashCount,
  pluginsState,
  refreshPlugins,
  removePluginFolder,
  requestUninstall,
  rescanPlugins,
  resetPluginsForTest,
  revealPlugin,
  setPluginEnabled,
  showInstallInManager,
  startInstall,
  unblockPlugin,
} from "./plugins.svelte";
import { settle } from "./testing";
import { FLAGGED_ID, folderFixture, pluginFixtures } from "../test/fixtures";

interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

/** Mocks IPC (with events) and records every command. `handlers` answer by command name. */
function mock(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}): Call[] {
  const calls: Call[] = [];
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      if (cmd in handlers) {
        return handlers[cmd]!(a);
      }
      if (cmd === "plugins_list") return pluginFixtures();
      if (cmd === "plugins_folders") return folderFixture();
      return null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

const pluginCalls = (calls: Call[]) => calls.filter((c) => c.cmd.startsWith("plugins_") || c.cmd.startsWith("plugin:dialog"));
const entry = (name: string) => pluginFixtures().find((e) => e.name === name)!;

afterEach(() => {
  clearMocks();
  clearNotices();
  resetPluginsForTest();
  setPlatformForTest(null);
});

describe("plugins store (T-809)", () => {
  it("loads the list and the crash counts the rack reads", async () => {
    mock();
    await refreshPlugins();
    expect(pluginsState().list).toHaveLength(8);
    expect(pluginCrashCount(FLAGGED_ID)).toBe(3);
    expect(pluginCrashCount("clap:com.acme.deesser")).toBe(0);
    expect(pluginCrashCount(null)).toBe(0);
  });

  it("opens the manager on a plugin and loads the list and folders", async () => {
    const calls = mock();
    openPluginManager({ focus: FLAGGED_ID });
    await settle();
    expect(pluginsState().open).toBe(true);
    expect(pluginsState().focusKey).toBe(FLAGGED_ID);
    expect(pluginsState().folders?.custom).toEqual(["/media/plugins"]);
    expect(calls.map((c) => c.cmd)).toEqual(expect.arrayContaining(["plugins_list", "plugins_folders"]));
  });

  it("calls the right command for each action", async () => {
    const calls = mock();
    await setPluginEnabled(entry("De-esser"), false);
    await blockPlugin(entry("Breath Control"));
    await unblockPlugin(entry("glitchy-comp"));
    await clearPluginFlag(entry("Breath Control"));
    await revealPlugin(entry("Small Room"));
    await rescanPlugins(true);
    const actions = pluginCalls(calls).filter((c) => c.cmd !== "plugins_list");
    expect(actions).toEqual([
      { cmd: "plugins_set_enabled", args: { moduleId: "clap:com.acme.deesser", enabled: false } },
      { cmd: "plugins_block", args: { path: "/home/u/.clap/breath-control.clap" } },
      { cmd: "plugins_unblock", args: { path: "/home/u/Downloads/glitchy-comp.clap" } },
      // Unblocking rescans so the file is picked up again.
      { cmd: "plugins_rescan", args: { full: false } },
      { cmd: "plugins_clear_flag", args: { moduleId: FLAGGED_ID } },
      { cmd: "plugins_reveal", args: { path: "/usr/lib/lv2/small-room.lv2" } },
      { cmd: "plugins_rescan", args: { full: true } },
    ]);
    expect(pluginsState().focusKey).toBe("path:/home/u/Downloads/glitchy-comp.clap");
  });

  it("uninstalls after confirmation, then refreshes the list (H-29)", async () => {
    const calls = mock();
    const target = entry("De-esser");
    expect(pluginsState().uninstallPrompt).toBeNull();

    requestUninstall(target);
    expect(pluginsState().uninstallPrompt).toEqual({ entry: target, busy: false });

    const done = confirmUninstall();
    expect(pluginsState().uninstallPrompt?.busy).toBe(true);
    await done;

    expect(pluginsState().uninstallPrompt).toBeNull();
    const actions = pluginCalls(calls).filter((c) => c.cmd !== "plugins_list");
    expect(actions).toEqual([
      { cmd: "plugins_uninstall", args: { path: target.path } },
    ]);
  });

  it("cancelling the uninstall prompt calls nothing", async () => {
    const calls = mock();
    requestUninstall(entry("De-esser"));
    cancelUninstall();
    expect(pluginsState().uninstallPrompt).toBeNull();
    expect(pluginCalls(calls).some((c) => c.cmd === "plugins_uninstall")).toBe(false);
  });

  it("a failed uninstall closes the prompt and reports the error", async () => {
    mock({
      plugins_uninstall: () => {
        throw { code: "invalid_argument", key: "error.plugins.uninstall.outside_install_folder", params: {} };
      },
    });
    requestUninstall(entry("De-esser"));
    await confirmUninstall();
    expect(pluginsState().uninstallPrompt).toBeNull();
  });

  it("adds a folder from the native folder picker and removes one", async () => {
    let picked: string | null = "/opt/plugins";
    const calls = mock({ "plugin:dialog|open": () => picked });
    await addPluginFolder();
    const open = calls.find((c) => c.cmd === "plugin:dialog|open")!;
    expect((open.args.options as { directory: boolean }).directory).toBe(true);
    expect(calls.find((c) => c.cmd === "plugins_add_folder")?.args).toEqual({ path: "/opt/plugins" });
    // A rescan follows the change: progress shows until its events arrive.
    expect(pluginsState().scan).not.toBeNull();

    picked = null;
    const before = calls.length;
    await addPluginFolder();
    expect(calls.slice(before).some((c) => c.cmd === "plugins_add_folder")).toBe(false);

    await removePluginFolder("/media/plugins");
    expect(calls.find((c) => c.cmd === "plugins_remove_folder")?.args).toEqual({ path: "/media/plugins" });
  });

  it("follows plugin_scan_progress, then refreshes on the summary", async () => {
    const calls = mock();
    const stop = await initPlugins();
    await settle();
    const listsBefore = calls.filter((c) => c.cmd === "plugins_list").length;

    await emit("plugin_scan_progress", { done: 2, total: 5, current_path: "/usr/lib/clap/a.clap", summary: null });
    expect(pluginsState().scan).toEqual({ done: 2, total: 5, currentPath: "/usr/lib/clap/a.clap" });
    await emit("plugin_scan_progress", { done: 3, total: 5, current_path: "/usr/lib/clap/b.clap", summary: null });
    expect(pluginsState().scan?.done).toBe(3);

    const summary = { scanned: 2, cached: 3, failed: 0, blocklisted_now: 1, blocklisted: 1, effects: 4, newly_registered: ["clap:x"] };
    await emit("plugin_scan_progress", { done: 5, total: 5, current_path: null, summary });
    await settle();
    expect(pluginsState().scan).toBeNull();
    expect(pluginsState().lastScan).toEqual(summary);
    expect(calls.filter((c) => c.cmd === "plugins_list").length).toBe(listsBefore + 1);
    stop();
  });

  describe("Install module…", () => {
    const installed: PluginInstallResultDto = {
      kind: "installed",
      path: "/home/u/.clap/acme.clap",
      replaced: false,
      effects: [{ id: "clap:com.acme.deesser", name: "De-esser" }],
    };

    it("picks a .clap, installs it and reports the effects it added", async () => {
      const calls = mock({ "plugin:dialog|open": () => "/home/u/Downloads/acme.clap", plugins_install: () => installed });
      await startInstall();
      const open = calls.find((c) => c.cmd === "plugin:dialog|open")!;
      const options = open.args.options as { directory: boolean; filters: { extensions: string[] }[] };
      expect(options.directory).toBe(false);
      expect(options.filters[0]!.extensions).toEqual(["clap", "vst3"]);
      expect(calls.find((c) => c.cmd === "plugins_install")?.args).toEqual({
        path: "/home/u/Downloads/acme.clap",
        replace: false,
      });
      expect(pluginsState().install).toMatchObject({ phase: "installed", target: "/home/u/.clap/acme.clap" });
      // The list refreshes so the manager (and Add module) show the new effect.
      expect(calls.at(-1)?.cmd).toBe("plugins_list");

      showInstallInManager();
      expect(pluginsState().install.phase).toBe("idle");
      expect(pluginsState().open).toBe(true);
      expect(pluginsState().focusKey).toBe("clap:com.acme.deesser");
    });

    it("asks before replacing a plugin with the same name", async () => {
      const calls = mock({
        "plugin:dialog|open": () => "/home/u/Downloads/acme.clap",
        plugins_install: (a) =>
          a.replace ? { ...installed, replaced: true } : { kind: "collision", path: "/home/u/.clap/acme.clap" },
      });
      await startInstall();
      expect(pluginsState().install).toEqual({
        phase: "collision",
        source: "/home/u/Downloads/acme.clap",
        target: "/home/u/.clap/acme.clap",
      });
      await confirmReplace();
      expect(calls.filter((c) => c.cmd === "plugins_install").map((c) => c.args.replace)).toEqual([false, true]);
      expect(pluginsState().install).toMatchObject({ phase: "installed", replaced: true });
    });

    it("cancelling the collision prompt installs nothing", async () => {
      const calls = mock({
        "plugin:dialog|open": () => "/home/u/Downloads/acme.clap",
        plugins_install: () => ({ kind: "collision", path: "/home/u/.clap/acme.clap" }),
      });
      await startInstall();
      closeInstall();
      expect(pluginsState().install.phase).toBe("idle");
      expect(calls.filter((c) => c.cmd === "plugins_install")).toHaveLength(1);
    });

    it("reports a failure and whether the file was blocklisted", async () => {
      mock({
        "plugin:dialog|open": () => "/home/u/Downloads/bad.clap",
        plugins_install: () => ({
          kind: "failed",
          code: "scan_crashed",
          detail: "crashed while being scanned (signal 11)",
          blocklisted: true,
          cause: "crashed",
        }),
      });
      await startInstall();
      expect(pluginsState().install).toEqual({
        phase: "failed",
        source: "/home/u/Downloads/bad.clap",
        code: "scan_crashed",
        detail: "crashed while being scanned (signal 11)",
        blocklisted: true,
        cause: "crashed",
      });
      showInstallInManager();
      expect(pluginsState().focusKey).toBe("path:/home/u/Downloads/bad.clap");
    });

    it("does nothing when the picker is cancelled", async () => {
      const calls = mock({ "plugin:dialog|open": () => null });
      await startInstall();
      expect(calls.some((c) => c.cmd === "plugins_install")).toBe(false);
      expect(pluginsState().install.phase).toBe("idle");
    });

    it("H-34: the picker's title and filter name explain the .vst3-bundle quirk on Linux only", async () => {
      setPlatformForTest("linux");
      let calls = mock({ "plugin:dialog|open": () => null });
      await startInstall();
      let open = calls.find((c) => c.cmd === "plugin:dialog|open")!;
      let options = open.args.options as { title: string; filters: { name: string; extensions: string[] }[] };
      expect(options.title).toContain(".vst3");
      expect(options.filters[0]!.name).toContain(".vst3");
      expect(options.filters[0]!.extensions).toEqual(["clap", "vst3"]);

      setPlatformForTest("mac");
      calls = mock({ "plugin:dialog|open": () => null });
      await startInstall();
      open = calls.find((c) => c.cmd === "plugin:dialog|open")!;
      options = open.args.options as { title: string; filters: { name: string; extensions: string[] }[] };
      expect(options.title).not.toContain(".vst3");
      expect(options.filters[0]!.name).not.toContain(".vst3");
    });
  });
});
