import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { MeterSpeedPref, Settings } from "../ipc/bindings";
import { clearActionHandlers } from "../shortcuts";
import { resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { resetTransportForTest } from "../state/transport.svelte";
import { settingsFixture } from "../test/fixtures";
import MeterBridge from "./MeterBridge.svelte";

/**
 * H-123 (owner: "can i setup the speed? between fast and slow?"): the shared meter-speed control
 * — one setting for both meters, not two (the owner asked to set "the speed"), reusing the
 * analyzer's own Fast/Medium/Slow segmented-control pattern. The ballistics themselves are tested
 * in `meters/ballistics.test.ts`; `state/transport.test.ts` and `record/InputMeter.test.ts` cover
 * each meter applying the chosen speed. This file is only the shared control's own wiring.
 */

let mounted: ReturnType<typeof mount> | null = null;
let settingsSaved: Settings | undefined;

afterEach(() => {
  if (mounted) {
    unmount(mounted);
    mounted = null;
  }
  document.body.innerHTML = "";
  clearMocks();
  clearActionHandlers();
  resetRecordForTest();
  resetTransportForTest();
  resetSettingsStateForTest();
  settingsSaved = undefined;
});

async function setUp(speed: MeterSpeedPref = "medium"): Promise<void> {
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ meter_speed: speed });
    }
    if (cmd === "settings_set") {
      settingsSaved = (args as { settings: Settings }).settings;
      return settingsSaved;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await loadSettings();
}

function render(): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  mounted = mount(MeterBridge, { target });
  flushSync();
  return target;
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("MeterBridge speed control (H-123)", () => {
  it("defaults to Medium and shows all three choices", async () => {
    await setUp("medium");
    const root = render();
    const control = root.querySelector('[data-testid="meter-speed"]')!;
    expect(control).not.toBeNull();
    const options = [...control.querySelectorAll('[role="radio"]')];
    expect(options.map((o) => o.textContent?.trim())).toEqual(["Fast", "Medium", "Slow"]);
    expect(control.querySelector('[aria-checked="true"]')?.textContent?.trim()).toBe("Medium");
  });

  it("reflects a persisted Fast/Slow choice", async () => {
    await setUp("fast");
    let root = render();
    expect(
      root.querySelector('[data-testid="meter-speed"] [aria-checked="true"]')?.textContent?.trim(),
    ).toBe("Fast");

    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    resetSettingsStateForTest();
    clearMocks();

    await setUp("slow");
    root = render();
    expect(
      root.querySelector('[data-testid="meter-speed"] [aria-checked="true"]')?.textContent?.trim(),
    ).toBe("Slow");
  });

  it("saves the chosen speed as a setting (settings_set) when changed, applying to both meters", async () => {
    await setUp("medium");
    const root = render();
    const fastButton = [...root.querySelectorAll<HTMLButtonElement>('[data-testid="meter-speed"] [role="radio"]')].find(
      (b) => b.textContent?.trim() === "Fast",
    )!;
    fastButton.click();
    await settle();
    expect(settingsSaved?.meter_speed).toBe("fast");
  });

  it("renders both the input and output meters (the natural side-by-side pair)", async () => {
    await setUp();
    const root = render();
    expect(root.querySelector('[data-testid="output-meter"]')).not.toBeNull();
    // The input meter renders nothing while disarmed (H-112) — only the bridge shell and the
    // output meter are guaranteed present here.
    expect(root.querySelector('[data-testid="meter-bridge"]')).not.toBeNull();
  });
});
