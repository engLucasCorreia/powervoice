import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App.svelte";
import { clearActionHandlers } from "./lib/keymap";
import { resetRackForTest } from "./lib/rack/rack.svelte";
import { resetTransportForTest } from "./lib/state/transport.svelte";

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetRackForTest();
});

describe("App shell", () => {
  it("renders all five layout regions and the mocked app_info version", async () => {
    mockIPC((cmd) => {
      if (cmd === "app_info") {
        return { name: "PowerVoice", version: "9.9.9" };
      }
      if (cmd === "settings_get") {
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
        };
      }
      if (cmd === "transport_get") {
        return {
          playing: false,
          playhead_samples: 0,
          play_start_samples: 0,
          doc_len_samples: 0,
          doc_rate_hz: 0,
          can_play: false,
        };
      }
      if (cmd === "clock_now_ns") {
        return 0;
      }
      if (cmd === "rack_list_modules") {
        return [];
      }
      if (cmd === "rack_get") {
        return { slots: [], ab: false, latency_samples: 0 };
      }
      // telemetry_subscribe, event listeners, …
      return null;
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });

    // `onMount` awaits the mocked `invoke()` call before setting `version`; let that microtask
    // chain settle, then force Svelte to flush the resulting state update into the DOM.
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    for (const testId of ["toolbar", "editor", "rack", "markers-properties", "meter-bridge"]) {
      expect(target.querySelector(`[data-testid="${testId}"]`), `missing region: ${testId}`).not.toBeNull();
    }
    expect(target.querySelector('[data-testid="app-version"]')?.textContent).toBe("9.9.9");
    expect(target.querySelector('[data-testid="transport-time"]')?.textContent).toBe("00:00:00.000");

    unmount(app);
    target.remove();
  });
});
