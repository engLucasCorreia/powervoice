import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RackSlotDto } from "../ipc/bindings";
import { settle } from "../plugins/testing";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import { closeAllPluginWindows, rackState, resetRackForTest } from "./rack.svelte";
import RackSlot from "./RackSlot.svelte";

/**
 * T-901 item 4: a sandboxed plugin slot offers "Open plugin window" (button, double-click on the
 * name, slot menu); the button shows whether the window is open and closes it again; a plugin
 * without a GUI gets a disabled button that still explains why; in-process modules get none.
 */

function plugin(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  return rackSlotDto({
    uid: 5,
    module: "clap:com.acme.deesser@1.4.2",
    module_id: "clap:com.acme.deesser",
    name: "De-esser",
    sandboxed: true,
    has_editor: true,
    params: [],
    ...overrides,
  });
}

function render(s: RackSlotDto): { target: HTMLElement; teardown: () => void } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackSlot, {
    target,
    props: {
      slot: s,
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
  return {
    target,
    teardown: () => {
      unmount(app);
      target.remove();
    },
  };
}

type Call = { cmd: string; args: Record<string, unknown> };

function recordCalls(result: (cmd: string) => unknown): Call[] {
  const calls: Call[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
    return result(cmd);
  });
  return calls;
}

const button = (target: HTMLElement): HTMLButtonElement =>
  target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-window"]')!;

afterEach(() => {
  clearMocks();
  resetRackForTest();
});

describe("plugin window affordance (T-901)", () => {
  it("opens the plugin's window with a localized title", async () => {
    const opened = rackStateDto([plugin({ editor_open: true })]);
    const calls = recordCalls((cmd) => (cmd === "rack_editor_open" ? opened : null));
    const { target, teardown } = render(plugin());
    const b = button(target);
    expect(b).not.toBeNull();
    expect(b.getAttribute("aria-label")).toBe("Open plugin window");
    expect(b.getAttribute("aria-pressed")).toBe("false");
    b.click();
    await settle();
    expect(calls).toContainEqual({
      cmd: "rack_editor_open",
      args: { slot: 2, title: "De-esser — PowerVoice" },
    });
    expect(rackState().state.slots[0]?.editor_open).toBe(true);
    teardown();
  });

  it("shows an open window as pressed and closes it", async () => {
    const calls = recordCalls((cmd) => (cmd === "rack_editor_close" ? rackStateDto([plugin()]) : null));
    const { target, teardown } = render(plugin({ editor_open: true }));
    const b = button(target);
    expect(b.getAttribute("aria-pressed")).toBe("true");
    expect(b.getAttribute("aria-label")).toBe("Close plugin window");
    b.click();
    await settle();
    expect(calls).toContainEqual({ cmd: "rack_editor_close", args: { slot: 2 } });
    teardown();
  });

  it("opens the window on a double-click on the slot's name", async () => {
    const calls = recordCalls(() => rackStateDto([plugin({ editor_open: true })]));
    const { target, teardown } = render(plugin());
    const name = target.querySelector<HTMLElement>('[data-testid="rack-slot-name"]')!;
    name.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    expect(calls.map((c) => c.cmd)).toContain("rack_editor_open");
    teardown();
  });

  it("offers the window in the slot menu", async () => {
    const calls = recordCalls(() => rackStateDto([plugin({ editor_open: true })]));
    const { target, teardown } = render(plugin());
    target.querySelector<HTMLButtonElement>('[data-testid="rack-slot-menu"]')!.click();
    flushSync();
    const item = document.querySelector<HTMLElement>('[data-testid="rack-slot-window-menu"]')!;
    expect(item).not.toBeNull();
    expect(item.textContent).toContain("Open plugin window");
    item.click();
    await settle();
    expect(calls.map((c) => c.cmd)).toContain("rack_editor_open");
    teardown();
  });

  it("disables the button with an explanation for a plugin without a window", async () => {
    const calls = recordCalls(() => null);
    const { target, teardown } = render(plugin({ has_editor: false }));
    const b = button(target);
    expect(b.getAttribute("aria-disabled")).toBe("true");
    expect(b.getAttribute("aria-label")).toBe("This plugin has no window of its own");
    b.click();
    target
      .querySelector<HTMLElement>('[data-testid="rack-slot-name"]')!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    expect(calls).toEqual([]);
    teardown();
  });

  it("offers no window for an in-process module or a slot that isn't running", () => {
    mockIPC(() => null);
    const builtin = render(rackSlotDto());
    expect(button(builtin.target)).toBeNull();
    builtin.teardown();
    const failed = render(plugin({ status: { kind: "failed", message: "De-esser crashed" } }));
    expect(button(failed.target)).toBeNull();
    failed.teardown();
  });

  it("closes every plugin window", async () => {
    const calls = recordCalls(() => rackStateDto([plugin()]));
    await closeAllPluginWindows();
    expect(calls.map((c) => c.cmd)).toEqual(["rack_editor_close_all"]);
  });
});
