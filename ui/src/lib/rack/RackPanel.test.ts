import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { LocalizedTextDto, ModuleDescriptorDto, RackSlotDto, RackStateDto } from "../ipc/bindings";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { rackSlotDto, rackStateDto as rackFixture, transportStateDto } from "../test/fixtures";
import { resetRackForTest } from "./rack.svelte";
import RackPanel from "./RackPanel.svelte";

/**
 * Rack panel tests (SPEC-012 §2.1): the Add-module menu grouped by feature, the A/B
 * listening-only badge, the latency readout, drag-reorder wired to `rack_move`, and the
 * `MAX_SLOTS` limit disabling Add. Widget/schema detail is in `ParamControl.test.ts` and
 * `RackSlot.test.ts`.
 */

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function moduleFixture(id: string, features: string[]): ModuleDescriptorDto {
  return { id, name: text(id), vendor: "PowerVoice", description: text(""), features };
}

function slotFixture(uid: number): RackSlotDto {
  return rackSlotDto({ uid, name: `Slot ${uid}`, params: [], values: [] });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  resetTransportForTest();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function setup(rack: RackStateDto, modules: ModuleDescriptorDto[]) {
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "rack_list_modules":
          return modules;
        case "rack_get":
          return rack;
        case "rack_add":
          return rackFixture([...rack.slots, slotFixture(99)]);
        case "rack_ab":
          return rackFixture(rack.slots, (args as { on: boolean }).on, rack.latency_samples);
        case "rack_move":
          return rackFixture([...rack.slots].reverse());
        case "transport_get":
          return transportStateDto({ doc_rate_hz: 48_000 });
        case "clock_now_ns":
          return 0;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  const stopTransport = await initTransport();
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RackPanel, { target });
  await settle();
  const el = (id: string): HTMLElement | null => target.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  return {
    target,
    el,
    teardown: () => {
      unmount(app);
      stopTransport();
    },
  };
}

describe("RackPanel", () => {
  it("shows the empty state with no slots", async () => {
    const { el, teardown } = await setup(rackFixture([]), []);
    expect(el("rack-unavailable")).toBeNull();
    expect(el("rack")?.textContent).toContain("No modules yet");
    teardown();
  });

  it("groups the Add-module menu by feature", async () => {
    const modules = [
      moduleFixture("org.powervoice.parametric-eq", ["equalizer"]),
      moduleFixture("org.powervoice.gain", ["utility"]),
      moduleFixture("org.powervoice.dynamics", ["compressor"]),
    ];
    const { target, el, teardown } = await setup(rackFixture([]), modules);
    el("rack-add")?.click();
    flushSync();
    const groupTitles = [...target.querySelectorAll('[data-testid="rack-add-menu"] .heading')].map((g) => g.textContent);
    expect(groupTitles).toEqual(["EQ", "Dynamics", "Utility"]);
    const items = [...target.querySelectorAll('[data-testid="rack-add-item"]')].map((i) =>
      i.getAttribute("data-module-id"),
    );
    expect(items).toEqual([
      "org.powervoice.parametric-eq",
      "org.powervoice.dynamics",
      "org.powervoice.gain",
    ]);
    teardown();
  });

  it("lists installed CLAP effects under Plugins (CLAP), after the built-in categories", async () => {
    const modules = [
      moduleFixture("clap:com.acme.deesser", ["audio-effect", "restoration"]),
      moduleFixture("org.powervoice.gain", ["utility"]),
      moduleFixture("clap:com.acme.eq", ["audio-effect", "equalizer"]),
    ];
    const { target, el, teardown } = await setup(rackFixture([]), modules);
    el("rack-add")?.click();
    flushSync();
    const groupTitles = [...target.querySelectorAll('[data-testid="rack-add-menu"] .heading')].map((g) => g.textContent);
    expect(groupTitles).toEqual(["Utility", "Plugins (CLAP)"]);
    const items = [...target.querySelectorAll('[data-testid="rack-add-item"]')].map((i) =>
      i.getAttribute("data-module-id"),
    );
    expect(items).toEqual(["org.powervoice.gain", "clap:com.acme.deesser", "clap:com.acme.eq"]);
    teardown();
  });

  it("lists installed VST3 effects under Plugins (VST3), after Plugins (CLAP)", async () => {
    const modules = [
      moduleFixture("vst3:84E8DE5F92554F5396FAE4133C935A18", ["audio-effect", "compressor"]),
      moduleFixture("clap:com.acme.eq", ["audio-effect", "equalizer"]),
      moduleFixture("org.powervoice.gain", ["utility"]),
    ];
    const { target, el, teardown } = await setup(rackFixture([]), modules);
    el("rack-add")?.click();
    flushSync();
    const groupTitles = [...target.querySelectorAll('[data-testid="rack-add-menu"] .heading')].map((g) => g.textContent);
    expect(groupTitles).toEqual(["Utility", "Plugins (CLAP)", "Plugins (VST3)"]);
    const items = [...target.querySelectorAll('[data-testid="rack-add-item"]')].map((i) =>
      i.getAttribute("data-module-id"),
    );
    expect(items).toEqual([
      "org.powervoice.gain",
      "clap:com.acme.eq",
      "vst3:84E8DE5F92554F5396FAE4133C935A18",
    ]);
    teardown();
  });

  it("lists installed LV2 effects under Plugins (LV2), after Plugins (VST3)", async () => {
    const modules = [
      moduleFixture("lv2:http://lsp-plug.in/plugins/lv2/compressor_mono", ["audio-effect", "compressor"]),
      moduleFixture("vst3:84E8DE5F92554F5396FAE4133C935A18", ["audio-effect", "compressor"]),
      moduleFixture("org.powervoice.gain", ["utility"]),
    ];
    const { target, el, teardown } = await setup(rackFixture([]), modules);
    el("rack-add")?.click();
    flushSync();
    const groupTitles = [...target.querySelectorAll('[data-testid="rack-add-menu"] .heading')].map((g) => g.textContent);
    expect(groupTitles).toEqual(["Utility", "Plugins (VST3)", "Plugins (LV2)"]);
    const items = [...target.querySelectorAll('[data-testid="rack-add-item"]')].map((i) =>
      i.getAttribute("data-module-id"),
    );
    expect(items).toEqual([
      "org.powervoice.gain",
      "vst3:84E8DE5F92554F5396FAE4133C935A18",
      "lv2:http://lsp-plug.in/plugins/lv2/compressor_mono",
    ]);
    teardown();
  });

  it("adding a module appends it at the end of the chain", async () => {
    const { target, el, teardown } = await setup(rackFixture([slotFixture(1)]), [
      moduleFixture("org.powervoice.gain", ["utility"]),
    ]);
    el("rack-add")?.click();
    flushSync();
    target.querySelector<HTMLElement>('[data-testid="rack-add-item"]')?.click();
    await settle();
    expect(target.querySelectorAll('[data-testid="rack-slot"]')).toHaveLength(2);
    teardown();
  });

  it("disables Add once the rack has 16 slots", async () => {
    const slots = Array.from({ length: 16 }, (_, i) => slotFixture(i));
    const { el, teardown } = await setup(rackFixture(slots), []);
    expect((el("rack-add") as HTMLButtonElement).disabled).toBe(true);
    teardown();
  });

  it("A/B shows the listening-only badge when on", async () => {
    const { el, teardown } = await setup(rackFixture([slotFixture(1)], true), []);
    expect(el("rack-ab-badge")).toBeTruthy();
    el("rack-ab")?.click();
    await settle();
    teardown();
  });

  it("shows the total latency readout when > 0", async () => {
    const { el, teardown } = await setup(rackFixture([slotFixture(1)], false, 480), []);
    expect(el("rack-latency")?.textContent).toBe("10.0 ms (480 samples)");
    teardown();
  });

  it("drag-reordering a slot sends rack_move with the source and drop index", async () => {
    const calls: unknown[] = [];
    const rack = rackFixture([slotFixture(1), slotFixture(2)]);
    mockIPC(
      (cmd, args) => {
        if (cmd === "rack_list_modules") return [];
        if (cmd === "rack_get") return rack;
        if (cmd === "rack_move") {
          calls.push(args);
          return rackFixture([...rack.slots].reverse());
        }
        if (cmd === "transport_get") {
          return transportStateDto({ doc_rate_hz: 48_000 });
        }
        if (cmd === "clock_now_ns") return 0;
        return null;
      },
      { shouldMockEvents: true },
    );
    const stopTransport = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(RackPanel, { target });
    await settle();
    const slots = target.querySelectorAll<HTMLElement>('[data-testid="rack-slot"]');
    slots[0]!.dispatchEvent(new Event("dragstart", { bubbles: true }));
    slots[1]!.dispatchEvent(new Event("dragover", { bubbles: true, cancelable: true }));
    slots[1]!.dispatchEvent(new Event("drop", { bubbles: true, cancelable: true }));
    await settle();
    expect(calls).toEqual([{ from: 0, to: 1 }]);
    unmount(app);
    stopTransport();
  });
});
