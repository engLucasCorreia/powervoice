import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { NoiseProfileStatusDto } from "../ipc/bindings";
import { resetSelectionForTest, setSelectionFromResult } from "../state/selection.svelte";
import { resetRecordForTest } from "../state/record.svelte";
import { resetNrCaptureForTest } from "./nrCapture.svelte";
import NoiseReductionSection from "./NoiseReductionSection.svelte";
import { resetRackForTest } from "./rack.svelte";

/**
 * NR slot panel status line + Capture button (S3-06, SPEC-014 §2.8 item 1, AC-21 status/button
 * part).
 */

afterEach(() => {
  clearMocks();
  resetSelectionForTest();
  resetRecordForTest();
  resetRackForTest();
  resetNrCaptureForTest();
  document.body.innerHTML = "";
});

function render(status: NoiseProfileStatusDto, slotIndex = 0) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(NoiseReductionSection, { target, props: { slotIndex, status } });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

describe("status line", () => {
  it.each([
    ["none", "No noise print"],
    ["loaded", "Noise print loaded"],
    ["unreadable", "unreadable"],
    ["too_new", "newer PowerVoice"],
  ] as const)("shows the %s status text", (status, expectedSubstring) => {
    const { target, teardown } = render(status);
    expect(target.querySelector('[data-testid="nr-capture-status"]')?.textContent).toContain(
      expectedSubstring,
    );
    teardown();
  });
});

describe("Capture button", () => {
  it("is disabled with a tooltip when there is no selection", () => {
    const { target, teardown } = render("none");
    const button = target.querySelector<HTMLButtonElement>('[data-testid="nr-capture-button"]')!;
    expect(button.disabled).toBe(true);
    expect(button.title).toBe("Select some room tone first");
    teardown();
  });

  it("is enabled with a non-empty selection", () => {
    setSelectionFromResult([1_000, 5_000]);
    const { target, teardown } = render("none");
    const button = target.querySelector<HTMLButtonElement>('[data-testid="nr-capture-button"]')!;
    expect(button.disabled).toBe(false);
    expect(button.title).toBe("Shift+P");
    teardown();
  });

  it("clicking sends this slot's own index as the hint plus the current selection", async () => {
    setSelectionFromResult([2_000, 30_000]);
    let sentArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "nr_capture_start") {
        sentArgs = args;
        return { job_id: 5, slot: 3 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("none", 3);
    const button = target.querySelector<HTMLButtonElement>('[data-testid="nr-capture-button"]')!;
    button.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sentArgs).toEqual({ hintSlot: 3, start: 2_000, end: 30_000 });
    teardown();
  });

  it("shows a spinner and Cancel while a job runs for this slot", async () => {
    setSelectionFromResult([0, 24_000]);
    mockIPC((cmd) => {
      if (cmd === "nr_capture_start") {
        return { job_id: 1, slot: 0 };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const { target, teardown } = render("none", 0);
    target.querySelector<HTMLButtonElement>('[data-testid="nr-capture-button"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    expect(target.querySelector('[data-testid="nr-capture-spinner"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="nr-capture-cancel"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="nr-capture-button"]')).toBeNull();
    expect(target.querySelector('[data-testid="nr-capture-status"]')?.textContent).toContain(
      "Capturing",
    );

    let cancelledId: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "nr_capture_cancel") {
        cancelledId = (args as { jobId: number }).jobId;
        return null;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    target.querySelector<HTMLButtonElement>('[data-testid="nr-capture-cancel"]')!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelledId).toBe(1);
    teardown();
  });
});
