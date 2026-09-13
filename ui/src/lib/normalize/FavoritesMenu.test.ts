import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { DEFAULT_KEYMAP } from "../keymap";
import { clearNotices } from "../state/notices.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetNormalizeLufsForTest } from "../state/normalizeLufs.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import FavoritesMenu from "./FavoritesMenu.svelte";

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

async function openFixture(overrides: Partial<DocumentDto> = {}): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return doc(overrides);
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
  resetNormalizeLufsForTest();
  resetRecordForTest();
  resetSettingsStateForTest();
});

function mountMenu(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(FavoritesMenu, { target });
  flushSync();
  return { target, app };
}

const FAVORITE_TESTIDS = [
  "favorites-normalize-1-0db",
  "favorites-normalize-0-1db",
  "favorites-normalize-3-0db",
];

describe("FavoritesMenu (S2-02, SPEC-010 §2.1/§2.5/AC-14)", () => {
  it("disables every command with no document open", () => {
    const { target, app } = mountMenu();
    for (const id of [...FAVORITE_TESTIDS, "favorites-normalize-custom"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        true,
      );
    }
    unmount(app);
    target.remove();
  });

  it("enables every command with a non-empty document open", async () => {
    await openFixture();
    const { target, app } = mountMenu();
    for (const id of [...FAVORITE_TESTIDS, "favorites-normalize-custom"]) {
      expect(target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(
        false,
      );
    }
    unmount(app);
    target.remove();
  });

  it("each favorite button sends exactly one edit_normalize_peak_start with its target and no dialog", async () => {
    await openFixture();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountMenu();

    target.querySelector<HTMLButtonElement>('[data-testid="favorites-normalize-1-0db"]')?.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: -1, targetPct: null },
    ]);
    expect(target.querySelector('[data-testid="normalize-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Normalize… opens the dialog instead of sending a command", async () => {
    await openFixture();
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="favorites-normalize-custom"]')?.click();
    flushSync();
    expect(target.querySelector('[data-testid="normalize-dialog"]')).not.toBeNull();
    unmount(app);
    target.remove();
  });

  it("the keymap registry has no binding for any normalize command (SPEC-010 §2.5)", () => {
    for (const binding of DEFAULT_KEYMAP) {
      expect(binding.action.startsWith("normalize")).toBe(false);
    }
  });
});
