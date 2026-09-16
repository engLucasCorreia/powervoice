import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RackSlotDto } from "../ipc/bindings";
import { pluginsState, refreshPlugins, resetPluginsForTest } from "../plugins/plugins.svelte";
import { settle } from "../plugins/testing";
import { FLAGGED_ID, pluginFixtures } from "../test/fixtures";
import { resetRackForTest } from "./rack.svelte";
import RackSlot from "./RackSlot.svelte";

/** T-809 item 4: a plugin that crashed at runtime shows a warning key that opens the manager. */

function slot(moduleId: string): RackSlotDto {
  return {
    uid: 5,
    module: `${moduleId}@1.4.2`,
    module_id: moduleId,
    name: "Breath Control",
    bypass: false,
    latency_samples: 0,
    status: { kind: "active" },
    params: [],
    groups: [],
    values: [],
    noise_profile: null,
    curve_handles: null,
    transfer_handles: null,
    telemetry: [],
    sandboxed: true,
    automation_dropped: false,
    has_editor: false,
    editor_open: false,
  };
}

function render(s: RackSlotDto): { target: HTMLElement; teardown: () => void } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackSlot, {
    target,
    props: {
      slot: s,
      index: 0,
      rateHz: 48_000,
      dragOver: false,
      ondragstart: () => {},
      ondragover: () => {},
      ondrop: () => {},
      ondragend: () => {},
    },
  });
  flushSync();
  return {
    target,
    teardown: () => {
      unmount(app);
      target.remove();
    },
  };
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  resetPluginsForTest();
});

describe("flagged plugin in the rack (T-809)", () => {
  it("shows a warning key naming the crash count that opens the manager on the plugin", async () => {
    mockIPC((cmd) => (cmd === "plugins_list" ? pluginFixtures() : cmd === "plugins_folders" ? null : null));
    await refreshPlugins();
    const { target, teardown } = render(slot(FLAGGED_ID));
    const flag = target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-flagged"]')!;
    expect(flag).not.toBeNull();
    expect(flag.getAttribute("aria-label")).toBe("This plugin has crashed 3 times — open the plugin manager");
    flag.click();
    await settle();
    expect(pluginsState().open).toBe(true);
    expect(pluginsState().focusKey).toBe(FLAGGED_ID);
    teardown();
  });

  it("shows nothing for a plugin that never crashed", async () => {
    mockIPC((cmd) => (cmd === "plugins_list" ? pluginFixtures() : null));
    await refreshPlugins();
    const { target, teardown } = render(slot("clap:com.acme.deesser"));
    expect(target.querySelector('[data-testid="rack-slot-flagged"]')).toBeNull();
    teardown();
  });
});

/** H-62: a sandboxed slot whose event ring dropped automation shows a small warning icon. */
describe("automation-dropped flag in the rack (H-62)", () => {
  it("shows a warning icon naming what happened when the slot is flagged", () => {
    const { target, teardown } = render({ ...slot("clap:com.acme.deesser"), automation_dropped: true });
    const flag = target.querySelector('[data-testid="rack-slot-automation-dropped"]')!;
    expect(flag).not.toBeNull();
    expect(flag.getAttribute("aria-label")).toBe("This plugin fell behind and missed some parameter changes");
    teardown();
  });

  it("shows nothing while the slot hasn't dropped any automation", () => {
    const { target, teardown } = render(slot("clap:com.acme.deesser"));
    expect(target.querySelector('[data-testid="rack-slot-automation-dropped"]')).toBeNull();
    teardown();
  });
});
