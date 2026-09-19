import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import { docDto as doc } from "../test/fixtures";
import NormalizeToolbarButtons from "./NormalizeToolbarButtons.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

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
  resetWaveformViewForTest();
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
    // H-26: the favorites live in the shared menu, which only exists while open — with nothing
    // to normalize its trigger is disabled, so it can't open.
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-menu"]')?.disabled,
    ).toBe(true);
    expect(target.querySelector('[data-testid="toolbar-normalize-1-0db"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("opens a Peak / Loudness menu with the six favorites, values with a true minus and a no-break space before the unit", async () => {
    await openFixture();
    const { target, app } = mountButtons();
    target.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-menu"]')!.click();
    flushSync();
    const menu = target.querySelector('[data-testid="toolbar-normalize-popup"]')!;
    expect(menu.getAttribute("role")).toBe("menu");
    const labels = [...menu.querySelectorAll('[role="menuitem"]')].map((i) => i.textContent?.trim());
    expect(labels).toEqual([
      "−1.0\u00a0dBFS",
      "−0.1\u00a0dBFS",
      "−3.0\u00a0dBFS",
      "−16.0\u00a0LUFS",
      "−19.0\u00a0LUFS",
      "−23.0\u00a0LUFS",
    ]);
    unmount(app);
    target.remove();
  });

  it("each button sends exactly one edit_normalize_peak_start with its target, whole file scope", async () => {
    await openFixture();
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, app } = mountButtons();

    target.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-menu"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-3-0db"]')?.click();
    // H-96: `run()` now awaits `ensureListening()` *before* the start command, landing a couple
    // of microtask ticks later than a fixed `await Promise.resolve()` pair assumed.
    await vi.waitFor(() => expect(calls.length).toBeGreaterThan(0));

    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: -3, targetPct: null },
    ]);

    unmount(app);
    target.remove();
  });
});
