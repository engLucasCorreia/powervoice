import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { docDto } from "../test/fixtures";
import ClipPromptDialog from "./ClipPromptDialog.svelte";
import { documentState, resetDocumentStateForTest, saveDocument } from "./document.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
});

/** Triggers `dialog.overs` from `document_save` (mirrors `document.test.ts`'s clip-prompt
 * flows) so this file can focus on the dialog's own rendering/interaction. */
function triggerOvers(count: number, peakDbfs: number, onConfirmClip: () => unknown): Promise<boolean> {
  mockIPC((cmd, args) => {
    if (cmd === "document_save") {
      const confirmClip = (args as { confirmClip: boolean }).confirmClip;
      if (!confirmClip) {
        throw {
          code: "needs_confirmation",
          key: "dialog.overs",
          params: { count: String(count), peak_dbfs: String(peakDbfs) },
        };
      }
      return onConfirmClip();
    }
    if (cmd === "document_save_as") {
      return onConfirmClip();
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  return saveDocument();
}

describe("ClipPromptDialog (SPEC-005 §2.8)", () => {
  it("is hidden with no pending prompt", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ClipPromptDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="clip-prompt-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows the count and peak, and 'Clip and save' proceeds", async () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ClipPromptDialog, { target });
    flushSync();

    const pending = triggerOvers(12, 1.8, () => docDto({ audio_rev: 2 }));
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    expect(target.querySelector('[data-testid="clip-prompt-dialog"]')).not.toBeNull();
    const message = target.querySelector("p")?.textContent ?? "";
    expect(message).toContain("12");
    expect(message).toContain("+1.8");

    target.querySelector<HTMLButtonElement>('[data-testid="clip-prompt-clip"]')!.click();
    expect(await pending).toBe(true);
    flushSync();
    expect(target.querySelector('[data-testid="clip-prompt-dialog"]')).toBeNull();
    expect(documentState().current.audio_rev).toBe(2);

    unmount(app);
    target.remove();
  });

  it("'Save as 32-bit float instead' reaches document_save_as at 32f", async () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ClipPromptDialog, { target });
    flushSync();

    let floatArgs: unknown;
    const pending = triggerOvers(1, -0.5, () => {
      floatArgs = "unused";
      return docDto({ audio_rev: 3 });
    });
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="clip-prompt-float"]')!.click();
    expect(await pending).toBe(true);
    expect(floatArgs).toBe("unused");

    unmount(app);
    target.remove();
  });

  it("Cancel closes the dialog and leaves the document untouched", async () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ClipPromptDialog, { target });
    flushSync();

    const pending = triggerOvers(1, 0.1, () => {
      throw new Error("must not be called after Cancel");
    });
    await new Promise((r) => setTimeout(r, 0));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="clip-prompt-cancel"]')!.click();
    expect(await pending).toBe(false);
    flushSync();
    expect(target.querySelector('[data-testid="clip-prompt-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
