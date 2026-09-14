import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { applyAnalyzerPrefs, resetAnalyzerForTest } from "../analyzer/analyzer.svelte";
import type { Settings } from "../ipc/bindings";
import { clearActionHandlers, registerAction } from "../keymap";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { rendererPref, resetRendererPrefForTest } from "../state/rendererPref.svelte";
import { loadSettings, resetSettingsStateForTest, settingsState } from "../state/settings.svelte";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import ViewMenu from "./ViewMenu.svelte";

function makeSettings(overrides: Partial<Settings> = {}): Settings {
  return {
    version: 1,
    device: {
      host: "pipewire",
      input_device: null,
      input_channel: 1,
      output_device: null,
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    default_format: { sample_rate_hz: 48000, bit_depth: "24" },
    monitor_mode: "off",
    monitor_hint_shown: false,
    telemetry_rate_hz: 60,
    memory_budget_mib: 2048,
    normalize_dialog: { value: -1, unit: "db" },
    recent_files: [],
    spectral_defaults: {
      freq_scale: "log",
      colormap: "inferno",
      display_floor_db: -120,
      display_ceil_db: 0,
      fft_size: null,
    },
    analyzer_visible: true,
    analyzer_response: "medium",
    analyzer_peak_hold: true,
    multichannel_policy: "ask",
    renderer_preference: "auto",
    layout: {
      markers_width_px: 240,
      rack_width_px: 280,
      dock_height_px: 240,
      markers_collapsed: false,
      rack_collapsed: false,
      dock_tab: "meters",
    },
    record: {
      mode: "insert",
      punch_on_selection: true,
      preroll_s: 5,
      postroll_s: 1,
      preroll_at_cursor: false,
      hear_original: false,
      punch_xfade_ms: 10,
    },
    record_offsets: [],
    save_dither: "tpdf",
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetAnalyzerForTest();
  resetSettingsStateForTest();
  resetMenuBarForTest();
  resetSpectralForTest();
  resetRendererPrefForTest();
});

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
