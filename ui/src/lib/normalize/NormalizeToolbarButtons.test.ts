import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, EditResultDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import NormalizeToolbarButtons from "./NormalizeToolbarButtons.svelte";

function doc(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    ...overrides,
  };
}

async function openFixture(): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return doc();
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetNormalizeForTest();
  resetRecordForTest();
});

function mountButtons(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(NormalizeToolbarButtons, { target });
  flushSync();
  return { target, app };
}

describe("NormalizeToolbarButtons (S2-02, SPEC-010 §2.5/AC-14)", () => {
  it("is disabled with no document open", () => {
    const { target, app } = mountButtons();
    for (const id of [
      "toolbar-normalize-1-0db",
      "toolbar-normalize-0-1db",
      "toolbar-normalize-3-0db",
    ]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        true,
      );
    }
    unmount(app);
    target.remove();
  });

  it("each button sends exactly one edit_normalize_peak with its target, whole file scope", async () => {
    await openFixture();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak") {
        calls.push(args);
        return {
          changed: true,
          audio_rev: 2,
          len_samples: 480_000,
          selection: null,
          playhead_samples: 0,
        } satisfies EditResultDto;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountButtons();

    target.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-3-0db"]')?.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(calls).toEqual([{ startSamples: 0, endSamples: 480_000, targetDb: -3 }]);

    unmount(app);
    target.remove();
  });
});
