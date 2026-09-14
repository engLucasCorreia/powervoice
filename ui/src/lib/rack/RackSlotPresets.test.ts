import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { PresetEntryDto, RackSlotDto, RackStateDto } from "../ipc/bindings";
import { resetRackForTest } from "./rack.svelte";
import RackSlot from "./RackSlot.svelte";

/** T-406 (SPEC-012 §2.7): the slot menu's Presets submenu — list, load, save, delete, and
 * Reset to Default — drives the right IPC commands with the right arguments. */

function slotFixture(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  const gainParam = {
    id: 0,
    key: "gain_db",
    name: { text: "Gain", key: null },
    group: null,
    unit: { kind: "db" as const },
    min: -60,
    max: 24,
    default: 0,
    taper: { kind: "linear" as const },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 0,
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
  };
  return {
    uid: 1,
    module: "org.powervoice.gain@1.0.0",
    module_id: "org.powervoice.gain",
    name: "Gain",
    bypass: false,
    latency_samples: 0,
    status: { kind: "active" },
    params: [gainParam],
    groups: [],
    values: [{ id: 0, value: 0, normalized: 0.5, text: "0.0 dB" }],
    noise_profile: null,
    curve_handles: null,
    telemetry: [],
    ...overrides,
  };
}

const emptyRack: RackStateDto = { slots: [], ab: false, latency_samples: 0 };

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
});

function render(slot: RackSlotDto, onCommand: (cmd: string, args: unknown) => unknown) {
  mockIPC((cmd, args) => {
    if (cmd === "rack_get") {
      return emptyRack;
    }
    return onCommand(cmd, args);
  });
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackSlot, {
    target,
    props: {
      slot,
      index: 2,
      rateHz: 48_000,
      dragOver: false,
      ondragstart: () => {},
      ondragover: () => {},
      ondrop: () => {},
      ondragend: () => {},
    },
  });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function openPresetsSubmenu(target: HTMLElement): Promise<void> {
  target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-menu"]')!.click();
  flushSync();
  target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-presets"]')!.click();
  flushSync();
  await settle();
}

describe("slot preset menu (T-406, SPEC-012 §2.7)", () => {
  it("lists factory and user presets when opened", async () => {
    const calls: Array<[string, unknown]> = [];
    const factoryEntry: PresetEntryDto = {
      key: "boost_6db",
      name: { text: "Boost (+6 dB)", key: "module.gain.preset.boost_6db" },
      is_factory: true,
    };
    const userEntry: PresetEntryDto = {
      key: "My preset",
      name: { text: "My preset", key: null },
      is_factory: false,
    };
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_presets_list") {
        return [factoryEntry, userEntry];
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    await openPresetsSubmenu(target);

    expect(calls).toContainEqual(["module_presets_list", { moduleId: "org.powervoice.gain" }]);
    const submenu = target.querySelector('[data-testid="rack-slot-presets-submenu"]')!;
    expect(submenu.textContent).toContain("Boost (+6 dB)");
    expect(submenu.textContent).toContain("My preset");
    // Only the user preset gets a delete button.
    expect(target.querySelector('[data-testid="rack-slot-preset-delete-boost_6db"]')).toBeNull();
    expect(
      target.querySelector('[data-testid="rack-slot-preset-delete-My preset"]'),
    ).not.toBeNull();
    teardown();
  });

  it("loads a factory preset with the right PresetRefDto", async () => {
    const calls: Array<[string, unknown]> = [];
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_presets_list") {
        return [{ key: "boost_6db", name: { text: "Boost", key: null }, is_factory: true }];
      }
      if (cmd === "module_preset_load") {
        return emptyRack;
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    await openPresetsSubmenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-boost_6db"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual([
      "module_preset_load",
      {
        slot: 2,
        moduleId: "org.powervoice.gain",
        preset: { kind: "factory", key: "boost_6db" },
      },
    ]);
    teardown();
  });

  it("loads a user preset with a User PresetRefDto", async () => {
    const calls: Array<[string, unknown]> = [];
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_presets_list") {
        return [{ key: "Mine", name: { text: "Mine", key: null }, is_factory: false }];
      }
      if (cmd === "module_preset_load") {
        return emptyRack;
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    await openPresetsSubmenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-Mine"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual([
      "module_preset_load",
      { slot: 2, moduleId: "org.powervoice.gain", preset: { kind: "user", name: "Mine" } },
    ]);
    teardown();
  });

  it("saves the current state as a new preset with the typed name", async () => {
    const calls: Array<[string, unknown]> = [];
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_presets_list") {
        return [];
      }
      if (cmd === "module_preset_save") {
        return { key: "Mine", name: { text: "Mine", key: null }, is_factory: false };
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    await openPresetsSubmenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-save"]')!.click();
    flushSync();
    const input = target.querySelector<HTMLInputElement>('[data-testid="rack-slot-preset-name"]')!;
    input.value = "Mine";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    target
      .querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-save-confirm"]')!
      .click();
    flushSync();
    await settle();

    expect(calls).toContainEqual([
      "module_preset_save",
      { slot: 2, name: "Mine", includeNoisePrint: false, overwrite: false },
    ]);
    teardown();
  });

  it("does not show the include-noise-print checkbox for a module with no noise profile", async () => {
    const { target, teardown } = render(slotFixture(), (cmd) => {
      if (cmd === "module_presets_list") return [];
      throw new Error(`unexpected command ${cmd}`);
    });
    await openPresetsSubmenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-save"]')!.click();
    flushSync();
    expect(target.querySelector("input[type=checkbox]")).toBeNull();
    teardown();
  });

  it("deletes a user preset and refreshes the list", async () => {
    const calls: Array<[string, unknown]> = [];
    let deleted = false;
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_presets_list") {
        return deleted
          ? []
          : [{ key: "Mine", name: { text: "Mine", key: null }, is_factory: false }];
      }
      if (cmd === "module_preset_delete") {
        deleted = true;
        return null;
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    await openPresetsSubmenu(target);
    target
      .querySelector<HTMLButtonElement>('[data-testid="rack-slot-preset-delete-Mine"]')!
      .click();
    flushSync();
    await settle();
    flushSync();

    expect(calls).toContainEqual([
      "module_preset_delete",
      { moduleId: "org.powervoice.gain", name: "Mine" },
    ]);
    const submenu = target.querySelector('[data-testid="rack-slot-presets-submenu"]')!;
    expect(submenu.textContent).toContain("No saved presets");
    teardown();
  });

  it("resets the slot to its defaults", async () => {
    const calls: Array<[string, unknown]> = [];
    const { target, teardown } = render(slotFixture(), (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "module_reset_default") {
        return emptyRack;
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-menu"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-reset-default"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual(["module_reset_default", { slot: 2 }]);
    teardown();
  });

  it("hides the Presets and Reset to Default items for a placeholder slot", () => {
    const { target, teardown } = render(slotFixture({ module_id: null }), () => {
      throw new Error("no command expected");
    });
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-menu"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="rack-slot-presets"]')).toBeNull();
    expect(target.querySelector('[data-testid="rack-slot-reset-default"]')).toBeNull();
    teardown();
  });
});
