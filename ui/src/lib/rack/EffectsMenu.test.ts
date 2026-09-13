import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, it } from "vitest";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import EffectsMenu from "./EffectsMenu.svelte";
import { resetNrCaptureForTest } from "./nrCapture.svelte";
import { resetRackForTest } from "./rack.svelte";

/** Effects menu/toolbar (S3-06, SPEC-014 §2.3): "Capture Noise Print", enabled per §2.3. */

afterEach(() => {
  clearMocks();
  resetSelectionForTest();
  resetRecordForTest();
  resetRackForTest();
  resetNrCaptureForTest();
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
