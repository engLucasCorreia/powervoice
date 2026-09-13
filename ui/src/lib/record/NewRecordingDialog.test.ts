import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetDocumentStateForTest } from "../document/document.svelte";
import type { RecordStateDto, Settings } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { openNewRecordingPrompt, recordState, resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import NewRecordingDialog from "./NewRecordingDialog.svelte";

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
    telemetry_rate_hz: 60,
    memory_budget_mib: 2048,
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
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
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
