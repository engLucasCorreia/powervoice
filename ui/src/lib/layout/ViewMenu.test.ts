import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { applyAnalyzerPrefs, resetAnalyzerForTest } from "../analyzer/analyzer.svelte";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import type { Settings } from "../ipc/bindings";
import { clearActionHandlers, registerAction } from "../shortcuts";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { rendererPref, resetRendererPrefForTest } from "../state/rendererPref.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { loadSettings, resetSettingsStateForTest, settingsState } from "../state/settings.svelte";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import {
  resetWaveformViewForTest,
  setTimeRulerFormat,
  timeRulerFormatState,
} from "../state/waveformView.svelte";
import { resetTransportForTest } from "../state/transport.svelte";
import { applyThemePref, resetThemeForTest, themeState } from "../theme/theme.svelte";
import { docDto, settingsFixture as makeSettings } from "../test/fixtures";
import ViewMenu from "./ViewMenu.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetAnalyzerForTest();
  resetSettingsStateForTest();
  resetMenuBarForTest();
  resetSpectralForTest();
  resetRendererPrefForTest();
  resetThemeForTest();
  resetWaveformViewForTest();
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetTransportForTest();
});

async function openFixtureDocument(): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return docDto();
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ViewMenu, { target });
  flushSync();
  return { target, app };
}

function openMenu(target: HTMLElement): void {
  target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-view"]')!.click();
  flushSync();
}

describe("ViewMenu (H-19)", () => {
  it("is a closed dropdown with a menubar-item trigger by default", () => {
    mockIPC(() => null);
    const { target, app } = mountMenu();
    const trigger = target.querySelector('[data-testid="menu-trigger-view"]');
    expect(trigger?.getAttribute("role")).toBe("menuitem");
    expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
    expect(target.querySelector('[data-testid="view-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Spectral is a menuitemcheckbox reflecting state, with the registry's shortcut label", () => {
    mockIPC(() => null);
    spectralState().setVisible(true);
    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector('[data-testid="menu-view-spectral"]');
    expect(checkbox?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(checkbox?.getAttribute("aria-checked")).toBe("true");
    expect(target.querySelector('[data-testid="menu-view-spectral"] .shortcut')?.textContent).toBe(
      "Shift+D",
    );

    unmount(app);
    target.remove();
  });

  it("Spectral dispatches the same 'spectral.toggle' action as Shift+D, and closes the menu", () => {
    mockIPC(() => null);
    const handler = vi.fn();
    registerAction("spectral.toggle", handler);
    const { target, app } = mountMenu();
    openMenu(target);

    target.querySelector<HTMLButtonElement>('[data-testid="menu-view-spectral"]')!.click();
    expect(handler).toHaveBeenCalledOnce();
    flushSync();
    expect(target.querySelector('[data-testid="view-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Analyzer is a menuitemcheckbox reflecting visibility and toggles it on click", () => {
    mockIPC(() => null);
    applyAnalyzerPrefs({ visible: true, response: "medium", peakHold: true });
    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector('[data-testid="menu-view-analyzer"]');
    expect(checkbox?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(checkbox?.getAttribute("aria-checked")).toBe("true");
    (checkbox as HTMLButtonElement).click();
    flushSync();

    openMenu(target);
    expect(
      target.querySelector('[data-testid="menu-view-analyzer"]')?.getAttribute("aria-checked"),
    ).toBe("false");

    unmount(app);
    target.remove();
  });

  // H-28 item 3: a View-menu checkbox bound to `Settings.playhead_follow`, same pattern as
  // Analyzer — reflects the loaded setting and round-trips through settings_set on toggle.
  it("Follow Playhead is a menuitemcheckbox bound to Settings.playhead_follow", async () => {
    const fixture = makeSettings({ playhead_follow: true });
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return fixture;
      }
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      return null;
    });
    await loadSettings();

    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector('[data-testid="menu-view-playhead-follow"]');
    expect(checkbox?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(checkbox?.getAttribute("aria-checked")).toBe("true");

    (checkbox as HTMLButtonElement).click();
    flushSync();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(lastSaved?.playhead_follow).toBe(false);
    expect(settingsState().current?.playhead_follow).toBe(false);

    unmount(app);
    target.remove();
  });

  // T-206 (SPEC-006 §2.10): a View-menu checkbox bound to Settings.snap_to_zero_crossing, same
  // pattern as Follow Playhead — default off.
  it("Snap to Zero Crossing is a menuitemcheckbox bound to Settings.snap_to_zero_crossing", async () => {
    const fixture = makeSettings({ snap_to_zero_crossing: false });
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") {
        return fixture;
      }
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      return null;
    });
    await loadSettings();

    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector('[data-testid="menu-view-snap-to-zero-crossing"]');
    expect(checkbox?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(checkbox?.getAttribute("aria-checked")).toBe("false");

    (checkbox as HTMLButtonElement).click();
    flushSync();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(lastSaved?.snap_to_zero_crossing).toBe(true);
    expect(settingsState().current?.snap_to_zero_crossing).toBe(true);

    unmount(app);
    target.remove();
  });

  // H-39 item 1: Loop Playback is a menuitemcheckbox reflecting transport state, with the registry's
  // Ctrl/⌘+L shortcut label, and is disabled without a document.
  it("Loop Playback is a menuitemcheckbox bound to transport state", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return docDto();
      }
      if (cmd === "settings_get") {
        return makeSettings();
      }
      return null;
    });
    await openDocument("/home/user/take.wav");
    clearMocks();
    mockIPC(() => null);

    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector('[data-testid="menu-view-loop"]');
    expect(checkbox?.getAttribute("role")).toBe("menuitemcheckbox");
    expect(checkbox?.getAttribute("aria-checked")).toBe("false");
    expect(checkbox?.getAttribute("aria-disabled")).not.toBe("true");
    expect(target.querySelector('[data-testid="menu-view-loop"] .shortcut')?.textContent).toBe(
      "Ctrl+L",
    );

    unmount(app);
    target.remove();
  });

  it("Loop Playback is disabled without a document", () => {
    mockIPC(() => null);
    const { target, app } = mountMenu();
    openMenu(target);

    const checkbox = target.querySelector<HTMLButtonElement>('[data-testid="menu-view-loop"]');
    expect(checkbox?.disabled).toBe(true);

    unmount(app);
    target.remove();
  });

  it("Loop Playback dispatches the transport.toggle_loop action", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return docDto();
      }
      if (cmd === "settings_get") {
        return makeSettings();
      }
      return null;
    });
    await openDocument("/home/user/take.wav");
    clearMocks();

    const handler = vi.fn();
    registerAction("transport.toggle_loop", handler);
    mockIPC(() => null);

    const { target, app } = mountMenu();
    openMenu(target);

    target.querySelector<HTMLButtonElement>('[data-testid="menu-view-loop"]')!.click();
    expect(handler).toHaveBeenCalledOnce();
    flushSync();
    expect(target.querySelector('[data-testid="view-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  describe("Time Format submenu (T-206, SPEC-006 §2.5)", () => {
    it("lists timecode/samples/seconds as menuitemradio rows, the current one checked", () => {
      mockIPC(() => null);
      setTimeRulerFormat("samples");
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-time-format"]')!.click();
      flushSync();

      const rows = ["timecode", "samples", "seconds"].map((value) =>
        target.querySelector(`[data-testid="menu-time-format-${value}"]`),
      );
      expect(rows.map((row) => row?.getAttribute("role"))).toEqual(Array(3).fill("menuitemradio"));
      expect(rows.map((row) => row?.getAttribute("aria-checked"))).toEqual(["false", "true", "false"]);

      unmount(app);
      target.remove();
    });

    it("picking one updates the live store immediately (no persistence round trip needed)", () => {
      mockIPC(() => null);
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-time-format"]')!.click();
      flushSync();
      target.querySelector<HTMLButtonElement>('[data-testid="menu-time-format-seconds"]')!.click();
      flushSync();

      expect(timeRulerFormatState().current).toBe("seconds");
      expect(target.querySelector('[data-testid="view-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });
  });

  it("Zoom In/Out show the registry's shortcut labels and dispatch the matching actions", () => {
    mockIPC(() => null);
    const zoomIn = vi.fn();
    const zoomOut = vi.fn();
    registerAction("waveform.zoom_in", zoomIn);
    registerAction("waveform.zoom_out", zoomOut);
    const { target, app } = mountMenu();

    openMenu(target);
    expect(target.querySelector('[data-testid="menu-zoom-in"] .shortcut')?.textContent).toBe("=");
    target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-in"]')!.click();
    expect(zoomIn).toHaveBeenCalledOnce();

    openMenu(target);
    expect(target.querySelector('[data-testid="menu-zoom-out"] .shortcut')?.textContent).toBe("-");
    target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-out"]')!.click();
    expect(zoomOut).toHaveBeenCalledOnce();

    unmount(app);
    target.remove();
  });

  it("Zoom to Selection/Zoom Full are disabled with no document open (H-35)", () => {
    mockIPC(() => null);
    const { target, app } = mountMenu();

    openMenu(target);
    const toSelection = target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-to-selection"]')!;
    const full = target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-full"]')!;
    expect(toSelection.disabled).toBe(true);
    expect(full.disabled).toBe(true);
    // No shortcut chip — H-35: the keyboard binding is still deferred to SPEC-019.
    expect(target.querySelector('[data-testid="menu-zoom-to-selection"] .shortcut')).toBeNull();
    expect(target.querySelector('[data-testid="menu-zoom-full"] .shortcut')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Zoom to Selection is disabled without a selection, enabled with one; Zoom Full dispatches (H-35)", async () => {
    await openFixtureDocument();
    mockIPC(() => null);
    const zoomToSelection = vi.fn();
    const zoomFull = vi.fn();
    registerAction("waveform.zoom_to_selection", zoomToSelection);
    registerAction("waveform.zoom_full", zoomFull);
    const { target, app } = mountMenu();

    openMenu(target);
    const toSelection = target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-to-selection"]')!;
    expect(toSelection.disabled).toBe(true);
    const full = target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-full"]')!;
    expect(full.disabled).toBe(false);
    full.click();
    expect(zoomFull).toHaveBeenCalledOnce();

    setSelectionFromResult([100, 200]);
    openMenu(target);
    const toSelection2 = target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-to-selection"]')!;
    expect(toSelection2.disabled).toBe(false);
    toSelection2.click();
    expect(zoomToSelection).toHaveBeenCalledOnce();

    unmount(app);
    target.remove();
  });

  it("Zoom In/Out/Reset (Vertical) show the registry's shortcut labels and dispatch (H-35)", () => {
    mockIPC(() => null);
    const zoomInV = vi.fn();
    const zoomOutV = vi.fn();
    const zoomResetV = vi.fn();
    registerAction("waveform.zoom_in_vertical", zoomInV);
    registerAction("waveform.zoom_out_vertical", zoomOutV);
    registerAction("waveform.zoom_reset_vertical", zoomResetV);
    const { target, app } = mountMenu();

    openMenu(target);
    expect(target.querySelector('[data-testid="menu-zoom-in-vertical"] .shortcut')?.textContent).toBe(
      "Alt+=",
    );
    target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-in-vertical"]')!.click();
    expect(zoomInV).toHaveBeenCalledOnce();

    openMenu(target);
    expect(target.querySelector('[data-testid="menu-zoom-out-vertical"] .shortcut')?.textContent).toBe(
      "Alt+-",
    );
    target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-out-vertical"]')!.click();
    expect(zoomOutV).toHaveBeenCalledOnce();

    openMenu(target);
    expect(target.querySelector('[data-testid="menu-zoom-reset-vertical"] .shortcut')?.textContent).toBe(
      "Alt+0",
    );
    target.querySelector<HTMLButtonElement>('[data-testid="menu-zoom-reset-vertical"]')!.click();
    expect(zoomResetV).toHaveBeenCalledOnce();

    unmount(app);
    target.remove();
  });

  describe("Theme submenu (T-708, Settings.theme)", () => {
    it("lists every theme as a menuitemradio row, the current one checked", () => {
      mockIPC(() => null);
      applyThemePref("light");
      const { target, app } = mountMenu();
      openMenu(target);
      const trigger = target.querySelector<HTMLButtonElement>('[data-testid="menu-theme"]')!;
      expect(trigger.textContent).toContain("Theme");
      trigger.click();
      flushSync();

      const rows = ["dark", "light", "system", "high_contrast"].map((pref) =>
        target.querySelector(`[data-testid="menu-theme-${pref}"]`),
      );
      expect(rows.map((row) => row?.getAttribute("role"))).toEqual(Array(4).fill("menuitemradio"));
      expect(rows.map((row) => row?.getAttribute("aria-checked"))).toEqual(["false", "true", "false", "false"]);
      expect(rows[2]?.textContent).toContain("Match System");
      expect(rows[3]?.textContent).toContain("High Contrast");

      unmount(app);
      target.remove();
    });

    it("picking one applies it live (data-theme) and persists it", async () => {
      let lastSaved: Settings | undefined;
      mockIPC((cmd, args) => {
        if (cmd === "settings_get") {
          return makeSettings();
        }
        if (cmd === "settings_set") {
          lastSaved = (args as { settings: Settings }).settings;
          return lastSaved;
        }
        return null;
      });
      await loadSettings();
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-theme"]')!.click();
      flushSync();
      target.querySelector<HTMLButtonElement>('[data-testid="menu-theme-high_contrast"]')!.click();
      flushSync();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();

      expect(document.documentElement.dataset.theme).toBe("high-contrast");
      expect(themeState().pref).toBe("high_contrast");
      expect(lastSaved?.theme).toBe("high_contrast");
      expect(settingsState().current?.theme).toBe("high_contrast");

      unmount(app);
      target.remove();
    });
  });

  describe("Renderer submenu (H-19, Settings.renderer_preference)", () => {
    it("is a submenu of menuitemradio rows, Automatic checked by default", () => {
      mockIPC(() => null);
      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-renderer"]')!.click();
      flushSync();

      const auto = target.querySelector('[data-testid="menu-renderer-auto"]');
      const webgl2 = target.querySelector('[data-testid="menu-renderer-webgl2"]');
      expect(auto?.getAttribute("role")).toBe("menuitemradio");
      expect(auto?.getAttribute("aria-checked")).toBe("true");
      expect(webgl2?.getAttribute("aria-checked")).toBe("false");

      unmount(app);
      target.remove();
    });

    it("picking an option updates the live store and round-trips through settings_set", async () => {
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
        return null;
      });
      await loadSettings();

      const { target, app } = mountMenu();
      openMenu(target);
      target.querySelector<HTMLButtonElement>('[data-testid="menu-renderer"]')!.click();
      flushSync();
      target.querySelector<HTMLButtonElement>('[data-testid="menu-renderer-webgl2"]')!.click();
      flushSync();
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();

      // Applied immediately to the live H-13 renderer-selection store...
      expect(rendererPref().value).toBe("webgl2");
      // ...and persisted (round-trips through settings_set / the reloaded Settings object).
      expect(lastSaved?.renderer_preference).toBe("webgl2");
      expect(settingsState().current?.renderer_preference).toBe("webgl2");
      // The menu closes after picking an option, same as any other item.
      expect(target.querySelector('[data-testid="view-menu"]')).toBeNull();

      unmount(app);
      target.remove();
    });
  });
});
