import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearNotices } from "../state/notices.svelte";
import { openNormalizeDialog, resetNormalizeForTest } from "../state/normalize.svelte";
import { resetSelectionForTest } from "../state/selection.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import NormalizeDialog from "./NormalizeDialog.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

function doc(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    sidecar_dirty: false,
    spectral_view: null, waveform_view: null, recovered: false,
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
  resetWaveformViewForTest();
  resetSelectionForTest();
  resetNormalizeForTest();
  resetSettingsStateForTest();
});

function mountDialog(): { target: HTMLElement; app: object } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(NormalizeDialog, { target });
  flushSync();
  return { target, app };
}

describe("NormalizeDialog (S2-02, SPEC-010 §2.4/AC-14)", () => {
  it("is hidden until opened", () => {
    const { target, app } = mountDialog();
    expect(target.querySelector('[data-testid="normalize-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it.each(["-60.01", "0.01", "abc"])("rejects %s (Apply disabled)", async (bad) => {
    await openFixture();
    openNormalizeDialog();
    const { target, app } = mountDialog();

    const input = target.querySelector<HTMLInputElement>(
      '[data-testid="normalize-dialog-target"]',
    )!;
    input.value = bad;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="normalize-dialog-apply"]')?.disabled,
    ).toBe(true);

    unmount(app);
    target.remove();
  });

  it("Apply sends the typed value and closes the dialog", async () => {
    await openFixture();
    openNormalizeDialog();
    const { target, app } = mountDialog();

    const input = target.querySelector<HTMLInputElement>(
      '[data-testid="normalize-dialog-target"]',
    )!;
    input.value = "-6.02";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "edit_normalize_peak_start") {
        calls.push(args);
        return { job_id: 1 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="normalize-dialog-apply"]')!.click();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();

    expect(calls).toEqual([
      { startSamples: 0, endSamples: 480_000, targetDb: -6.02, targetPct: null },
    ]);
    expect(target.querySelector('[data-testid="normalize-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("the % unit toggle switches the field and converts the value", async () => {
    await openFixture();
    openNormalizeDialog();
    const { target, app } = mountDialog();

    const input = target.querySelector<HTMLInputElement>(
      '[data-testid="normalize-dialog-target"]',
    )!;
    input.value = "-1.00";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="normalize-dialog-unit-pct"]')!.click();
    flushSync();

    const afterToggle = target.querySelector<HTMLInputElement>(
      '[data-testid="normalize-dialog-target"]',
    )!;
    expect(Number(afterToggle.value)).toBeCloseTo(89.1, 1);
    expect(
      target.querySelector<HTMLButtonElement>('[data-testid="normalize-dialog-apply"]')?.disabled,
    ).toBe(false);

    unmount(app);
    target.remove();
  });

  it("Cancel closes without sending a command", async () => {
    await openFixture();
    openNormalizeDialog();
    const { target, app } = mountDialog();

    target.querySelector<HTMLButtonElement>('[data-testid="normalize-dialog-cancel"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="normalize-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Esc closes the dialog", async () => {
    await openFixture();
    openNormalizeDialog();
    const { target, app } = mountDialog();

    const dialog = target.querySelector<HTMLElement>('[data-testid="normalize-dialog"]')!;
    dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="normalize-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
