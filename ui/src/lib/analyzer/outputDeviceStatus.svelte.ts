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

/** The current output device status (`null` before the first fetch resolves). */
export function outputDeviceStatus(): { readonly current: DeviceStatusDto | null } {
  return {
    get current() {
      return status;
    },
  };
}

/** Fetches the current status and follows `devices_changed`. Returns a teardown function. */
export async function initOutputDeviceStatus(): Promise<() => void> {
  try {
    const view = await devicesList();
    status = view.output_status;
  } catch {
    status = null;
  }
  let unlisten: (() => void) | null = null;
  try {
    unlisten = await listen<DevicesDto>("devices_changed" satisfies EventName, (event) => {
      status = event.payload.output_status;
    });
  } catch {
    // stays at whatever the initial fetch produced.
  }
  return () => {
    try {
      unlisten?.();
    } catch {
      // harmless during teardown
    }
  };
}

/** Test/teardown helper. */
export function resetOutputDeviceStatusForTest(): void {
  status = null;
}
