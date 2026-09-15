import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { PluginInstallResultDto } from "../ipc/bindings";
import { setPlatformForTest } from "../ui/platform";
import InstallPluginDialog from "./InstallPluginDialog.svelte";
import { installFrom, pluginsState, resetPluginsForTest } from "./plugins.svelte";
import { settle } from "./testing";

let cleanup: (() => void) | null = null;

function mountDialog(): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(InstallPluginDialog, { target });
  cleanup = () => {
    unmount(app);
    target.remove();
  };
  flushSync();
  return target;
}

function mockInstall(answer: (replace: boolean) => PluginInstallResultDto | Promise<PluginInstallResultDto>): { replace: boolean[] } {
  const seen = { replace: [] as boolean[] };
  mockIPC((cmd, args) => {
    if (cmd === "plugins_install") {
      const replace = (args as { replace: boolean }).replace;
      seen.replace.push(replace);
      return answer(replace);
    }
    if (cmd === "plugins_list") return [];
    return null;
  });
  return seen;
}

afterEach(() => {
  cleanup?.();
  cleanup = null;
  clearMocks();
  resetPluginsForTest();
  setPlatformForTest(null);
});

const q = (root: ParentNode, id: string) => root.querySelector<HTMLElement>(`[data-testid="${id}"]`);
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";
const SOURCE = "/home/u/Downloads/acme-deesser.clap";

describe("Install module dialog (T-809)", () => {
  it("shows progress while copying and scanning, with the trust note", async () => {
    mockInstall(() => new Promise(() => {}));
    const root = mountDialog();
    void installFrom(SOURCE, false);
    await settle();
    const dialog = q(root, "plugin-install-dialog")!;
    expect(dialog.dataset.phase).toBe("installing");
    expect(text(q(root, "plugin-install-message"))).toContain("acme-deesser.clap");
    expect(dialog.querySelector("progress")).not.toBeNull();
    expect(text(dialog)).toContain("native code");
    expect(dialog.querySelector("button[data-role]")).toBeNull();
  });

  it("asks before replacing, then reports the effects it added", async () => {
    setPlatformForTest("linux");
    const seen = mockInstall((replace) =>
      replace
        ? {
            kind: "installed",
            path: "/home/u/.clap/acme-deesser.clap",
            replaced: true,
            effects: [
              { id: "clap:a", name: "De-esser" },
              { id: "clap:b", name: "De-esser (stereo)" },
            ],
          }
        : { kind: "collision", path: "/home/u/.clap/acme-deesser.clap" },
    );
    const root = mountDialog();
    await installFrom(SOURCE, false);
    flushSync();
    let dialog = q(root, "plugin-install-dialog")!;
    expect(dialog.dataset.phase).toBe("collision");
    expect(dialog.getAttribute("role")).toBe("alertdialog");
    expect(text(dialog.querySelector("h2"))).toBe("Replace acme-deesser.clap?");
    expect(text(q(root, "plugin-install-message"))).toContain("/home/u/.clap");
    // Linux order: Cancel, then the (destructive) Replace last.
    const buttons = [...dialog.querySelectorAll<HTMLButtonElement>("button[data-role]")].map((b) => b.dataset.testid);
    expect(buttons).toEqual(["plugin-install-cancel", "plugin-install-replace"]);
    expect(q(root, "plugin-install-replace")!.dataset.variant).toBe("danger");

    q(root, "plugin-install-replace")!.click();
    await settle();
    expect(seen.replace).toEqual([false, true]);
    dialog = q(root, "plugin-install-dialog")!;
    expect(dialog.dataset.phase).toBe("installed");
    expect(text(q(root, "plugin-install-message"))).toBe("acme-deesser.clap was replaced in /home/u/.clap.");
    expect([...root.querySelectorAll('[data-testid="plugin-install-effect"]')].map(text)).toEqual([
      "De-esser",
      "De-esser (stereo)",
    ]);
    expect(text(dialog)).toContain("Added 2 effects");
    q(root, "plugin-install-done")!.click();
    flushSync();
    expect(q(root, "plugin-install-dialog")).toBeNull();
  });

  it("Escape cancels the collision prompt", async () => {
    mockInstall(() => ({ kind: "collision", path: "/home/u/.clap/acme-deesser.clap" }));
    const root = mountDialog();
    await installFrom(SOURCE, false);
    flushSync();
    q(root, "plugin-install-dialog")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(pluginsState().install.phase).toBe("idle");
    expect(q(root, "plugin-install-dialog")).toBeNull();
  });

  it("explains a failure, its detail and the blocklisting, and opens the manager on it", async () => {
    mockInstall(() => ({
      kind: "failed",
      code: "scan_crashed",
      detail: "crashed while being scanned (signal 11)",
      blocklisted: true,
      cause: "crashed",
    }));
    const root = mountDialog();
    await installFrom(SOURCE, false);
    flushSync();
    const dialog = q(root, "plugin-install-dialog")!;
    expect(dialog.dataset.phase).toBe("failed");
    expect(text(dialog.querySelector("h2"))).toBe("Couldn't install acme-deesser.clap");
    expect(text(q(root, "plugin-install-reason"))).toBe("The plugin crashed while it was being scanned.");
    expect(text(q(root, "plugin-install-detail"))).toBe("Details: crashed while being scanned (signal 11)");
    expect(q(root, "plugin-install-blocklisted")).not.toBeNull();
    q(root, "plugin-install-show")!.click();
    flushSync();
    expect(pluginsState().open).toBe(true);
    expect(pluginsState().focusKey).toBe(`path:${SOURCE}`);
  });

  it("words a refusal of a blocklisted file with its cause, without a detail line", async () => {
    mockInstall(() => ({
      kind: "failed",
      code: "blocklisted",
      detail: "the file is blocklisted: timed out while being scanned",
      blocklisted: true,
      cause: "timed_out",
    }));
    const root = mountDialog();
    await installFrom(SOURCE, false);
    flushSync();
    expect(text(q(root, "plugin-install-reason"))).toBe(
      "This file is on the blocklist (Timed out during scan). Unblock it in the plugin manager to try again.",
    );
    expect(q(root, "plugin-install-detail")).toBeNull();
    expect(q(root, "plugin-install-blocklisted")).toBeNull();
    q(root, "plugin-install-close")!.click();
    flushSync();
    expect(q(root, "plugin-install-dialog")).toBeNull();
  });
});
