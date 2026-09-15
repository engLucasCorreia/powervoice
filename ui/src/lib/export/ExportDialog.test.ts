import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RackSlotDto, RackStateDto } from "../ipc/bindings";
import { loadRack, resetRackForTest } from "../rack/rack.svelte";
import { clearNotices } from "../state/notices.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { paramInfoDto, rackSlotDto, rackStateDto } from "../test/fixtures";
import ExportDialog from "./ExportDialog.svelte";
import { exportState, openExportDialog, resetExportStateForTest } from "./export.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetExportStateForTest();
  resetRackForTest();
  resetSelectionForTest();
});

/** A live noise-reduction slot with "Output noise only" on (SPEC-014 §2.6), for the
 * confirmation-dialog test. */
function noiseOnlySlot(): RackSlotDto {
  const param = paramInfoDto({
    id: 2,
    key: "noise_only",
    name: { text: "Output noise only", key: null },
    min: 0,
    max: 1,
    step: 1,
    decimals: 0,
    smoothing_ms: 0,
    flags: {
      automatable: true,
      stepped: true,
      boolean: true,
      read_only: false,
      hidden: false,
      bypass: false,
    },
  });
  return rackSlotDto({
    module: "org.powervoice.noise-reduction@1.0.0",
    module_id: "org.powervoice.noise-reduction",
    name: "Noise Reduction",
    params: [param],
    values: [{ id: 2, value: 1, normalized: 1, text: "On" }],
    noise_profile: "loaded",
  });
}

async function seedRack(slots: RackSlotDto[]): Promise<void> {
  const state: RackStateDto = rackStateDto(slots);
  mockIPC((cmd) => {
    if (cmd === "rack_list_modules") {
      return [];
    }
    if (cmd === "rack_get") {
      return state;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await loadRack();
  clearMocks();
}

describe("ExportDialog (ticket S4-04)", () => {
  it("is hidden with no pending prompt or job", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="export-dialog"]')).toBeNull();
    expect(target.querySelector('[data-testid="export-progress"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows WAV/FLAC/MP3 format choices, disabling MP3 when unavailable", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: false };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    const radios = target.querySelectorAll<HTMLInputElement>('input[name="export-kind"]');
    expect(radios.length).toBe(3);
    const mp3Radio = [...radios].find((r) => r.value === "mp3")!;
    expect(mp3Radio.disabled).toBe(true);
    const acxButton = target.querySelector<HTMLButtonElement>('[data-testid="export-acx"]')!;
    expect(acxButton.disabled).toBe(true);

    unmount(app);
    target.remove();
  });

  it("picking FLAC then confirming shows the native dialog and starts the export at that format", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    const flacRadio = target.querySelector<HTMLInputElement>('input[name="export-kind"][value="flac"]')!;
    flacRadio.click();
    flushSync();

    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.flac";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(startArgs).toMatchObject({
      request: { path: "/home/user/out.flac", format: { kind: "flac", bits: "24" } },
    });
    expect(exportState().prompt).toBeNull();
    expect(exportState().job?.jobId).toBe(1);

    unmount(app);
    target.remove();
  });

  it("the ACX preset button sets MP3 CBR 192 kbps / 44.1 kHz", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="export-acx"]')!.click();
    flushSync();

    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/acx.mp3";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 2 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(startArgs).toMatchObject({
      request: {
        format: { kind: "mp3", settings: { kind: "cbr", kbps: 192 } },
        sample_rate_hz: 44_100,
      },
    });

    unmount(app);
    target.remove();
  });

  it("Cancel clears the prompt without starting an export", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="export-cancel"]')!.click();
    flushSync();
    expect(exportState().prompt).toBeNull();
    expect(target.querySelector('[data-testid="export-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Selection is disabled with no selection, and enabled + sent as range with one (H-08)", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    const selectionRadio = target.querySelector<HTMLInputElement>(
      'input[name="export-range"][value="selection"]',
    )!;
    expect(selectionRadio.disabled).toBe(true);

    setSelectionFromResult([1_000, 5_000]);
    flushSync();
    expect(selectionRadio.disabled).toBe(false);
    selectionRadio.click();
    flushSync();

    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/selection.wav";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(startArgs).toMatchObject({
      request: { range: { start_sample: 1_000, end_sample: 5_000 } },
    });

    unmount(app);
    target.remove();
  });

  it("picking MP3 VBR sends the chosen quality to export_start", async () => {
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    target.querySelector<HTMLInputElement>('input[name="export-kind"][value="mp3"]')!.click();
    flushSync();
    target.querySelector<HTMLInputElement>('input[name="export-mp3-mode"][value="vbr"]')!.click();
    flushSync();
    const qualitySelect = target.querySelector<HTMLSelectElement>('[data-testid="export-vbr-quality"]')!;
    qualitySelect.value = "0";
    qualitySelect.dispatchEvent(new Event("change"));
    flushSync();

    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.mp3";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 6 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(startArgs).toMatchObject({
      request: { format: { kind: "mp3", settings: { kind: "vbr", quality: 0 } } },
    });

    unmount(app);
    target.remove();
  });

  it("shows the Output noise only confirmation, and Export anyway proceeds (SPEC-014 §2.6)", async () => {
    await seedRack([noiseOnlySlot()]);
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(target.querySelector('[data-testid="export-noise-only-confirm"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="export-dialog"]')).toBeNull();

    let startArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/noise-only.wav";
      }
      if (cmd === "export_start") {
        startArgs = args;
        return { job_id: 8 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="export-noise-only-continue"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(target.querySelector('[data-testid="export-noise-only-confirm"]')).toBeNull();
    expect(startArgs).toMatchObject({ request: { path: "/home/user/noise-only.wav" } });

    unmount(app);
    target.remove();
  });

  it("Cancel on the Output noise only confirmation starts no job", async () => {
    await seedRack([noiseOnlySlot()]);
    mockIPC((cmd) => {
      if (cmd === "export_formats") {
        return { mp3_available: true };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    openExportDialog("take");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ExportDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="export-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="export-noise-only-cancel"]')!.click();
    flushSync();

    expect(target.querySelector('[data-testid="export-noise-only-confirm"]')).toBeNull();
    expect(exportState().job).toBeNull();

    unmount(app);
    target.remove();
  });
});
