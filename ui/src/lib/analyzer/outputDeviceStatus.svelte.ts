/**
 * Output device status for the analyzer panel's "No output device" state (SPEC-007 §2.9): a
 * small dedicated store — no other feature needs a reactive device-status subscription yet, so
 * this doesn't try to be a general devices store (the Audio Devices dialog fetches its own
 * `devices_list` snapshot on demand instead).
 */
import { listen } from "@tauri-apps/api/event";
import type { DeviceStatusDto, DevicesDto, EventName } from "../ipc/bindings";
import { devicesList } from "../ipc/commands";

let status = $state<DeviceStatusDto | null>(null);
let deviceName = $state<string | null>(null);

/**
 * The current output device status (`null` before the first fetch resolves) and, since H-59, the
 * applied output device's name — SPEC-001 §2.1's transport-bar device-status control names the
 * device it is reporting on.
 */
export function outputDeviceStatus(): {
  readonly current: DeviceStatusDto | null;
  readonly name: string | null;
} {
  return {
    get current() {
      return status;
    },
    get name() {
      return deviceName;
    },
  };
}

/**
 * H-59: the store has two consumers now (the analyzer panel's "No output device" state and the
 * transport bar's device-status control), which mount and unmount independently — so the fetch
 * and the `devices_changed` listener are shared, ref-counted, and torn down when the last
 * consumer goes away.
 */
let subscribers = 0;
let shared: Promise<() => void> | null = null;

/** Fetches the current status and follows `devices_changed`. Returns a teardown function. */
export async function initOutputDeviceStatus(): Promise<() => void> {
  subscribers += 1;
  shared ??= subscribe();
  const stopShared = shared;
  let released = false;
  await stopShared;
  return () => {
    if (released) {
      return;
    }
    released = true;
    subscribers -= 1;
    if (subscribers === 0) {
      shared = null;
      void stopShared.then((stop) => stop());
    }
  };
}

async function subscribe(): Promise<() => void> {
  try {
    const view = await devicesList();
    status = view.output_status;
    deviceName = view.output_device ?? view.prefs.output_device;
  } catch {
    status = null;
    deviceName = null;
  }
  let unlisten: (() => void) | null = null;
  try {
    unlisten = await listen<DevicesDto>("devices_changed" satisfies EventName, (event) => {
      status = event.payload.output_status;
      deviceName = event.payload.output_device ?? event.payload.prefs.output_device;
    });
  } catch {
    // stays at whatever the initial fetch produced.
  }
  return () => {
    try {
      // H-59: `unlisten` hands back a promise that rejects when the Tauri event plugin is gone
      // (a test's `clearMocks`, or teardown during shutdown). Swallow it, or it surfaces as an
      // unhandled rejection long after the caller stopped caring.
      void Promise.resolve(unlisten?.() as unknown).catch(() => {});
    } catch {
      // harmless during teardown
    }
  };
}

/** Test/teardown helper. */
export function resetOutputDeviceStatusForTest(): void {
  status = null;
  deviceName = null;
  subscribers = 0;
  shared = null;
}
