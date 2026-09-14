import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { documentState, openSaveAsPrompt, resetDocumentStateForTest } from "./document.svelte";
import SaveAsDialog from "./SaveAsDialog.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
});

describe("SaveAsDialog (ticket: Save As with bit-depth choice)", () => {
  it("is hidden with no pending prompt", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="save-as-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("picking a bit depth then confirming shows the native dialog and saves at that depth", async () => {
    openSaveAsPrompt();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();

    const radios = target.querySelectorAll<HTMLInputElement>('input[name="save-as-bits"]');
    expect(radios.length).toBe(3);
    const bit16 = [...radios].find((r) => r.value === "16")!;
    bit16.click();
    flushSync();

    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "document_save_as") {
        savedArgs = args;
        return {
          name: "out.wav",
          path: "/home/user/out.wav",
          sample_rate_hz: 48_000,
          len_samples: 0,
          dirty: false,
          audio_rev: 1,
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="save-as-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(savedArgs).toEqual({
      path: "/home/user/out.wav",
      container: "wav",
      bits: "16",
      dither: "tpdf",
      confirmClip: false,
      confirmMultichannel: false,
    });
    expect(documentState().saveAsPrompt).toBeNull();

    unmount(app);
    target.remove();
  });

  it("picking FLAC narrows the bit-depth choices to 16/24 and reaches the saver", async () => {
    openSaveAsPrompt();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();

    const formats = target.querySelectorAll<HTMLInputElement>('input[name="save-as-format"]');
    expect(formats.length).toBe(2);
    const flac = [...formats].find((r) => r.value === "flac")!;
    flac.click();
    flushSync();

    const bitRadios = target.querySelectorAll<HTMLInputElement>('input[name="save-as-bits"]');
    expect([...bitRadios].map((r) => r.value)).toEqual(["16", "24"]);

    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.flac";
      }
      if (cmd === "document_save_as") {
        savedArgs = args;
        return {
          name: "out.flac",
          path: "/home/user/out.flac",
          sample_rate_hz: 48_000,
          len_samples: 0,
          dirty: false,
          audio_rev: 1,
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="save-as-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect(savedArgs).toEqual({
      path: "/home/user/out.flac",
      container: "flac",
      bits: "24",
      dither: "tpdf",
      confirmClip: false,
      confirmMultichannel: false,
    });

    unmount(app);
    target.remove();
  });

  it("H-20 (SPEC-005 §2.7): shows the Dither row for integer targets, TPDF preselected, and sends the chosen mode", async () => {
    openSaveAsPrompt();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();

    const ditherRadios = target.querySelectorAll<HTMLInputElement>('input[name="save-as-dither"]');
    expect([...ditherRadios].map((r) => r.value)).toEqual(["tpdf", "none"]);
    expect(ditherRadios[0]!.checked).toBe(true);

    const none = [...ditherRadios].find((r) => r.value === "none")!;
    none.click();
    flushSync();

    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "document_save_as") {
        savedArgs = args;
        return {
          name: "out.wav",
          path: "/home/user/out.wav",
          sample_rate_hz: 48_000,
          len_samples: 0,
          dirty: false,
          audio_rev: 1,
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="save-as-choose"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    expect((savedArgs as { dither: string }).dither).toBe("none");

    unmount(app);
    target.remove();
  });

  it("H-20 (SPEC-005 §2.7): hides the Dither row for a 32-bit float target (never dithers)", () => {
    openSaveAsPrompt();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();

    const bit32 = [
      ...target.querySelectorAll<HTMLInputElement>('input[name="save-as-bits"]'),
    ].find((r) => r.value === "32f")!;
    bit32.click();
    flushSync();

    expect(target.querySelectorAll('input[name="save-as-dither"]').length).toBe(0);

    unmount(app);
    target.remove();
  });

  it("Cancel clears the prompt without saving", () => {
    openSaveAsPrompt();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SaveAsDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="save-as-cancel"]')!.click();
    flushSync();
    expect(documentState().saveAsPrompt).toBeNull();
    expect(target.querySelector('[data-testid="save-as-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
