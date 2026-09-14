import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { Settings } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import PreferencesDialog from "./PreferencesDialog.svelte";
import { closePreferences, openPreferences, preferencesState, resetPreferencesForTest } from "./preferences.svelte";

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
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetSettingsStateForTest();
  resetPreferencesForTest();
});

function mountDialog(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(PreferencesDialog, { target });
  flushSync();
  return { target, app };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}

function q<T extends Element = HTMLElement>(root: HTMLElement, id: string): T | null {
  return root.querySelector<T>(`[data-testid="${id}"]`);
}

describe("Preferences dialog (H-17 item 5)", () => {
  it("stays hidden until openPreferences is called", () => {
    const { target, app } = mountDialog();
    expect(q(target, "preferences-dialog")).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows the current memory budget and saves a change on commit (SPEC-004 §2.4/§3)", async () => {
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") return makeSettings();
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const slider = q<HTMLInputElement>(target, "preferences-memory-budget")!;
    expect(Number(slider.value)).toBe(2048);
    expect(q(target, "preferences-memory-budget-value")?.textContent).toContain("2.1 GB");

    slider.value = "4096";
    slider.dispatchEvent(new Event("input", { bubbles: true }));
    slider.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();

    expect(lastSaved?.memory_budget_mib).toBe(4096);

    unmount(app);
    target.remove();
  });

  it("closes on Close and on Escape", async () => {
    mockIPC((cmd) => (cmd === "settings_get" ? makeSettings() : null));
    openPreferences();
    const { target, app } = mountDialog();
    await settle();

    expect(preferencesState().open).toBe(true);
    q<HTMLButtonElement>(target, "preferences-close")!.click();
    flushSync();
    expect(preferencesState().open).toBe(false);
    expect(q(target, "preferences-dialog")).toBeNull();

    openPreferences();
    flushSync();
    q(target, "preferences-dialog")!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    flushSync();
    expect(preferencesState().open).toBe(false);

    closePreferences();
    unmount(app);
    target.remove();
  });
});
