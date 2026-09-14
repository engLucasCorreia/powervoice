import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "./document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { applyRecordStateForTest, recordState, resetRecordForTest } from "../state/record.svelte";
import DocumentMenu from "./DocumentMenu.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetRecordForTest();
});

describe("DocumentMenu (S1-03)", () => {
  it("shows 'no document' and disables Save/Save As with none open", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DocumentMenu, { target });
    flushSync();

    expect(target.querySelector('[data-testid="document-name"]')?.textContent).toBe(
      "No file open",
    );
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-save"]')?.disabled,
    ).toBe(true);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-save-as"]')?.disabled,
    ).toBe(true);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-export"]')?.disabled,
    ).toBe(true);

    unmount(app);
    target.remove();
  });

  it("shows the document name (with a modified marker) and enables Save/Save As once open", async () => {
    const fixture: DocumentDto = {
      name: "take.wav",
      path: "/home/user/take.wav",
      sample_rate_hz: 48_000,
      len_samples: 480_000,
      dirty: true,
      audio_rev: 1, sidecar_dirty: false, spectral_view: null, waveform_view: null,
    };
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DocumentMenu, { target });
    flushSync();

    expect(target.querySelector('[data-testid="document-name"]')?.textContent).toBe(
      "take.wav *",
    );
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-save"]')?.disabled,
    ).toBe(false);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-export"]')?.disabled,
    ).toBe(false);

    unmount(app);
    target.remove();
  });

  it("New Recording… opens the format prompt (H-06), disabled while recording", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DocumentMenu, { target });
    flushSync();

    expect(recordState().newRecordingPrompt).toBeNull();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-new-recording"]')!.click();
    flushSync();
    expect(recordState().newRecordingPrompt).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("New Recording… is disabled while a take is recording or finishing", () => {
    applyRecordStateForTest({ recording: true });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(DocumentMenu, { target });
    flushSync();
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="menu-new-recording"]')?.disabled,
    ).toBe(true);
    unmount(app);
    target.remove();
  });
});
