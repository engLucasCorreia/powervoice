import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetDocumentStateForTest } from "../document/document.svelte";
import type { RecordStateDto, Settings } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { openNewRecordingPrompt, recordState, resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import NewRecordingDialog from "./NewRecordingDialog.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

function settingsFixture(): Settings {
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
    default_format: { sample_rate_hz: 48_000, bit_depth: "24" },
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
    theme: "dark",
    playhead_follow: true,
  };
}

function recordedDto(rate: number): RecordStateDto {
  return {
    input_device: "Mic",
    input_channel: 1,
    input_status: "healthy",
    armed: true,
    input_open: true,
    input_rate_hz: rate,
    recording: true,
    finishing: false,
    monitor: "off",
    monitoring: false,
    monitor_latency_us: null,
    monitor_dropouts: 0,
    dropout_count: 0,
    disk_remaining_s: null,
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
  resetSettingsStateForTest();
});

describe("NewRecordingDialog (H-06)", () => {
  it("is hidden with no pending prompt", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NewRecordingDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="new-recording-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("prefills from the current default format", async () => {
    mockIPC((cmd) => (cmd === "settings_get" ? settingsFixture() : null));
    await loadSettings();
    openNewRecordingPrompt({ sample_rate_hz: 96_000, bit_depth: "32f" });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NewRecordingDialog, { target });
    flushSync();

    const rateChecked = target.querySelector<HTMLInputElement>(
      'input[name="new-recording-rate"]:checked',
    );
    const bitsChecked = target.querySelector<HTMLInputElement>(
      'input[name="new-recording-bits"]:checked',
    );
    expect(rateChecked?.value).toBe("96000");
    expect(bitsChecked?.value).toBe("32f");

    unmount(app);
    target.remove();
  });

  it("H-10 item 7: offers 88.2 kHz alongside 44.1/48/96 kHz (SPEC-002 §2.2)", () => {
    openNewRecordingPrompt({ sample_rate_hz: 48_000, bit_depth: "24" });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NewRecordingDialog, { target });
    flushSync();

    const rates = Array.from(
      target.querySelectorAll<HTMLInputElement>('input[name="new-recording-rate"]'),
    ).map((r) => r.value);
    expect(rates).toEqual(["44100", "48000", "88200", "96000"]);

    unmount(app);
    target.remove();
  });

  it("picking a format then confirming saves it as the default and starts recording", async () => {
    let savedArgs: unknown;
    let startArgs: unknown;
    mockIPC((cmd, args) => {
      switch (cmd) {
        case "settings_get":
          return settingsFixture();
        case "settings_set":
          savedArgs = args;
          return (args as { settings: Settings }).settings;
        case "record_start":
          startArgs = args;
          return recordedDto(44_100);
        default:
          return null;
      }
    });
    await loadSettings();
    openNewRecordingPrompt({ sample_rate_hz: 48_000, bit_depth: "24" });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NewRecordingDialog, { target });
    flushSync();

    target
      .querySelectorAll<HTMLInputElement>('input[name="new-recording-rate"]')
      .forEach((r) => {
        if (r.value === "44100") {
          r.click();
        }
      });
    flushSync();
    target
      .querySelectorAll<HTMLInputElement>('input[name="new-recording-bits"]')
      .forEach((r) => {
        if (r.value === "16") {
          r.click();
        }
      });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="new-recording-confirm"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(savedArgs).toEqual({
      settings: { ...settingsFixture(), default_format: { sample_rate_hz: 44_100, bit_depth: "16" } },
    });
    expect(startArgs).toEqual({
      replace: true,
      format: { sample_rate_hz: 44_100, bit_depth: "16" },
    });
    expect(recordState().newRecordingPrompt).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Cancel clears the prompt without recording", () => {
    openNewRecordingPrompt({ sample_rate_hz: 48_000, bit_depth: "24" });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NewRecordingDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="new-recording-cancel"]')!.click();
    flushSync();
    expect(recordState().newRecordingPrompt).toBeNull();
    expect(target.querySelector('[data-testid="new-recording-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
