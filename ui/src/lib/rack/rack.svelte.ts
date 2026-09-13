import { listen } from "@tauri-apps/api/event";
import type {
  EventName,
  IpcError,
  ModuleDescriptorDto,
  ParamChangedDto,
  RackLatencyDto,
  RackStateDto,
} from "../ipc/bindings";
import {
  paramSetNormalized,
  paramSetText,
  rackAb,
  rackAdd,
  rackBypass,
  rackGet,
  rackListModules,
  rackMove,
  rackRemove,
  rackRestart,
} from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";

/**
 * Rack store (S3-01, SPEC-012 §2.1–§2.6): the rack panel's slot list and the module registry,
 * kept in sync with the engine through `rack_get`/mutating commands and the `rack_changed` /
 * `param_changed` / `rack_latency` events. Parameter drags are coalesced here to at most one
 * `param_set_normalized` call per animation frame (§2.4 "no lost values": the latest value
 * always wins).
 */

const EMPTY: RackStateDto = { slots: [], ab: false, latency_samples: 0 };

let state = $state<RackStateDto>({ ...EMPTY });
let modules = $state<ModuleDescriptorDto[]>([]);
let loading = $state(true);
/** True once a command has failed with `error.rack_unavailable` (no output device open yet). */
let unavailable = $state(false);

/** Read-only accessor for components. */
export function rackState(): {
  readonly state: RackStateDto;
  readonly modules: ModuleDescriptorDto[];
  readonly loading: boolean;
  readonly unavailable: boolean;
} {
  return {
    get state() {
      return state;
    },
    get modules() {
      return modules;
    },
    get loading() {
      return loading;
    },
    get unavailable() {
      return unavailable;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (!isIpcError(err)) {
    return;
  }
  if (err.key === "error.rack_unavailable") {
    unavailable = true;
  }
  pushNotice(noticeFromIpcError(err));
}

function applyState(next: RackStateDto): void {
  state = next;
  unavailable = false;
}

/** Merges one `param_changed` echo in place, without replacing slot/param object identity where
 * unaffected — keeps an open text field elsewhere in the panel untouched. */
function applyParamChanged(msg: ParamChangedDto): void {
  const slot = state.slots[msg.slot];
  if (!slot) {
    return;
  }
  const values = slot.values.map((v) =>
    v.id === msg.id
      ? { id: msg.id, value: msg.value, normalized: msg.normalized, text: msg.text }
      : v,
  );
  const slots = state.slots.slice();
  slots[msg.slot] = { ...slot, values };
  state = { ...state, slots };
  unavailable = false;
}

async function run(command: () => Promise<RackStateDto>): Promise<void> {
  try {
    applyState(await command());
  } catch (err) {
    report(err);
  }
}

export const addModule = (moduleId: string, index: number): Promise<void> =>
  run(() => rackAdd(moduleId, index));
export const removeSlot = (index: number): Promise<void> => run(() => rackRemove(index));
export const moveSlot = (from: number, to: number): Promise<void> => run(() => rackMove(from, to));
export const setBypass = (index: number, on: boolean): Promise<void> =>
  run(() => rackBypass(index, on));
export const setAb = (on: boolean): Promise<void> => run(() => rackAb(on));
export const restartSlot = (index: number): Promise<void> => run(() => rackRestart(index));

/**
 * Sets a parameter from typed text (Rust parses it, SPEC-012 §2.6); returns whether it was
 * accepted. `notifyOnError: false` (the inline text-entry field) shows its own error outline
 * instead of a toast — see `ParamControl.svelte`.
 */
export async function setParamText(
  slot: number,
  id: number,
  text: string,
  opts: { notifyOnError?: boolean } = {},
): Promise<boolean> {
  try {
    applyState(await paramSetText(slot, id, text));
    return true;
  } catch (err) {
    if (opts.notifyOnError !== false) {
      report(err);
    }
    return false;
  }
}

// --- Parameter drags: at most one IPC call per animation frame, latest wins (SPEC-012 §2.4) ---

const requestFrame: (cb: () => void) => number =
  typeof requestAnimationFrame === "function"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => setTimeout(cb, 16) as unknown as number;
const cancelFrame: (id: number) => void =
  typeof cancelAnimationFrame === "function"
    ? (id) => cancelAnimationFrame(id)
    : (id) => clearTimeout(id);

interface PendingDrag {
  value: number;
  frame: number;
}

const pendingDrags = new Map<string, PendingDrag>();

function dragKey(slot: number, id: number): string {
  return `${slot}:${id}`;
}

function flushDrag(key: string, slot: number, id: number): void {
  const pending = pendingDrags.get(key);
  if (!pending) {
    return;
  }
  pendingDrags.delete(key);
  paramSetNormalized(slot, id, pending.value)
    .then(applyState)
    .catch(report);
}

/**
 * Slider drag: a normalized `[0, 1]` position. Coalesced to at most one `param_set_normalized`
 * call per animation frame — call this as often as pointer events arrive; only the latest value
 * before each frame is sent.
 */
export function setParamNormalized(slot: number, id: number, value: number): void {
  const key = dragKey(slot, id);
  const existing = pendingDrags.get(key);
  if (existing) {
    existing.value = value;
    return;
  }
  const frame = requestFrame(() => flushDrag(key, slot, id));
  pendingDrags.set(key, { value, frame });
}

/** Test helper: waits for any coalesced drag calls to be sent. */
export function flushPendingDrags(): void {
  for (const [key, pending] of pendingDrags) {
    cancelFrame(pending.frame);
    pendingDrags.delete(key);
    const [slotStr, idStr] = key.split(":");
    paramSetNormalized(Number(slotStr), Number(idStr), pending.value)
      .then(applyState)
      .catch(report);
  }
}

// --- Loading and live updates ---------------------------------------------------------------

/** Loads the registry and the current rack, subscribes to `rack_changed`/`param_changed`/
 * `rack_latency`, and returns the teardown. */
export async function loadRack(): Promise<() => void> {
  loading = true;
  const cleanups: Array<() => void> = [];

  try {
    modules = await rackListModules();
  } catch (err) {
    report(err);
  }
  try {
    applyState(await rackGet());
  } catch (err) {
    report(err);
  } finally {
    loading = false;
  }

  try {
    const unlisten = await listen<RackStateDto>("rack_changed" satisfies EventName, (e) =>
      applyState(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Without events the state still follows command results.
  }
  try {
    const unlisten = await listen<ParamChangedDto>("param_changed" satisfies EventName, (e) =>
      applyParamChanged(e.payload),
    );
    cleanups.push(unlisten);
  } catch {
    // Same fallback.
  }
  try {
    const unlisten = await listen<RackLatencyDto>("rack_latency" satisfies EventName, (e) => {
      state = { ...state, latency_samples: e.payload.latency_samples };
    });
    cleanups.push(unlisten);
  } catch {
    // Same fallback.
  }

  return () => {
    for (const cleanup of cleanups) {
      try {
        cleanup();
      } catch {
        // A failed unlisten during teardown is harmless.
      }
    }
    for (const pending of pendingDrags.values()) {
      cancelFrame(pending.frame);
    }
    pendingDrags.clear();
  };
}

/** Test/teardown helper. */
export function resetRackForTest(): void {
  state = { ...EMPTY };
  modules = [];
  loading = true;
  unavailable = false;
  for (const pending of pendingDrags.values()) {
    cancelFrame(pending.frame);
  }
  pendingDrags.clear();
}
