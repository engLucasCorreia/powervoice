import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { isPlatformMac, SHORTCUTS } from "../shortcuts/registry";
import { shortcutRows } from "../shortcuts/shortcutLabel";
import ShortcutsDialog from "./ShortcutsDialog.svelte";
import { openShortcutsDialog, resetShortcutsDialogForTest } from "./shortcutsDialog.svelte";

afterEach(() => {
  resetShortcutsDialogForTest();
});

describe("ShortcutsDialog (T-701)", () => {
  it("renders nothing when closed", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ShortcutsDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="shortcuts-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("lists every registry entry with the registry's own display label (never a hand-typed one)", () => {
    openShortcutsDialog();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ShortcutsDialog, { target });
    flushSync();

    const dialog = target.querySelector('[data-testid="shortcuts-dialog"]');
    expect(dialog?.getAttribute("role")).toBe("dialog");
    expect(dialog?.getAttribute("aria-modal")).toBe("true");

    for (const row of shortcutRows(isPlatformMac())) {
      const el = target.querySelector(`[data-testid="shortcuts-row-${row.entry.action}"]`);
      expect(el, `missing row for ${row.entry.action}`).not.toBeNull();
      expect(el!.textContent).toContain(row.display);
    }

    unmount(app);
    target.remove();
  });

  it("groups rows by scope, with a heading per non-empty scope", () => {
    openShortcutsDialog();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ShortcutsDialog, { target });
    flushSync();

    const headings = [...target.querySelectorAll("h3")].map((h) => h.textContent);
    // Today only "global" and "waveform" have entries (dialog/text-input are reserved, T-701's
    // report explains why) — this fails the moment a "dialog"/"text-input" entry is added without
    // giving it a heading too.
    const scopesWithEntries = new Set(SHORTCUTS.map((s) => s.scope));
    expect(scopesWithEntries).toEqual(new Set(["global", "waveform"]));
    expect(headings).toEqual(["General", "Waveform view"]);

    unmount(app);
    target.remove();
  });

  it("Escape closes the dialog", () => {
    openShortcutsDialog();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ShortcutsDialog, { target });
    flushSync();
    target
      .querySelector('[data-testid="shortcuts-dialog"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="shortcuts-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Close button closes the dialog", () => {
    openShortcutsDialog();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ShortcutsDialog, { target });
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="shortcuts-close"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="shortcuts-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });
});
