import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DevicesDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import AudioDevicesDialog from "./AudioDevicesDialog.svelte";

/** A one-output, one-input host with the output currently lost (SPEC-001 §2.3's red state). */
function devicesDto(overrides: Partial<DevicesDto> = {}): DevicesDto {
  return {
    hosts: ["alsa"],
    host: "alsa",
    devices: [
      {
        name: "DAC",
        base_name: "DAC",
        input: false,
        output: true,
        input_channels: 0,
        system_default: true,
        input_is_monitor: false,
        output_rates_hz: [44_100, 48_000],
        output_buffer_sizes: [256, 512],
      },
    ],
    default_input: null,
    default_output: "DAC",
    prefs: {
      host: "alsa",
      input_device: null,
      input_channel: 1,
      output_device: "DAC",
      sample_rate_hz: 48_000,
      buffer_size_frames: null,
    },
    output_device: "DAC",
    output_rate_hz: 48_000,
    output_buffer_frames: null,
    output_status: "lost",
    input_device: null,
    input_status: "not_selected",
    ...overrides,
  };
}

/** Lets the mocked IPC round trips settle, then flushes Svelte's effects. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

/** Mounts the dialog and waits for its `devices_list` round trip. */
async function open(): Promise<{ target: HTMLElement; app: Record<string, unknown> }> {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(AudioDevicesDialog, { target, props: { onclose: () => {} } });
  await settle();
  return { target, app };
}

afterEach(() => {
  clearMocks();
  clearNotices();
});

describe("AudioDevicesDialog (SPEC-001 §2.1)", () => {
  it("rescans on demand and shows the recovered status (H-59)", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "devices_list") {
        return devicesDto();
      }
      if (cmd === "devices_rescan") {
        return devicesDto({ output_status: "healthy" });
      }
      return null;
    });

    const { target, app } = await open();
    expect(target.querySelector('[data-testid="devices-status"]')?.getAttribute("data-status")).toBe(
      "lost",
    );

    const rescan = target.querySelector<HTMLButtonElement>('[data-testid="devices-rescan"]');
    expect(rescan, "SPEC-001 §2.1 requires a Rescan button").not.toBeNull();
    rescan?.click();
    await settle();

    expect(calls).toContain("devices_rescan");
    expect(target.querySelector('[data-testid="devices-status"]')?.getAttribute("data-status")).toBe(
      "healthy",
    );

    unmount(app);
    target.remove();
  });

  it("keeps the dialog usable when a rescan fails", async () => {
    mockIPC((cmd) => {
      if (cmd === "devices_list") {
        return devicesDto();
      }
      if (cmd === "devices_rescan") {
        throw { code: "internal", key: "error.internal", params: {} };
      }
      return null;
    });

    const { target, app } = await open();
    target.querySelector<HTMLButtonElement>('[data-testid="devices-rescan"]')?.click();
    await settle();

    // The failure becomes a notice, and the button is enabled again rather than stuck "busy".
    expect(target.querySelector<HTMLButtonElement>('[data-testid="devices-rescan"]')?.disabled).toBe(
      false,
    );
    expect(target.querySelector('[data-testid="devices-status"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });
});
