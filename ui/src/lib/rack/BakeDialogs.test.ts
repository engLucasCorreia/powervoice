import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RackSlotDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import {
  applyBakeJobProgress,
  bakeState,
  resetBakeForTest,
  startBake,
} from "../state/bake.svelte";
import { docDto, paramInfoDto, rackSlotDto, rackStateDto } from "../test/fixtures";
import BakeDialogs from "./BakeDialogs.svelte";
import { loadRack, resetRackForTest } from "./rack.svelte";

async function setUp(slots: RackSlotDto[]): Promise<void> {
  mockIPC((cmd) => {
    switch (cmd) {
      case "document_open":
        return docDto({ len_samples: 96_000 });
      case "rack_list_modules":
        return [];
      case "rack_get":
        return rackStateDto(slots);
      default:
        throw new Error(`unmocked command: ${cmd}`);
    }
  });
  await openDocument("/home/user/take.wav");
  const stop = await loadRack();
  stop();
  clearMocks();
}

function mockBake(): Array<[string, unknown]> {
  const calls: Array<[string, unknown]> = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "edit_bake_start") {
      return { job_id: 3 };
    }
    if (cmd === "edit_bake_cancel") {
      return null;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  return calls;
}

const noiseOnly = (): RackSlotDto =>
  rackSlotDto({
    params: [paramInfoDto({ id: 2, key: "noise_only" })],
    values: [{ id: 2, value: 1, normalized: 1, text: "On" }],
  });

function mountDialogs(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(BakeDialogs, { target });
  flushSync();
  return { target, app };
}

const q = (target: HTMLElement, id: string): HTMLElement | null =>
  target.querySelector<HTMLElement>(`[data-testid="${id}"]`);

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
  resetBakeForTest();
  resetRackForTest();
  resetDocumentStateForTest();
  document.body.innerHTML = "";
});

describe("BakeDialogs (T-602)", () => {
  it("shows SPEC-014's noise-only confirmation; Continue starts the bake", async () => {
    await setUp([noiseOnly()]);
    const calls = mockBake();
    const { target, app } = mountDialogs();
    await startBake();
    flushSync();
    const dialog = q(target, "bake-noise-only-confirm");
    expect(dialog).not.toBeNull();
    expect(dialog!.textContent).toContain("Output noise only");
    expect(calls).toEqual([]);

    q(target, "bake-noise-only-continue")!.click();
    await vi.waitFor(() => expect(bakeState().job?.state).toBe("running"));
    flushSync();
    expect(q(target, "bake-noise-only-confirm")).toBeNull();
    expect(calls).toContainEqual(["edit_bake_start", { startSamples: 0, endSamples: 96_000 }]);
    unmount(app);
  });

  it("Cancel and Escape close the confirmation without baking", async () => {
    await setUp([noiseOnly()]);
    const calls = mockBake();
    const { target, app } = mountDialogs();
    await startBake();
    flushSync();
    q(target, "bake-noise-only-cancel")!.click();
    flushSync();
    expect(q(target, "bake-noise-only-confirm")).toBeNull();

    await startBake();
    flushSync();
    q(target, "bake-noise-only-confirm")!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    flushSync();
    expect(q(target, "bake-noise-only-confirm")).toBeNull();
    expect(calls).toEqual([]);
    unmount(app);
  });

  it("shows progress after 250 ms; Cancel cancels the job and the dialog closes when it ends", async () => {
    await setUp([rackSlotDto()]);
    const calls = mockBake();
    const { target, app } = mountDialogs();
    await startBake();
    flushSync();
    expect(q(target, "bake-progress-dialog")).toBeNull();
    vi.advanceTimersByTime(260);
    flushSync();
    const dialog = q(target, "bake-progress-dialog");
    expect(dialog).not.toBeNull();
    expect(dialog!.textContent).toContain("Baking rack…");

    applyBakeJobProgress({ job_id: 3, kind: "bake", state: "running", fraction: 0.4 });
    flushSync();
    expect((q(target, "bake-progress-bar") as HTMLProgressElement).value).toBeCloseTo(0.4);

    q(target, "bake-progress-cancel")!.click();
    await Promise.resolve();
    expect(calls).toContainEqual(["edit_bake_cancel", { jobId: 3 }]);

    applyBakeJobProgress({ job_id: 3, kind: "bake", state: "cancelled", fraction: 0 });
    flushSync();
    expect(q(target, "bake-progress-dialog")).toBeNull();
    expect(bakeState().job).toBeNull();
    unmount(app);
  });

  it("a bake that finishes within 250 ms never shows the progress dialog", async () => {
    await setUp([rackSlotDto()]);
    mockBake();
    const { target, app } = mountDialogs();
    await startBake();
    flushSync();
    applyBakeJobProgress({ job_id: 3, kind: "bake", state: "done", fraction: 1 });
    flushSync();
    vi.advanceTimersByTime(500);
    flushSync();
    expect(q(target, "bake-progress-dialog")).toBeNull();
    expect(bakeState().job).toBeNull();
    unmount(app);
  });
});
