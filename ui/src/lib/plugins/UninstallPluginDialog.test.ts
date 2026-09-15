import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { pluginEntry } from "../test/fixtures";
import { cancelUninstall, confirmUninstall, pluginsState, requestUninstall, resetPluginsForTest } from "./plugins.svelte";
import { settle } from "./testing";
import UninstallPluginDialog from "./UninstallPluginDialog.svelte";

/**
 * H-29: the Plugin Manager's row ⋯ menu → "Uninstall…" confirmation. Mounted independently of
 * `PluginManagerDialog` (like `InstallPluginDialog`'s own test file) since the two are separate
 * components, both mounted in `App.svelte`.
 */

let cleanup: (() => void) | null = null;

function mountDialog(): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(UninstallPluginDialog, { target });
  cleanup = () => {
    unmount(app);
    target.remove();
  };
  flushSync();
  return target;
}

function mockUninstall(answer: () => unknown = () => null): { calls: Record<string, unknown>[] } {
  const calls: Record<string, unknown>[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "plugins_uninstall") return answer();
    if (cmd === "plugins_list") return [];
    return null;
  });
  return { calls };
}

afterEach(() => {
  cleanup?.();
  cleanup = null;
  clearMocks();
  clearNotices();
  resetPluginsForTest();
});

const q = (root: ParentNode, id: string) => root.querySelector<HTMLElement>(`[data-testid="${id}"]`);
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";
const PLUGIN = pluginEntry({ path: "/home/u/.clap/acme-deesser.clap" });

describe("Uninstall confirm dialog (H-29)", () => {
  it("is hidden until requested, and names the plugin", () => {
    mockUninstall();
    const root = mountDialog();
    expect(q(root, "plugin-uninstall-dialog")).toBeNull();

    requestUninstall(PLUGIN);
    flushSync();
    const dialog = q(root, "plugin-uninstall-dialog")!;
    expect(dialog).not.toBeNull();
    expect(dialog.getAttribute("role")).toBe("alertdialog");
    expect(text(dialog.querySelector("h2"))).toBe("Uninstall De-esser?");
    expect(text(q(root, "plugin-uninstall-message"))).toContain("removes the plugin file");
  });

  it("Cancel closes it without calling the command", () => {
    const { calls } = mockUninstall();
    const root = mountDialog();
    requestUninstall(PLUGIN);
    flushSync();
    q(root, "plugin-uninstall-cancel")!.click();
    flushSync();
    expect(q(root, "plugin-uninstall-dialog")).toBeNull();
    expect(calls.some((c) => c.cmd === "plugins_uninstall")).toBe(false);
  });

  it("Escape cancels it too", () => {
    mockUninstall();
    const root = mountDialog();
    requestUninstall(PLUGIN);
    flushSync();
    q(root, "plugin-uninstall-dialog")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(q(root, "plugin-uninstall-dialog")).toBeNull();
  });

  it("Uninstall removes the file, disabling the buttons meanwhile, then closes", async () => {
    let resolve!: () => void;
    const { calls } = mockUninstall(() => new Promise<null>((r) => (resolve = () => r(null))));
    const root = mountDialog();
    requestUninstall(PLUGIN);
    flushSync();
    q(root, "plugin-uninstall-confirm")!.click();
    flushSync();
    expect(calls.find((c) => c.cmd === "plugins_uninstall")?.args).toEqual({
      path: "/home/u/.clap/acme-deesser.clap",
    });
    // `loading` (not native `disabled`) on the confirm button — it keeps focus and swallows
    // clicks (Button.svelte) — while Cancel is genuinely disabled meanwhile.
    expect(q(root, "plugin-uninstall-confirm")!.getAttribute("aria-disabled")).toBe("true");
    expect(q(root, "plugin-uninstall-cancel")!.hasAttribute("disabled")).toBe(true);

    resolve();
    await settle();
    expect(q(root, "plugin-uninstall-dialog")).toBeNull();
    expect(pluginsState().uninstallPrompt).toBeNull();
  });

  it("a failed removal closes the prompt too (the error shows as a toast elsewhere)", async () => {
    mockUninstall(() => {
      throw { code: "invalid_argument", key: "error.plugins.uninstall.outside_install_folder", params: {} };
    });
    const root = mountDialog();
    requestUninstall(PLUGIN);
    flushSync();
    await confirmUninstall();
    flushSync();
    expect(q(root, "plugin-uninstall-dialog")).toBeNull();
  });

  it("cancelUninstall is a no-op once the removal is already in flight", () => {
    mockUninstall(() => new Promise(() => {}));
    const root = mountDialog();
    requestUninstall(PLUGIN);
    flushSync();
    q(root, "plugin-uninstall-confirm")!.click();
    flushSync();
    cancelUninstall();
    flushSync();
    expect(q(root, "plugin-uninstall-dialog")).not.toBeNull();
  });
});
