/**
 * Dev-only preview (H-25): mounts the real App against mocked IPC so the shell can be opened —
 * and screenshotted — in a plain browser, without a Tauri backend or audio devices.
 * `main.ts` loads this only for `?preview` when `import.meta.env.DEV` (never in a build):
 *   npm --prefix ui run dev → http://localhost:1420/?preview (add `&theme=light` for light).
 * Commands it doesn't know return null, like the App shell tests.
 */
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Settings, ThemePref } from "../lib/ipc/bindings";

export function installPreviewIpc(theme: ThemePref): void {
  const settings: Settings = {
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
    theme,
  };

  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "app_info":
          return { name: "PowerVoice", version: "0.1.0" };
        case "settings_get":
          return settings;
        case "settings_set":
          return (args as { settings: Settings }).settings;
        case "transport_get":
          return {
            playing: false,
            playhead_samples: 0,
            play_start_samples: 0,
            doc_len_samples: 0,
            doc_rate_hz: 0,
            can_play: false,
          };
        case "clock_now_ns":
          return 0;
        case "rack_list_modules":
          return [];
        case "rack_get":
          return { slots: [], ab: false, latency_samples: 0 };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
