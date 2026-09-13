import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { resetNormalizeForTest } from "../state/normalize.svelte";
import { resetNormalizeLufsForTest } from "../state/normalizeLufs.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import EffectsMenu from "./EffectsMenu.svelte";
import { resetNrCaptureForTest } from "./nrCapture.svelte";
import { resetRackForTest } from "./rack.svelte";

/** Effects menu/toolbar (S3-06, SPEC-014 §2.3: "Capture Noise Print"; H-09, SPEC-010 §2.5:
 * "Normalize…"/"Normalize (LUFS)…"). */

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
  resetDocumentStateForTest();
  resetSelectionForTest();
  resetRecordForTest();
  resetRackForTest();
  resetNrCaptureForTest();
  resetNormalizeForTest();
  resetNormalizeLufsForTest();
  resetSettingsStateForTest();
  document.body.innerHTML = "";
});

function render() {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(EffectsMenu, { target });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

it("is disabled without a selection", () => {
  const { target, teardown } = render();
  const button = target.querySelector<HTMLButtonElement>('[data-testid="menu-capture-noise-print"]')!;
  expect(button.disabled).toBe(true);
  teardown();
});

it("is enabled with a selection and starts a capture with no hint (last-focused slot)", async () => {
  setSelectionFromResult([0, 24_000]);
  let sentArgs: unknown;
  mockIPC((cmd, args) => {
    if (cmd === "nr_capture_start") {
      sentArgs = args;
      return { job_id: 1, slot: 0 };
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  const { target, teardown } = render();
  const button = target.querySelector<HTMLButtonElement>('[data-testid="menu-capture-noise-print"]')!;
  expect(button.disabled).toBe(false);
  button.click();
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(sentArgs).toEqual({ hintSlot: null, start: 0, end: 24_000 });
  teardown();
});

it("Normalize…/Normalize (LUFS)… are disabled with no document open", () => {
  const { target, teardown } = render();
  expect(
    target.querySelector<HTMLButtonElement>('[data-testid="menu-normalize-dialog"]')?.disabled,
  ).toBe(true);
  expect(
    target.querySelector<HTMLButtonElement>('[data-testid="menu-normalize-lufs-dialog"]')
      ?.disabled,
  ).toBe(true);
  teardown();
});

it("Normalize… opens the shared normalize dialog (H-09, SPEC-010 §2.5)", async () => {
  await openFixture();
  const { target, teardown } = render();
  const button = target.querySelector<HTMLButtonElement>('[data-testid="menu-normalize-dialog"]')!;
  expect(button.disabled).toBe(false);
  button.click();
  flushSync();
  // The dialog component itself lives in FavoritesMenu.svelte (mounted separately in the real
  // app); here we only assert the shared store's dialogOpen flag flipped.
  const { normalizeState } = await import("../state/normalize.svelte");
  expect(normalizeState().dialogOpen).toBe(true);
  teardown();
});

it("Normalize (LUFS)… opens the shared LUFS normalize dialog", async () => {
  await openFixture();
  const { target, teardown } = render();
  const button = target.querySelector<HTMLButtonElement>(
    '[data-testid="menu-normalize-lufs-dialog"]',
  )!;
  expect(button.disabled).toBe(false);
  button.click();
  flushSync();
  const { normalizeLufsState } = await import("../state/normalizeLufs.svelte");
  expect(normalizeLufsState().dialogOpen).toBe(true);
  teardown();
});
