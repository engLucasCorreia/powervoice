import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import { openDocument, requestOpen, resetDocumentStateForTest } from "./document.svelte";
import { docDto } from "../test/fixtures";
import UnsavedChangesDialog from "./UnsavedChangesDialog.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
});

describe("UnsavedChangesDialog (SPEC-004 §2.8)", () => {
  it("is hidden with no pending prompt, and Cancel resolves the guard without opening", async () => {
    const dirty = docDto({ dirty: true });
    mockIPC((cmd) => (cmd === "document_open" ? dirty : null));
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(UnsavedChangesDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="unsaved-changes-dialog"]')).toBeNull();

    let openCalled = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|open") {
        openCalled = true;
        return null;
      }
      return null;
    });
    const pending = requestOpen();
    await Promise.resolve();
    flushSync();
    expect(target.querySelector('[data-testid="unsaved-changes-dialog"]')).not.toBeNull();
    expect(target.querySelector("p")?.textContent).toContain("take.wav");

    target.querySelector<HTMLButtonElement>('[data-testid="unsaved-cancel"]')!.click();
    await pending;
    flushSync();
    expect(openCalled).toBe(false);
    expect(target.querySelector('[data-testid="unsaved-changes-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
