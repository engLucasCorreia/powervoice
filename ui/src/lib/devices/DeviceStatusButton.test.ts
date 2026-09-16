import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetOutputDeviceStatusForTest } from "../analyzer/outputDeviceStatus.svelte";
import type { DevicesDto } from "../ipc/bindings";
import DeviceStatusButton from "./DeviceStatusButton.svelte";

function devicesDto(overrides: Partial<DevicesDto> = {}): DevicesDto {
  return {
    hosts: ["alsa"],
    host: "alsa",
    devices: [],
    default_input: null,
    default_output: "Scarlett 2i2",
    prefs: {
      host: "alsa",
      input_device: null,
      input_channel: 1,
      output_device: "Scarlett 2i2",
      sample_rate_hz: 48_000,
      buffer_size_frames: null,
    },
    output_device: "Scarlett 2i2",
    output_rate_hz: 48_000,
    output_buffer_frames: null,
    output_status: "healthy",
    input_device: null,
    input_status: "not_selected",
    ...overrides,
  };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

afterEach(() => {
  clearMocks();
  resetOutputDeviceStatusForTest();
});

describe("DeviceStatusButton (SPEC-001 §2.1 transport-bar device status)", () => {
  it("names the output device and follows its status through devices_changed", async () => {
    mockIPC((cmd) => (cmd === "devices_list" ? devicesDto() : null), { shouldMockEvents: true });

    let opened = 0;
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DeviceStatusButton, { target, props: { onopen: () => (opened += 1) } });
    await settle();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="open-audio-devices"]');
    expect(button?.textContent).toContain("Scarlett 2i2");
    expect(button?.getAttribute("data-status")).toBe("healthy");

    // SPEC-001 §2.3: losing the device turns the lamp red without reopening the dialog.
    await emit("devices_changed", devicesDto({ output_status: "lost" }));
    await settle();
    expect(
      target.querySelector('[data-testid="open-audio-devices"]')?.getAttribute("data-status"),
    ).toBe("lost");

    // Clicking it opens Settings -> Audio Devices.
    target.querySelector<HTMLButtonElement>('[data-testid="open-audio-devices"]')?.click();
    expect(opened).toBe(1);

    unmount(app);
    target.remove();
  });

  it("falls back to a named action with no output device selected", async () => {
    mockIPC((cmd) =>
      cmd === "devices_list"
        ? devicesDto({
            output_device: null,
            output_status: "not_selected",
            prefs: { ...devicesDto().prefs, output_device: null },
          })
        : null,
    );

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DeviceStatusButton, { target, props: { onopen: () => {} } });
    await settle();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="open-audio-devices"]');
    expect(button?.textContent?.trim()).toBe("No output device");
    expect(button?.getAttribute("data-status")).toBe("not_selected");

    unmount(app);
    target.remove();
  });
});
