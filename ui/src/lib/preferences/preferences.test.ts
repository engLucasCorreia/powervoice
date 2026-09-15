import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RecordOffsetEntry, Settings } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { applyRecordStateForTest, resetRecordForTest } from "../state/record.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import { settingsFixture as makeSettings } from "../test/fixtures";
import { resetThemeForTest } from "../theme/theme.svelte";
import PreferencesDialog from "./PreferencesDialog.svelte";
import { closePreferences, openPreferences, preferencesState, resetPreferencesForTest } from "./preferences.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetSettingsStateForTest();
  resetPreferencesForTest();
  resetThemeForTest();
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

describe("Preferences → Appearance (T-708)", () => {
  it("offers all four themes with previews and applies + saves a pick at once", async () => {
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") return makeSettings({ theme: "dark" });
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      if (cmd === "plugins_list") return [];
      return null;
    });
    const { loadSettings } = await import("../state/settings.svelte");
    await loadSettings();
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const appearance = q(target, "preferences-appearance")!;
    const radios = [...appearance.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
    expect(radios.map((r) => r.dataset.testid)).toEqual([
      "preferences-theme-dark",
      "preferences-theme-light",
      "preferences-theme-system",
      "preferences-theme-high_contrast",
    ]);
    expect(q(target, "preferences-theme-dark")?.getAttribute("aria-checked")).toBe("true");
    expect(appearance.querySelectorAll('[data-testid^="theme-swatch-"]').length).toBe(4);

    q<HTMLButtonElement>(target, "preferences-theme-light")!.click();
    await settle();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(lastSaved?.theme).toBe("light");
    expect(q(target, "preferences-theme-light")?.getAttribute("aria-checked")).toBe("true");

    q<HTMLButtonElement>(target, "preferences-theme-high_contrast")!.click();
    await settle();
    expect(document.documentElement.dataset.theme).toBe("high-contrast");
    expect(lastSaved?.theme).toBe("high_contrast");

    unmount(app);
    target.remove();
  });
});

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

function offsetEntry(
  input: string,
  offsetMs: number,
  source: RecordOffsetEntry["source"],
): RecordOffsetEntry {
  return {
    host: "pipewire",
    input_device: input,
    output_device: "Speakers",
    device_rate_hz: 48_000,
    offset_ms: offsetMs,
    source,
    updated_unix_ms: 1_789_000_000_000,
    confidence: source === "calibrated" ? 1 : null,
    buffer_frames: 256,
  };
}

/** Serves `initial` from `settings_get` and records the last `settings_set`. */
function mockSettings(initial: Settings): { saved: () => Settings | undefined } {
  let lastSaved: Settings | undefined;
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") return initial;
    if (cmd === "settings_set") {
      lastSaved = (args as { settings: Settings }).settings;
      return lastSaved;
    }
    return null;
  });
  return { saved: () => lastSaved };
}

describe("Preferences → Recording (H-21 item 6, SPEC-022 §2.3/§2.13)", () => {
  afterEach(() => {
    resetRecordForTest();
  });

  it("shows the Punch & pre-roll values and saves (clamped) changes", async () => {
    const ipc = mockSettings(makeSettings());
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const preroll = q<HTMLInputElement>(target, "preferences-preroll")!;
    expect(Number(preroll.value)).toBe(5);
    preroll.value = "2.5";
    preroll.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(ipc.saved()?.record.preroll_s).toBe(2.5);

    q<HTMLInputElement>(target, "preferences-record-mode-overwrite")!.click();
    await settle();
    expect(ipc.saved()?.record.mode).toBe("overwrite");

    const postroll = q<HTMLInputElement>(target, "preferences-postroll")!;
    postroll.value = "99";
    postroll.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(ipc.saved()?.record.postroll_s).toBe(20);
    expect(ipc.saved()?.record.preroll_s).toBe(2.5);

    closePreferences();
    unmount(app);
    target.remove();
  });

  it("lists the recording offsets per device setup and forgets one", async () => {
    const usb = offsetEntry("USB Mic", 3, "calibrated");
    const headset = offsetEntry("Headset", -1.25, "manual");
    const ipc = mockSettings(makeSettings({ record_offsets: [usb, headset] }));
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const rows = target.querySelectorAll('[data-testid="preferences-offset-entry"]');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.textContent).toContain("USB Mic → Speakers (pipewire, 48 kHz)");
    expect(rows[0]!.textContent).toContain("+3.00 ms · calibrated");
    expect(rows[1]!.textContent).toContain("−1.25 ms · manual");
    rows[0]!.querySelector<HTMLButtonElement>('[data-testid="preferences-offset-remove"]')!.click();
    await settle();
    expect(ipc.saved()?.record_offsets).toEqual([headset]);

    closePreferences();
    unmount(app);
    target.remove();
  });

  it("opens the calibration wizard and closes Preferences (SPEC-022 §2.14)", async () => {
    mockSettings(makeSettings());
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    q<HTMLButtonElement>(target, "preferences-calibrate")!.click();
    await settle();
    const { recordState } = await import("../state/record.svelte");
    expect(recordState().calibration?.stage).toBe("connect");
    expect(preferencesState().open).toBe(false);

    unmount(app);
    target.remove();
  });

  it("shows an empty offsets list and locks the controls while recording", async () => {
    mockSettings(makeSettings());
    openPreferences();
    await settle();
    applyRecordStateForTest({ recording: true });
    const { target, app } = mountDialog();
    await settle();

    expect(q(target, "preferences-offsets-empty")).not.toBeNull();
    expect(q(target, "preferences-recording-locked")).not.toBeNull();
    for (const id of [
      "preferences-preroll",
      "preferences-postroll",
      "preferences-xfade",
      "preferences-record-mode-insert",
      "preferences-punch-on-selection",
      "preferences-hear-original",
    ]) {
      expect(q<HTMLInputElement>(target, id)!.disabled).toBe(true);
    }

    closePreferences();
    unmount(app);
    target.remove();
  });
});

describe("Preferences → Editing (T-703)", () => {
  it("changes the multichannel file policy (SPEC-005 §2.4/§3 `multichannel_policy`)", async () => {
    const ipc = mockSettings(makeSettings({ multichannel_policy: "ask" }));
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const select = q<HTMLSelectElement>(target, "preferences-multichannel-policy")!;
    expect(select.selectedOptions[0]?.textContent).toBe("Ask each time");

    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(ipc.saved()?.multichannel_policy).toBe("always_mix");

    closePreferences();
    unmount(app);
    target.remove();
  });

  it("toggles snap-to-zero-crossing (SPEC-006 §2.10), matching the View menu's setting", async () => {
    const ipc = mockSettings(makeSettings({ snap_to_zero_crossing: false }));
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const checkbox = q<HTMLInputElement>(target, "preferences-snap-to-zero-crossing")!;
    expect(checkbox.checked).toBe(false);
    checkbox.click();
    await settle();
    expect(ipc.saved()?.snap_to_zero_crossing).toBe(true);

    closePreferences();
    unmount(app);
    target.remove();
  });
});

describe("Preferences → Advanced (T-703, SPEC-003 §3 `telemetry_rate_hz`)", () => {
  it("shows and changes the playhead/meter update rate", async () => {
    const ipc = mockSettings(makeSettings({ telemetry_rate_hz: 60 }));
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    const select = q<HTMLSelectElement>(target, "preferences-telemetry-rate")!;
    expect(select.selectedOptions[0]?.textContent).toBe("60 Hz");

    select.value = "0";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(ipc.saved()?.telemetry_rate_hz).toBe(30);

    closePreferences();
    unmount(app);
    target.remove();
  });
});

describe("Preferences → Reset to defaults (T-703 item 3)", () => {
  function mockSettingsWithDefaults(initial: Settings): { saved: () => Settings | undefined } {
    let lastSaved: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_get") return lastSaved ?? initial;
      if (cmd === "settings_set") {
        lastSaved = (args as { settings: Settings }).settings;
        return lastSaved;
      }
      if (cmd === "settings_defaults") return makeSettings();
      return null;
    });
    return { saved: () => lastSaved };
  }

  afterEach(() => {
    resetRecordForTest();
  });

  it("resets only the Recording section's fields, leaving offsets and everything else alone", async () => {
    const custom = offsetEntry("USB Mic", 3, "calibrated");
    const initial = makeSettings({
      record: { ...makeSettings().record, preroll_s: 9, mode: "overwrite" },
      record_offsets: [custom],
      theme: "light",
    });
    const ipc = mockSettingsWithDefaults(initial);
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    q<HTMLButtonElement>(target, "preferences-reset-recording")!.click();
    await settle();
    expect(q(target, "preferences-reset-dialog")).not.toBeNull();
    q<HTMLButtonElement>(target, "preferences-reset-confirm")!.click();
    await settle();

    const saved = ipc.saved();
    expect(saved?.record).toEqual(makeSettings().record);
    // Untouched by a Recording-scoped reset.
    expect(saved?.record_offsets).toEqual([custom]);
    expect(saved?.theme).toBe("light");
    expect(q(target, "preferences-reset-dialog")).toBeNull();

    closePreferences();
    unmount(app);
    target.remove();
  });

  it("Cancel leaves settings unchanged", async () => {
    const initial = makeSettings({ theme: "light" });
    const ipc = mockSettingsWithDefaults(initial);
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    q<HTMLButtonElement>(target, "preferences-reset-appearance")!.click();
    await settle();
    q<HTMLButtonElement>(target, "preferences-reset-cancel")!.click();
    await settle();

    expect(ipc.saved()).toBeUndefined();
    expect(q(target, "preferences-reset-dialog")).toBeNull();

    closePreferences();
    unmount(app);
    target.remove();
  });

  it("Reset all to defaults resets every dialog-editable field at once", async () => {
    const initial = makeSettings({
      theme: "light",
      memory_budget_mib: 4096,
      telemetry_rate_hz: 30,
      multichannel_policy: "always_mix",
      snap_to_zero_crossing: true,
      record: { ...makeSettings().record, preroll_s: 9 },
    });
    const ipc = mockSettingsWithDefaults(initial);
    openPreferences();
    await settle();
    const { target, app } = mountDialog();
    await settle();

    q<HTMLButtonElement>(target, "preferences-reset-all")!.click();
    await settle();
    q<HTMLButtonElement>(target, "preferences-reset-confirm")!.click();
    await settle();

    const defaults = makeSettings();
    const saved = ipc.saved();
    expect(saved?.theme).toBe(defaults.theme);
    expect(saved?.memory_budget_mib).toBe(defaults.memory_budget_mib);
    expect(saved?.telemetry_rate_hz).toBe(defaults.telemetry_rate_hz);
    expect(saved?.multichannel_policy).toBe(defaults.multichannel_policy);
    expect(saved?.snap_to_zero_crossing).toBe(defaults.snap_to_zero_crossing);
    expect(saved?.record).toEqual(defaults.record);

    closePreferences();
    unmount(app);
    target.remove();
  });
});
