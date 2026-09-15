import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type {
  LocalizedTextDto,
  ModuleDescriptorDto,
  ParamChangedDto,
  RackLatencyDto,
  RackSlotDto,
  RackStateDto,
} from "../ipc/bindings";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { paramInfoDto, rackSlotDto, rackStateDto as rackFixture } from "../test/fixtures";
import {
  addModule,
  flushPendingDrags,
  flushPendingPlainDrags,
  loadRack,
  moveSlot,
  rackState,
  removeSlot,
  resetRackForTest,
  restartSlot,
  setAb,
  setBypass,
  setParamNormalized,
  setParamPlain,
  setParamPlainDragged,
  setParamText,
} from "./rack.svelte";

/**
 * Rack store tests (S3-01, SPEC-012 §2.1–§2.6): command flows through the mocked IPC layer, the
 * `rack_changed`/`param_changed`/`rack_latency` events, drag coalescing, and the
 * `error.rack_unavailable` no-live-rack state. Component-level widget behavior is in
 * `ParamControl.test.ts`; slot layout (drag-reorder, group visibility) is in `RackPanel.test.ts`.
 */

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function moduleFixture(id: string): ModuleDescriptorDto {
  return { id, name: text(id), vendor: "PowerVoice", description: text(""), features: ["utility"] };
}

function slotFixture(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  const params = overrides.params ?? [paramInfoDto()];
  return rackSlotDto({ params, ...overrides });
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetRackForTest();
});

describe("loadRack", () => {
  it("loads the registry and the current rack, and subscribes to live updates", async () => {
    const calls: string[] = [];
    mockIPC(
      (cmd) => {
        calls.push(cmd);
        if (cmd === "rack_list_modules") {
          return [moduleFixture("org.powervoice.gain")];
        }
        if (cmd === "rack_get") {
          return rackFixture([slotFixture()]);
        }
        throw new Error(`unmocked command: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    const rs = rackState();
    // H-03: the slot meters' module-telemetry channel (`VXMT`) is subscribed last; an unmocked
    // (failing) subscription leaves the meters at rest without disturbing the load.
    expect(calls).toEqual(["rack_list_modules", "rack_get", "module_telemetry_subscribe"]);
    expect(rs.loading).toBe(false);
    expect(rs.modules.map((m) => m.id)).toEqual(["org.powervoice.gain"]);
    expect(rs.state.slots).toHaveLength(1);
    teardown();
  });

  it("flags error.rack_unavailable without failing the whole load", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "rack_list_modules") {
          return [];
        }
        if (cmd === "rack_get") {
          throw { code: "internal", key: "error.rack_unavailable", params: {} };
        }
        throw new Error(`unmocked command: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    const rs = rackState();
    expect(rs.loading).toBe(false);
    expect(rs.unavailable).toBe(true);
    teardown();
  });
});

describe("mutating commands", () => {
  function setupIPC(initial: RackStateDto) {
    let state = initial;
    const calls: Array<{ cmd: string; args: unknown }> = [];
    mockIPC((cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "rack_add":
          state = rackFixture([...state.slots, slotFixture({ uid: 2 })]);
          return state;
        case "rack_remove":
          state = rackFixture([]);
          return state;
        case "rack_move":
          state = rackFixture([...state.slots].reverse());
          return state;
        case "rack_bypass":
          state = rackFixture([{ ...state.slots[0]!, bypass: true }]);
          return state;
        case "rack_ab":
          state = rackFixture(state.slots, true);
          return state;
        case "rack_restart":
          return state;
        default:
          throw new Error(`unmocked command: ${cmd}`);
      }
    });
    return { calls, current: () => state };
  }

  it("addModule sends rack_add with the module id and index, and applies the result", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture()]));
    await addModule("org.powervoice.gain", 1);
    expect(calls).toEqual([{ cmd: "rack_add", args: { moduleId: "org.powervoice.gain", index: 1 } }]);
    expect(rackState().state.slots).toHaveLength(2);
  });

  it("removeSlot sends rack_remove with the slot index", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture()]));
    await removeSlot(0);
    expect(calls).toEqual([{ cmd: "rack_remove", args: { slot: 0 } }]);
    expect(rackState().state.slots).toHaveLength(0);
  });

  it("moveSlot sends rack_move with from/to", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture({ uid: 1 }), slotFixture({ uid: 2 })]));
    await moveSlot(0, 1);
    expect(calls).toEqual([{ cmd: "rack_move", args: { from: 0, to: 1 } }]);
  });

  it("setBypass sends rack_bypass with slot/on", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture()]));
    await setBypass(0, true);
    expect(calls).toEqual([{ cmd: "rack_bypass", args: { slot: 0, on: true } }]);
    expect(rackState().state.slots[0]!.bypass).toBe(true);
  });

  it("setAb sends rack_ab with on", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture()]));
    await setAb(true);
    expect(calls).toEqual([{ cmd: "rack_ab", args: { on: true } }]);
    expect(rackState().state.ab).toBe(true);
  });

  it("restartSlot sends rack_restart with the slot index", async () => {
    const { calls } = setupIPC(rackFixture([slotFixture()]));
    await restartSlot(0);
    expect(calls).toEqual([{ cmd: "rack_restart", args: { slot: 0 } }]);
  });
});

describe("setParamText (SPEC-012 §2.6: Rust parses typed text)", () => {
  it("applies the returned state and reports true on success", async () => {
    const updated = rackFixture([
      slotFixture({ values: [{ id: 0, value: 3, normalized: 0.8, text: "3.0 dB" }] }),
    ]);
    mockIPC((cmd, args) => {
      expect(cmd).toBe("param_set_text");
      expect(args).toEqual({ slot: 0, id: 0, text: "3" });
      return updated;
    });
    const ok = await setParamText(0, 0, "3");
    expect(ok).toBe(true);
    expect(rackState().state.slots[0]!.values[0]!.text).toBe("3.0 dB");
  });

  it("rejects unparseable text without applying state, and reports false", async () => {
    mockIPC(() => {
      throw { code: "invalid_argument", key: "error.rack_rejected", params: { message: "not a number" } };
    });
    const ok = await setParamText(0, 0, "not a number", { notifyOnError: false });
    expect(ok).toBe(false);
    expect(noticesState().toasts).toHaveLength(0);
  });

  it("pushes a notice on failure unless notifyOnError is false", async () => {
    mockIPC(() => {
      throw { code: "invalid_argument", key: "error.rack_rejected", params: { message: "bad" } };
    });
    await setParamText(0, 0, "bad");
    expect(noticesState().toasts).toHaveLength(1);
    expect(noticesState().toasts[0]!.key).toBe("error.rack_rejected");
  });
});

describe("setParamNormalized drag coalescing (SPEC-012 §2.4: ≤1 call/frame, latest wins)", () => {
  it("coalesces multiple updates before the next frame into one call with the latest value", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return rackFixture([slotFixture()]);
    });
    setParamNormalized(0, 0, 0.1);
    setParamNormalized(0, 0, 0.5);
    setParamNormalized(0, 0, 0.9);
    flushPendingDrags();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual([{ slot: 0, id: 0, value: 0.9 }]);
  });

  it("tracks separate slots/params independently", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return rackFixture([slotFixture()]);
    });
    setParamNormalized(0, 0, 0.2);
    setParamNormalized(1, 5, 0.7);
    flushPendingDrags();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toContainEqual({ slot: 0, id: 0, value: 0.2 });
    expect(calls).toContainEqual({ slot: 1, id: 5, value: 0.7 });
  });
});

describe("setParamPlain (S3-07, SPEC-015 §2.6.6: EQ graph plain-value gestures)", () => {
  it("sends the value immediately (double-click / Alt+click, not a drag)", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push({ cmd, args });
      return rackFixture([slotFixture()]);
    });
    await setParamPlain(0, 11, 1_234.5);
    expect(calls).toEqual([{ cmd: "param_set_plain", args: { slot: 0, id: 11, value: 1_234.5 } }]);
  });

  it("setParamPlainDragged coalesces to one call per animation frame, latest wins", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return rackFixture([slotFixture()]);
    });
    setParamPlainDragged(0, 11, 100);
    setParamPlainDragged(0, 11, 500);
    setParamPlainDragged(0, 11, 999);
    flushPendingPlainDrags();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual([{ slot: 0, id: 11, value: 999 }]);
  });

  it("tracks separate slots/params independently, and independently of normalized drags", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push({ cmd, args });
      return rackFixture([slotFixture()]);
    });
    setParamPlainDragged(0, 11, 200);
    setParamNormalized(0, 11, 0.4);
    flushPendingDrags();
    flushPendingPlainDrags();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toContainEqual({ cmd: "param_set_plain", args: { slot: 0, id: 11, value: 200 } });
    expect(calls).toContainEqual({
      cmd: "param_set_normalized",
      args: { slot: 0, id: 11, value: 0.4 },
    });
  });
});

describe("live events", () => {
  it("param_changed updates only the affected value in place", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "rack_list_modules") return [];
        if (cmd === "rack_get") {
          return rackFixture([
            slotFixture({
              params: [paramInfoDto({ id: 0 }), paramInfoDto({ id: 1, key: "mix" })],
              values: [
                { id: 0, value: 0, normalized: 0.5, text: "0.0 dB" },
                { id: 1, value: 0, normalized: 0.5, text: "0.0 dB" },
              ],
            }),
          ]);
        }
        throw new Error(`unmocked: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    const untouchedValue = rackState().state.slots[0]!.values[1];
    const payload: ParamChangedDto = { slot: 0, id: 0, value: 6, normalized: 1, text: "6.0 dB" };
    await emit("param_changed", payload);
    flushSync();
    const slot = rackState().state.slots[0]!;
    expect(slot.values[0]).toEqual({ id: 0, value: 6, normalized: 1, text: "6.0 dB" });
    // The untouched value keeps its object identity (an open text field elsewhere stays put).
    expect(slot.values[1]).toBe(untouchedValue);
    teardown();
  });

  it("param_changed for an out-of-range slot is ignored", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "rack_list_modules") return [];
        if (cmd === "rack_get") return rackFixture([slotFixture()]);
        throw new Error(`unmocked: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    await emit("param_changed", { slot: 5, id: 0, value: 1, normalized: 1, text: "x" } satisfies ParamChangedDto);
    flushSync();
    expect(rackState().state.slots).toHaveLength(1);
    teardown();
  });

  it("rack_changed replaces the whole state (a restart's new schema, H-01 handoff)", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "rack_list_modules") return [];
        if (cmd === "rack_get") return rackFixture([slotFixture()]);
        throw new Error(`unmocked: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    const next = rackFixture([slotFixture({ uid: 9 }), slotFixture({ uid: 10 })]);
    await emit("rack_changed", next);
    flushSync();
    expect(rackState().state.slots.map((s) => s.uid)).toEqual([9, 10]);
    teardown();
  });

  it("rack_latency updates only the latency reading", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "rack_list_modules") return [];
        if (cmd === "rack_get") return rackFixture([slotFixture()], false, 64);
        throw new Error(`unmocked: ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    const teardown = await loadRack();
    const payload: RackLatencyDto = { latency_samples: 512 };
    await emit("rack_latency", payload);
    flushSync();
    expect(rackState().state.latency_samples).toBe(512);
    expect(rackState().state.slots).toHaveLength(1);
    teardown();
  });
});
