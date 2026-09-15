import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { LocalizedTextDto, ModuleDescriptorDto, RackStateDto } from "../ipc/bindings";
import { rackStateDto } from "../test/fixtures";
import ManagePresetsDialog from "./ManagePresetsDialog.svelte";
import { openManagePresets, resetManagePresetsForTest } from "./managePresets.svelte";
import { loadRack, resetRackForTest } from "./rack.svelte";

/**
 * Manage Presets… dialog (H-22, SPEC-012 §2.7 follow-up): list/rename/delete/export per module
 * and for the rack, keyboard-accessible; factory presets are read-only; import validates the
 * file and asks to replace an existing name exactly like save does.
 */

interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function moduleFixture(id: string): ModuleDescriptorDto {
  return { id, name: text(id), vendor: "PowerVoice", description: text(""), features: [] };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

/** Mocks IPC (with events), records every call, and loads the rack registry so the module tab has
 * something to select. */
async function setup(
  modules: ModuleDescriptorDto[],
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
): Promise<Call[]> {
  const calls: Call[] = [];
  const rack: RackStateDto = rackStateDto();
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      if (cmd in handlers) {
        return handlers[cmd]!(a);
      }
      if (cmd === "rack_list_modules") return modules;
      if (cmd === "rack_get") return rack;
      throw new Error(`unmocked command: ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  await loadRack();
  return calls;
}

function mountDialog(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ManagePresetsDialog, { target });
  flushSync();
  return { target, app };
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  resetManagePresetsForTest();
  document.body.innerHTML = "";
});

describe("Manage Presets… dialog (H-22)", () => {
  it("is closed until openManagePresets is called", async () => {
    await setup([]);
    const { target, app } = mountDialog();
    expect(target.querySelector('[data-testid="manage-presets-dialog"]')).toBeNull();
    unmount(app);
  });

  it("lists factory presets read-only and user presets with rename/delete/export (rack tab)", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [
        { key: "podcast_voice", name: text("Podcast voice"), is_factory: true },
        { key: "Mine", name: text("Mine"), is_factory: false },
      ],
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    expect(calls.some((c) => c.cmd === "rack_presets_list")).toBe(true);
    const rows = target.querySelectorAll('[data-testid="manage-presets-row"]');
    expect(rows.length).toBe(2);
    expect(rows[0]!.getAttribute("data-factory")).toBe("true");
    expect(rows[0]!.querySelector("button")).toBeNull();
    expect(rows[1]!.getAttribute("data-factory")).toBe("false");
    expect(target.querySelector('[data-testid="manage-presets-rename-Mine"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="manage-presets-delete-Mine"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="manage-presets-export-Mine"]')).not.toBeNull();

    unmount(app);
  });

  it("the module tab selects a module and lists that module's presets", async () => {
    const calls = await setup([moduleFixture("org.powervoice.gain"), moduleFixture("org.powervoice.eq")], {
      module_presets_list: (a) =>
        a.moduleId === "org.powervoice.eq" ? [{ key: "Bright", name: text("Bright"), is_factory: false }] : [],
    });
    openManagePresets({ tab: "module", moduleId: "org.powervoice.eq" });
    const { target, app } = mountDialog();
    await settle();

    expect(calls).toContainEqual({ cmd: "module_presets_list", args: { moduleId: "org.powervoice.eq" } });
    expect(target.querySelector('[data-testid="manage-presets-row"]')?.textContent).toContain("Bright");

    unmount(app);
  });

  it("renaming a preset calls rename and refreshes the list (rack tab, keyboard confirm)", async () => {
    let renamed = false;
    const calls = await setup([], {
      rack_presets_list: () =>
        renamed
          ? [{ key: "Renamed", name: text("Renamed"), is_factory: false }]
          : [{ key: "Mine", name: text("Mine"), is_factory: false }],
      rack_preset_rename: (a) => {
        renamed = true;
        expect(a).toEqual({ oldName: "Mine", newName: "Renamed" });
        return { key: "Renamed", name: text("Renamed"), is_factory: false };
      },
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-rename-Mine"]')!.click();
    flushSync();
    const input = target.querySelector<HTMLInputElement>('[data-testid="manage-presets-rename-input"]')!;
    input.value = "Renamed";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    // Keyboard confirm (Enter), not just a click — part of the "keyboard accessible" requirement.
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();
    await settle();

    expect(calls.some((c) => c.cmd === "rack_preset_rename")).toBe(true);
    expect(target.querySelector('[data-testid="manage-presets-rename-dialog"]')).toBeNull();
    expect(target.querySelector('[data-testid="manage-presets-list"]')?.textContent).toContain("Renamed");

    unmount(app);
  });

  it("Escape cancels a rename without calling rename", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [{ key: "Mine", name: text("Mine"), is_factory: false }],
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-rename-Mine"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="manage-presets-rename-dialog"]')).not.toBeNull();
    target
      .querySelector('[data-testid="manage-presets-rename-dialog"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();

    expect(target.querySelector('[data-testid="manage-presets-rename-dialog"]')).toBeNull();
    expect(calls.some((c) => c.cmd === "rack_preset_rename")).toBe(false);

    unmount(app);
  });

  it("deleting a preset asks to confirm (destructively) before calling delete", async () => {
    let deleted = false;
    const calls = await setup([], {
      rack_presets_list: () => (deleted ? [] : [{ key: "Mine", name: text("Mine"), is_factory: false }]),
      rack_preset_delete: () => {
        deleted = true;
        return null;
      },
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-delete-Mine"]')!.click();
    flushSync();
    expect(calls.some((c) => c.cmd === "rack_preset_delete")).toBe(false);
    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-delete-confirm"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual({ cmd: "rack_preset_delete", args: { name: "Mine" } });
    expect(target.querySelector('[data-testid="manage-presets-empty"]')).not.toBeNull();

    unmount(app);
  });

  it("deleting can be cancelled without calling delete", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [{ key: "Mine", name: text("Mine"), is_factory: false }],
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-delete-Mine"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-delete-cancel"]')!.click();
    flushSync();

    expect(target.querySelector('[data-testid="manage-presets-delete-dialog"]')).toBeNull();
    expect(calls.some((c) => c.cmd === "rack_preset_delete")).toBe(false);

    unmount(app);
  });

  it("exports a preset to the path chosen by the native save dialog", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [{ key: "Mine", name: text("Mine"), is_factory: false }],
      "plugin:dialog|save": () => "/home/u/Mine.json",
      rack_preset_export: () => null,
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-export-Mine"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual({ cmd: "rack_preset_export", args: { name: "Mine", path: "/home/u/Mine.json" } });

    unmount(app);
  });

  it("imports a file chosen by the native open dialog", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [],
      "plugin:dialog|open": () => "/home/u/Mine.json",
      rack_preset_import: () => ({ key: "Mine", name: text("Mine"), is_factory: false }),
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-import"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual({
      cmd: "rack_preset_import",
      args: { path: "/home/u/Mine.json", overwrite: false },
    });

    unmount(app);
  });

  it("import asks to replace when the file's name already exists, and replaces on confirm", async () => {
    const calls = await setup([], {
      rack_presets_list: () => [{ key: "Mine", name: text("Mine"), is_factory: false }],
      "plugin:dialog|open": () => "/home/u/Mine.json",
      rack_preset_import: (a) => {
        if (a.overwrite) {
          return { key: "Mine", name: text("Mine"), is_factory: false };
        }
        throw { code: "invalid_argument", key: "error.preset_already_exists", params: { name: "Mine" } };
      },
    });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-import"]')!.click();
    flushSync();
    await settle();

    const conflict = target.querySelector('[data-testid="manage-presets-import-conflict-dialog"]');
    expect(conflict).not.toBeNull();
    expect(conflict!.textContent).toContain("Mine");
    target.querySelector<HTMLButtonElement>('[data-testid="manage-presets-import-conflict-confirm"]')!.click();
    flushSync();
    await settle();

    expect(calls).toContainEqual({
      cmd: "rack_preset_import",
      args: { path: "/home/u/Mine.json", overwrite: true },
    });

    unmount(app);
  });

  it("Escape closes the dialog when nothing else is open", async () => {
    await setup([], { rack_presets_list: () => [] });
    openManagePresets({ tab: "rack" });
    const { target, app } = mountDialog();
    await settle();

    target
      .querySelector('[data-testid="manage-presets-dialog"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();

    expect(target.querySelector('[data-testid="manage-presets-dialog"]')).toBeNull();
    unmount(app);
  });
});
