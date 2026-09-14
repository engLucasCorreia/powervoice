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

    expect(savedArgs).toEqual({ path: "/home/user/out.wav", bits: "16" });
    expect(documentState().saveAsPrompt).toBeNull();

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
