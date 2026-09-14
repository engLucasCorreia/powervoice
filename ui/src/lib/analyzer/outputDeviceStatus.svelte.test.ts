import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DevicesDto } from "../ipc/bindings";
import {
  initOutputDeviceStatus,
  outputDeviceStatus,
  resetOutputDeviceStatusForTest,
} from "./outputDeviceStatus.svelte";

function devicesDto(overrides: Partial<DevicesDto> = {}): DevicesDto {
  return {
    hosts: ["pipewire"],
    host: "pipewire",
    devices: [],
    default_input: null,
    default_output: null,
    prefs: {
      host: "pipewire",
      input_device: null,
      input_channel: 1,
      output_device: null,
      sample_rate_hz: null,
      buffer_size_frames: null,
    },
    output_device: null,
    output_rate_hz: null,
    output_buffer_frames: null,
    output_status: "healthy",
    input_device: null,
    input_status: "not_selected",
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  resetOutputDeviceStatusForTest();
});

/** H-16 (SPEC-007 §2.9): the analyzer's "No output device" state reflects the real device
 * status, not just "no VXSA frame arrived yet". */
describe("outputDeviceStatus", () => {
  it("fetches the initial status from devices_list", async () => {
    mockIPC((cmd) => {
      if (cmd === "devices_list") {
        return devicesDto({ output_status: "not_selected" });
      }
      return null;
    }, { shouldMockEvents: true });

    const stop = await initOutputDeviceStatus();
    expect(outputDeviceStatus().current).toBe("not_selected");
    stop();
  });

  it("follows devices_changed for a device loss/recovery while mounted", async () => {
    mockIPC((cmd) => {
      if (cmd === "devices_list") {
        return devicesDto({ output_status: "healthy" });
      }
      return null;
    }, { shouldMockEvents: true });

    const stop = await initOutputDeviceStatus();
    expect(outputDeviceStatus().current).toBe("healthy");

    await emit("devices_changed", devicesDto({ output_status: "lost" }));
    expect(outputDeviceStatus().current).toBe("lost");

    await emit("devices_changed", devicesDto({ output_status: "fallback" }));
    expect(outputDeviceStatus().current).toBe("fallback");

    stop();
  });

  it("a failed devices_list fetch leaves the status null rather than throwing", async () => {
    mockIPC(() => {
      throw new Error("boom");
    });
    const stop = await initOutputDeviceStatus();
    expect(outputDeviceStatus().current).toBeNull();
    stop();
  });
});
