import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import App from "../../App.svelte";
import { clearActionHandlers } from "../shortcuts";
import { resetLayoutForTest } from "../layout/layoutSettings.svelte";
import { closePluginManager, resetPluginsForTest } from "../plugins/plugins.svelte";
import { resetRackForTest } from "../rack/rack.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import { resetTransportForTest } from "../state/transport.svelte";
import { rackStateDto, settingsFixture, transportStateDto } from "../test/fixtures";
import { resetTourForTest } from "./tour.svelte";
import { TOUR_IDS, TOURS } from "./tours";

/**
 * Every tour step's target exists in the rendered app shell (T-709): removing or renaming a
 * `data-tour` anchor fails here. A step lists fallbacks in preference order; the last one is the
 * anchor that must always be there. A step's `enter` hook runs first (it may switch the dock tab
 * or open the plugin manager, as it does in the real tour).
 */
const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

afterEach(() => {
  closePluginManager();
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetRackForTest();
  resetLayoutForTest();
  resetSettingsStateForTest();
  resetPluginsForTest();
  resetTourForTest();
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
});

describe("tour anchors exist in the app shell (T-709)", () => {
  it("every step of every tour has its target", async () => {
    stubSize(1280, 720);
    mockIPC((cmd) => {
      switch (cmd) {
        case "app_info":
          return { name: "PowerVoice", version: "9.9.9" };
        case "settings_get":
          // The Welcome offer answered, so it doesn't sit over the shell.
          return settingsFixture({ tours: { progress: [{ id: "welcome", version: 1, outcome: "dismissed" }] } });
        case "settings_set":
          return null;
        case "transport_get":
          return transportStateDto();
        case "clock_now_ns":
          return 0;
        case "rack_list_modules":
        case "plugins_list":
        case "plugins_folders":
        case "recovery_list":
          return [];
        case "rack_get":
          return rackStateDto();
        default:
          return null;
      }
    });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });
    await settle();
    await settle();

    const missing: string[] = [];
    let checked = 0;
    for (const id of TOUR_IDS) {
      for (const step of TOURS[id].steps) {
        step.enter?.();
        await settle();
        const required = step.target?.at(-1);
        if (required === undefined) {
          continue;
        }
        checked += 1;
        if (!target.querySelector(`[data-tour="${required}"]`) && !document.querySelector(`[data-tour="${required}"]`)) {
          missing.push(`${id}/${step.id} → data-tour="${required}"`);
        }
      }
    }

    expect(missing).toEqual([]);
    expect(checked).toBeGreaterThan(20);
    unmount(app);
    target.remove();
  });
});
