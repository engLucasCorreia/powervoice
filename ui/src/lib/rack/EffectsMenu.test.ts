import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearActionHandlers, registerAction } from "../keymap";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { resetNrCaptureForTest } from "./nrCapture.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetNormalizeLufsForTest } from "../state/normalizeLufs.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import EffectsMenu from "./EffectsMenu.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetMenuBarForTest();
  resetNrCaptureForTest();
  resetNormalizeForTest();
  resetNormalizeLufsForTest();
  resetRecordForTest();
  resetSelectionForTest();
});

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EffectsMenu, { target });
  flushSync();
  return { target, app };
}

function openMenu(target: HTMLElement): void {
  target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-effects"]')!.click();
  flushSync();
}

describe("EffectsMenu (H-19)", () => {
  it("is a closed dropdown with a menubar-item trigger by default", () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector('[data-testid="menu-trigger-effects"]');
    expect(trigger?.getAttribute("role")).toBe("menuitem");
    expect(target.querySelector('[data-testid="effects-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Normalize…/Normalize (LUFS)…/Capture Noise Print are disabled without a selection", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    for (const id of ["menu-normalize-dialog", "menu-normalize-lufs-dialog", "menu-capture-noise-print"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        true,
      );
    }
    unmount(app);
    target.remove();
  });

  it("shows Capture Noise Print's registry shortcut label", () => {
    const { target, app } = mountMenu();
    openMenu(target);
    expect(
      target.querySelector('[data-testid="menu-capture-noise-print"] .shortcut')?.textContent,
    ).toBe("Shift+P");
    unmount(app);
    target.remove();
  });

  it("enables Normalize…/Normalize (LUFS)…/Capture Noise Print with a selection", () => {
    setSelectionFromResult([0, 100]);
    const { target, app } = mountMenu();
    openMenu(target);
    for (const id of ["menu-normalize-dialog", "menu-normalize-lufs-dialog", "menu-capture-noise-print"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled, id).toBe(
        false,
      );
    }
    unmount(app);
    target.remove();
  });

  it("Normalize…/Normalize (LUFS)… open their dialogs and close the menu", () => {
    setSelectionFromResult([0, 100]);
    const { target, app } = mountMenu();
    openMenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="menu-normalize-dialog"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="effects-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Capture Noise Print dispatches the same action as Shift+P", () => {
    setSelectionFromResult([0, 100]);
    const handler = vi.fn();
    registerAction("nr.capture_noise_print", handler);
    const { target, app } = mountMenu();
    openMenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="menu-capture-noise-print"]')!.click();
    expect(handler).toHaveBeenCalledOnce();
    unmount(app);
    target.remove();
  });

  describe("Favorites submenu", () => {
    it("is a submenu trigger listing the dB and LUFS presets", () => {
      setSelectionFromResult([0, 100]);
      const { target, app } = mountMenu();
      openMenu(target);
      const trigger = target.querySelector('[data-testid="menu-favorites"]');
      expect(trigger?.getAttribute("role")).toBe("menuitem");
      expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
      trigger!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      flushSync();

      expect(target.querySelector('[data-testid="menu-favorites-normalize-1-0db"]')).not.toBeNull();
      expect(
        target.querySelector('[data-testid="menu-favorites-normalize-lufs-16"]'),
      ).not.toBeNull();
      unmount(app);
      target.remove();
    });

    it("presets are disabled without a selection", () => {
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-favorites"]')!.click();
      flushSync();
      expect(
        target.querySelector<HTMLButtonElement>(
          '[data-testid="menu-favorites-normalize-1-0db"]',
        )?.disabled,
      ).toBe(true);
      unmount(app);
      target.remove();
    });

    it("picking a preset calls the normalize command and closes the whole menu", async () => {
      setSelectionFromResult([0, 100]);
      let started = false;
      mockIPC((cmd) => {
        if (cmd === "edit_normalize_peak_start") {
          started = true;
          return { job_id: 1 };
        }
        // Mirrors the pre-H-19 `FavoritesMenu.test.ts` convention: throw for anything
        // unexpected (rather than a catch-all `null`) so `ensureListening()`'s `listen(...)`
        // call — an untested implementation detail of `normalize.svelte.ts`, not this menu —
        // fails fast and is swallowed by its own try/catch, instead of "succeeding" against a
        // mock that doesn't actually implement the Tauri event plugin (which then breaks
        // `resetNormalizeForTest()`'s real unlisten call in `afterEach`).
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-favorites"]')!.click();
      flushSync();
      target
        .querySelector<HTMLButtonElement>('[data-testid="menu-favorites-normalize-1-0db"]')!
        .click();
      await Promise.resolve();
      await Promise.resolve();
      flushSync();

      expect(started).toBe(true);
      expect(target.querySelector('[data-testid="effects-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });
  });
});
