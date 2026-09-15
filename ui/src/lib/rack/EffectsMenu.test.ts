import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RackStateDto } from "../ipc/bindings";
import { clearActionHandlers, registerAction } from "../keymap";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetNormalizeLufsForTest } from "../state/normalizeLufs.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import EffectsMenu from "./EffectsMenu.svelte";
import { resetNrCaptureForTest } from "./nrCapture.svelte";
import { loadRack, resetRackForTest } from "./rack.svelte";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { resetBakeForTest } from "../state/bake.svelte";
import { applyRecordStateForTest } from "../state/record.svelte";
import { docDto } from "../test/fixtures";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetMenuBarForTest();
  resetNrCaptureForTest();
  resetNormalizeForTest();
  resetNormalizeLufsForTest();
  resetRecordForTest();
  resetSelectionForTest();
  resetRackForTest();
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

  describe("Rack Presets submenu (T-406, SPEC-012 \"the rack-preset menu\")", () => {
    async function settle(): Promise<void> {
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();
    }

    function openRackPresets(target: HTMLElement): void {
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-rack-presets"]')!.click();
      flushSync();
    }

    it("lists factory and user rack presets when opened", async () => {
      mockIPC((cmd) => {
        if (cmd === "rack_presets_list") {
          return [
            { key: "podcast_voice", name: { text: "Podcast voice", key: null }, is_factory: true },
            { key: "Mine", name: { text: "Mine", key: null }, is_factory: false },
          ];
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openRackPresets(target);
      await settle();

      const submenu = target.querySelector('[data-testid="rack-presets-submenu"]')!;
      expect(submenu.textContent).toContain("Podcast voice");
      expect(submenu.textContent).toContain("Mine");
      expect(target.querySelector('[data-testid="rack-preset-delete-podcast_voice"]')).toBeNull();
      expect(target.querySelector('[data-testid="rack-preset-delete-Mine"]')).not.toBeNull();

      unmount(app);
      target.remove();
    });

    it("loads a preset directly when the rack is empty (no replace confirmation)", async () => {
      const calls: Array<[string, unknown]> = [];
      mockIPC((cmd, args) => {
        calls.push([cmd, args]);
        if (cmd === "rack_presets_list") {
          return [{ key: "gentle_cleanup", name: { text: "Gentle cleanup", key: null }, is_factory: true }];
        }
        if (cmd === "rack_preset_load") {
          return rackStateDto();
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openRackPresets(target);
      await settle();
      target.querySelector<HTMLButtonElement>('[data-testid="rack-preset-gentle_cleanup"]')!.click();
      flushSync();
      await settle();

      expect(calls).toContainEqual([
        "rack_preset_load",
        { preset: { kind: "factory", key: "gentle_cleanup" } },
      ]);
      expect(target.querySelector('[data-testid="effects-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });

    it("asks for confirmation before replacing a non-empty rack, and loads on confirm", async () => {
      const calls: Array<[string, unknown]> = [];
      const oneSlot: RackStateDto = rackStateDto([rackSlotDto({ params: [], values: [] })]);
      mockIPC(
        (cmd, args) => {
          calls.push([cmd, args]);
          switch (cmd) {
            case "rack_list_modules":
              return [];
            case "rack_get":
              return oneSlot;
            case "rack_presets_list":
              return [{ key: "podcast_voice", name: { text: "Podcast voice", key: null }, is_factory: true }];
            case "rack_preset_load":
              return rackStateDto();
            default:
              throw new Error(`unmocked command: ${cmd}`);
          }
        },
        { shouldMockEvents: true },
      );
      const stopLoading = await loadRack();
      calls.length = 0;

      const { target, app } = mountMenu();
      openRackPresets(target);
      await settle();
      target
        .querySelector<HTMLButtonElement>('[data-testid="rack-preset-podcast_voice"]')!
        .click();
      flushSync();

      // Not loaded yet — a confirmation is shown instead.
      expect(calls.some(([cmd]) => cmd === "rack_preset_load")).toBe(false);
      const confirmButton = target.querySelector<HTMLButtonElement>(
        '[data-testid="rack-preset-confirm-replace"]',
      );
      expect(confirmButton).not.toBeNull();
      confirmButton!.click();
      flushSync();
      await settle();

      expect(calls).toContainEqual([
        "rack_preset_load",
        { preset: { kind: "factory", key: "podcast_voice" } },
      ]);

      unmount(app);
      target.remove();
      stopLoading();
    });

    it("saves the live rack as a new preset with the typed name", async () => {
      const calls: Array<[string, unknown]> = [];
      mockIPC((cmd, args) => {
        calls.push([cmd, args]);
        if (cmd === "rack_presets_list") {
          return [];
        }
        if (cmd === "rack_preset_save") {
          return { key: "Mine", name: { text: "Mine", key: null }, is_factory: false };
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openRackPresets(target);
      await settle();
      target.querySelector<HTMLButtonElement>('[data-testid="rack-preset-save"]')!.click();
      flushSync();
      const input = target.querySelector<HTMLInputElement>('[data-testid="rack-preset-name"]')!;
      input.value = "Mine";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      flushSync();
      target.querySelector<HTMLButtonElement>('[data-testid="rack-preset-save-confirm"]')!.click();
      flushSync();
      await settle();

      expect(calls).toContainEqual(["rack_preset_save", { name: "Mine", overwrite: false }]);

      unmount(app);
      target.remove();
    });

    it("deletes a user preset and refreshes the list", async () => {
      const calls: Array<[string, unknown]> = [];
      let deleted = false;
      mockIPC((cmd, args) => {
        calls.push([cmd, args]);
        if (cmd === "rack_presets_list") {
          return deleted ? [] : [{ key: "Mine", name: { text: "Mine", key: null }, is_factory: false }];
        }
        if (cmd === "rack_preset_delete") {
          deleted = true;
          return null;
        }
        throw new Error(`unmocked command: ${cmd}`);
      });
      const { target, app } = mountMenu();
      openRackPresets(target);
      await settle();
      target.querySelector<HTMLButtonElement>('[data-testid="rack-preset-delete-Mine"]')!.click();
      flushSync();
      await settle();

      expect(calls).toContainEqual(["rack_preset_delete", { name: "Mine" }]);
      const submenu = target.querySelector('[data-testid="rack-presets-submenu"]')!;
      expect(submenu.textContent).toContain("No saved presets");

      unmount(app);
      target.remove();
    });
  });
});

describe("Bake Rack (T-602)", () => {
  afterEach(() => {
    resetBakeForTest();
    resetDocumentStateForTest();
  });

  async function setUp(open: boolean, slots: RackStateDto["slots"]): Promise<void> {
    mockIPC((cmd) => {
      switch (cmd) {
        case "document_open":
          return docDto({ len_samples: 48_000 });
        case "rack_list_modules":
          return [];
        case "rack_get":
          return rackStateDto(slots);
        default:
          throw new Error(`unmocked command: ${cmd}`);
      }
    });
    if (open) {
      await openDocument("/home/user/take.wav");
    }
    const stop = await loadRack();
    stop();
    clearMocks();
  }

  function bakeItem(target: HTMLElement): HTMLButtonElement {
    return target.querySelector<HTMLButtonElement>('[data-testid="menu-bake-rack"]')!;
  }

  it("is listed after Capture Noise Print and disabled without a document", async () => {
    await setUp(false, [rackSlotDto()]);
    const { target, app } = mountMenu();
    openMenu(target);
    const item = bakeItem(target);
    expect(item.textContent).toContain("Bake Rack");
    expect(item.disabled).toBe(true);
    const ids = [...target.querySelectorAll("[data-testid]")].map((e) => e.getAttribute("data-testid"));
    expect(ids.indexOf("menu-bake-rack")).toBe(ids.indexOf("menu-capture-noise-print") + 1);
    unmount(app);
    target.remove();
  });

  it("is disabled with an empty or fully bypassed rack", async () => {
    await setUp(true, []);
    let { target, app } = mountMenu();
    openMenu(target);
    expect(bakeItem(target).disabled).toBe(true);
    unmount(app);
    target.remove();

    await setUp(true, [rackSlotDto({ bypass: true })]);
    resetMenuBarForTest();
    ({ target, app } = mountMenu());
    openMenu(target);
    expect(bakeItem(target).disabled).toBe(true);
    unmount(app);
    target.remove();
  });

  it("is enabled with a document and an active rack, and disabled while recording", async () => {
    await setUp(true, [rackSlotDto()]);
    const { target, app } = mountMenu();
    openMenu(target);
    expect(bakeItem(target).disabled).toBe(false);
    applyRecordStateForTest({ recording: true });
    flushSync();
    expect(bakeItem(target).disabled).toBe(true);
    unmount(app);
    target.remove();
  });

  it("bakes the whole file without a selection and closes the menu", async () => {
    await setUp(true, [rackSlotDto()]);
    const calls: Array<[string, unknown]> = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "edit_bake_start") {
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountMenu();
    openMenu(target);
    bakeItem(target).click();
    flushSync();
    await vi.waitFor(() =>
      expect(calls).toContainEqual(["edit_bake_start", { startSamples: 0, endSamples: 48_000 }]),
    );
    expect(target.querySelector('[data-testid="effects-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });
});
