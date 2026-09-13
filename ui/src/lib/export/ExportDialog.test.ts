import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import ExportDialog from "./ExportDialog.svelte";
import { exportState, openExportDialog, resetExportStateForTest } from "./export.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetExportStateForTest();
});

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
});
