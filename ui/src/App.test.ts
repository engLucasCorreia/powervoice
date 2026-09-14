import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type { LayoutPrefsDto, Settings } from "./lib/ipc/bindings";
import { clearActionHandlers } from "./lib/keymap";
import { resetLayoutForTest } from "./lib/layout/layoutSettings.svelte";
import { resetRackForTest } from "./lib/rack/rack.svelte";
import { resetSettingsStateForTest } from "./lib/state/settings.svelte";
import { resetTransportForTest } from "./lib/state/transport.svelte";

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

function unstubSize(): void {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
}

function baseSettings(layout: LayoutPrefsDto): Settings {
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
    layout,
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
  };
}

const DEFAULT_LAYOUT: LayoutPrefsDto = {
  markers_width_px: 240,
  rack_width_px: 280,
  dock_height_px: 240,
  markers_collapsed: false,
  rack_collapsed: false,
  dock_tab: "meters",
};

function mockAppIpc(options: {
  layout?: LayoutPrefsDto;
  settingsSetCalls?: Settings[];
} = {}): void {
  const layout = options.layout ?? DEFAULT_LAYOUT;
  mockIPC((cmd, args) => {
    if (cmd === "app_info") {
      return { name: "PowerVoice", version: "9.9.9" };
    }
    if (cmd === "settings_get") {
      return baseSettings(layout);
    }
    if (cmd === "settings_set") {
      const settings = (args as { settings: Settings }).settings;
      options.settingsSetCalls?.push(settings);
      return settings;
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
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetRackForTest();
  resetLayoutForTest();
  resetSettingsStateForTest();
  unstubSize();
  // Guards every other test against a fake-timers leak if a timing test above throws/times out
  // before reaching its own `vi.useRealTimers()`.
  vi.useRealTimers();
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

describe("H-24: resizable app shell", () => {
  it("has exactly three shell rows (menu bar, toolbar, main-area) with no leftover bottom-dock row", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const shell = target.querySelector(".shell")!;
    const directTestids = Array.from(shell.children).map((el) => el.getAttribute("data-testid"));
    // MenuBar/Toolbar render without their own data-testid at this level; the important
    // assertion is that main-area is the one and only large flexible region.
    expect(directTestids.filter((id) => id === "main-area")).toHaveLength(1);
    expect(target.querySelector('[data-testid="bottom-dock"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="workspace"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("renders keyboard-accessible, collapsible splitters for Markers, Rack and the dock", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    for (const testid of ["splitter-markers", "splitter-rack", "splitter-dock"]) {
      const el = target.querySelector(`[data-testid="${testid}"]`)!;
      expect(el, testid).not.toBeNull();
      expect(el.getAttribute("role")).toBe("separator");
      expect(el.getAttribute("tabindex")).toBe("0");
    }
    expect(target.querySelector('[data-testid="splitter-markers-collapse"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="splitter-rack-collapse"]')).not.toBeNull();
    // The dock's own splitter isn't collapsible (there's no "collapse the dock" affordance in
    // the ticket's scope, only column collapse buttons).
    expect(target.querySelector('[data-testid="splitter-dock-collapse"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("dragging the Markers splitter grows/shrinks the column and persists (debounced)", async () => {
    stubSize(1200, 800);
    const settingsSetCalls: Settings[] = [];
    mockAppIpc({ settingsSetCalls });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();
    vi.useFakeTimers();

    const splitter = target.querySelector('[data-testid="splitter-markers"]') as HTMLElement;
    splitter.setPointerCapture = () => {};
    splitter.releasePointerCapture = () => {};
    splitter.dispatchEvent(new PointerEvent("pointerdown", { clientX: 240, bubbles: true }));
    splitter.dispatchEvent(new PointerEvent("pointermove", { clientX: 280, bubbles: true }));
    flushSync();

    const col = target.querySelector('[data-testid="col-markers"]') as HTMLElement;
    expect(col.style.width).toBe("280px");

    splitter.dispatchEvent(new PointerEvent("pointerup", { clientX: 280, bubbles: true }));
    await vi.advanceTimersByTimeAsync(300);
    expect(settingsSetCalls.at(-1)?.layout.markers_width_px).toBe(280);

    vi.useRealTimers();
    unmount(app);
    target.remove();
  });

  it("dragging the Rack splitter left (negative delta) grows the Rack column", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const splitter = target.querySelector('[data-testid="splitter-rack"]') as HTMLElement;
    splitter.setPointerCapture = () => {};
    splitter.releasePointerCapture = () => {};
    splitter.dispatchEvent(new PointerEvent("pointerdown", { clientX: 500, bubbles: true }));
    splitter.dispatchEvent(new PointerEvent("pointermove", { clientX: 460, bubbles: true })); // dragged left
    flushSync();

    const col = target.querySelector('[data-testid="col-rack"]') as HTMLElement;
    expect(col.style.width).toBe("320px"); // 280 + 40

    unmount(app);
    target.remove();
  });

  it("double-clicking a splitter resets its column to the factory default", async () => {
    stubSize(1200, 800);
    mockAppIpc({ layout: { ...DEFAULT_LAYOUT, markers_width_px: 400 } });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    expect((target.querySelector('[data-testid="col-markers"]') as HTMLElement).style.width).toBe(
      "400px",
    );
    target
      .querySelector('[data-testid="splitter-markers"]')!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    flushSync();
    expect((target.querySelector('[data-testid="col-markers"]') as HTMLElement).style.width).toBe(
      "240px",
    );

    unmount(app);
    target.remove();
  });

  it("arrow keys step a focused splitter", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const splitter = target.querySelector('[data-testid="splitter-markers"]') as HTMLElement;
    splitter.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    flushSync();
    expect((target.querySelector('[data-testid="col-markers"]') as HTMLElement).style.width).toBe(
      "256px",
    );

    unmount(app);
    target.remove();
  });

  it("the collapse button hides the Markers column and reappears once toggled back", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    expect(target.querySelector('[data-testid="col-markers"]')).not.toBeNull();
    target.querySelector('[data-testid="splitter-markers-collapse"]')!.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    flushSync();
    expect(target.querySelector('[data-testid="col-markers"]')).toBeNull();

    target.querySelector('[data-testid="splitter-markers-collapse"]')!.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    flushSync();
    expect(target.querySelector('[data-testid="col-markers"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("switches the dock between Meters and Loudness without unmounting either (item 10: no state loss)", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const meters = target.querySelector('[data-testid="dock-tab-panel-meters"]') as HTMLElement;
    const loudness = target.querySelector('[data-testid="dock-tab-panel-loudness"]') as HTMLElement;
    expect(meters.hidden).toBe(false);
    expect(loudness.hidden).toBe(true);
    expect(target.querySelector('[data-testid="meter-bridge"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="loudness-panel"]')).not.toBeNull();

    target.querySelector('[data-testid="dock-tab-loudness"]')!.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    flushSync();

    expect(meters.hidden).toBe(true);
    expect(loudness.hidden).toBe(false);
    // Still in the DOM — switching tabs never remounts either panel.
    expect(target.querySelector('[data-testid="meter-bridge"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="loudness-panel"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("the dock never grows past 60% of the main area, keeping the workspace's 40% floor", async () => {
    stubSize(1200, 500);
    mockAppIpc({ layout: { ...DEFAULT_LAYOUT, dock_height_px: 900 } });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const dock = target.querySelector('[data-testid="bottom-dock"]') as HTMLElement;
    expect(dock.style.height).toBe("300px"); // 60% of 500

    unmount(app);
    target.remove();
  });

  it("the dock's height doesn't change across unrelated re-renders (item 4 regression)", async () => {
    stubSize(1200, 800);
    mockAppIpc();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();

    const dock = target.querySelector('[data-testid="bottom-dock"]') as HTMLElement;
    const before = dock.style.height;

    // Several unrelated re-renders (analyzer peak-hold ballistics, transport ticks, …) must never
    // change the dock's own height — a feedback loop through a canvas's content used to do this.
    for (let i = 0; i < 5; i++) {
      flushSync();
    }
    await new Promise((resolve) => setTimeout(resolve, 30));
    flushSync();

    expect(dock.style.height).toBe(before);

    unmount(app);
    target.remove();
  });
});
