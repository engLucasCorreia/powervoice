import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { JobProgressDto, RackSlotDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { loadRack, resetRackForTest } from "../rack/rack.svelte";
import { docDto, paramInfoDto, rackSlotDto, rackStateDto } from "../test/fixtures";
import {
  applyBakeJobProgress,
  bakeState,
  canBake,
  cancelBakeConfirm,
  cancelBakeJob,
  continueBakeConfirm,
  dismissBakeJob,
  rackIsActive,
  resetBakeForTest,
  startBake,
} from "./bake.svelte";
import { clearNotices, noticesState } from "./notices.svelte";
import { applyRecordStateForTest, resetRecordForTest } from "./record.svelte";
import { resetSelectionForTest, setSelectionFromResult } from "./selection.svelte";

async function openDoc(): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return docDto({ len_samples: 480_000 });
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  await openDocument("/home/user/take.wav");
  clearMocks();
}

async function setRack(slots: RackSlotDto[]): Promise<void> {
  mockIPC((cmd) => {
    switch (cmd) {
      case "rack_list_modules":
        return [];
      case "rack_get":
        return rackStateDto(slots);
      default:
        throw new Error(`unmocked command: ${cmd}`);
    }
  });
  const stop = await loadRack();
  stop();
  clearMocks();
}

/** A Noise Reduction slot with "Output noise only" on (SPEC-014 §2.6). */
function noiseOnlySlot(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  return rackSlotDto({
    uid: 2,
    module: "org.powervoice.noise-reduction@1.0.0",
    module_id: "org.powervoice.noise-reduction",
    name: "Noise Reduction",
    params: [paramInfoDto({ id: 2, key: "noise_only" })],
    values: [{ id: 2, value: 1, normalized: 1, text: "On" }],
    ...overrides,
  });
}

/** Mocks `edit_bake_start`/`edit_bake_cancel`, recording their arguments. */
function mockBake(jobId = 7, beforeReturn?: () => void): Array<[string, unknown]> {
  const calls: Array<[string, unknown]> = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "edit_bake_start") {
      beforeReturn?.();
      return { job_id: jobId };
    }
    if (cmd === "edit_bake_cancel") {
      return null;
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  return calls;
}

function progress(overrides: Partial<JobProgressDto> = {}): JobProgressDto {
  return { job_id: 7, kind: "bake", state: "running", fraction: 0, ...overrides };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetBakeForTest();
  resetRackForTest();
  resetRecordForTest();
  resetSelectionForTest();
  resetDocumentStateForTest();
});

describe("rackIsActive", () => {
  it("needs at least one slot that isn't bypassed", () => {
    expect(rackIsActive([])).toBe(false);
    expect(rackIsActive([rackSlotDto({ bypass: true })])).toBe(false);
    expect(rackIsActive([rackSlotDto({ bypass: true }), rackSlotDto({ uid: 2 })])).toBe(true);
  });
});

describe("canBake (Effects → Bake Rack's enabled state)", () => {
  it("is false without a document, even with an active rack", async () => {
    await setRack([rackSlotDto()]);
    expect(canBake()).toBe(false);
  });

  it("is false with an empty or fully bypassed rack", async () => {
    await openDoc();
    await setRack([]);
    expect(canBake()).toBe(false);
    await setRack([rackSlotDto({ bypass: true })]);
    expect(canBake()).toBe(false);
  });

  it("is true with a document and an active rack, false while recording", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    expect(canBake()).toBe(true);
    applyRecordStateForTest({ recording: true });
    expect(canBake()).toBe(false);
  });

  it("is false while a bake runs", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    mockBake();
    await startBake();
    expect(bakeState().job?.state).toBe("running");
    expect(canBake()).toBe(false);
  });
});

describe("startBake", () => {
  it("bakes the whole file when nothing is selected", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    const calls = mockBake();
    await startBake();
    expect(calls).toContainEqual(["edit_bake_start", { startSamples: 0, endSamples: 480_000 }]);
    expect(bakeState().job).toEqual({ jobId: 7, fraction: 0, state: "running" });
  });

  it("bakes the selection when there is one", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    setSelectionFromResult([100, 2_000]);
    const calls = mockBake();
    await startBake();
    expect(calls).toContainEqual(["edit_bake_start", { startSamples: 100, endSamples: 2_000 }]);
  });

  it("follows job_progress for its own job only, then clears on dismiss", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    mockBake();
    await startBake();
    applyBakeJobProgress(progress({ fraction: 0.5 }));
    expect(bakeState().job?.fraction).toBe(0.5);
    applyBakeJobProgress(progress({ kind: "export", fraction: 0.9 }));
    applyBakeJobProgress(progress({ job_id: 99, fraction: 0.9 }));
    expect(bakeState().job?.fraction).toBe(0.5);
    applyBakeJobProgress(progress({ state: "done", fraction: 1 }));
    expect(bakeState().job?.state).toBe("done");
    dismissBakeJob();
    expect(bakeState().job).toBeNull();
  });

  it("keeps a terminal event that arrives before the job id does", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    mockBake(7, () => applyBakeJobProgress(progress({ state: "done", fraction: 1 })));
    await startBake();
    expect(bakeState().job?.state).toBe("done");
  });

  it("reports a refused start as a notice and runs no job", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    mockIPC(() => {
      throw { code: "busy", key: "error.document_busy", params: {} };
    });
    await startBake();
    expect(bakeState().job).toBeNull();
    expect(JSON.stringify(noticesState().toasts)).toContain("error.document_busy");
  });

  it("Cancel asks the backend to cancel the running job", async () => {
    await openDoc();
    await setRack([rackSlotDto()]);
    const calls = mockBake(12);
    await startBake();
    cancelBakeJob();
    await Promise.resolve();
    expect(calls).toContainEqual(["edit_bake_cancel", { jobId: 12 }]);
  });
});

describe("Output noise only confirmation (SPEC-014 §2.6)", () => {
  it("asks first; Cancel bakes nothing, Continue bakes the confirmed scope", async () => {
    await openDoc();
    await setRack([rackSlotDto(), noiseOnlySlot()]);
    const calls = mockBake();
    await startBake();
    expect(bakeState().confirmOpen).toBe(true);
    expect(calls).toEqual([]);
    cancelBakeConfirm();
    expect(bakeState().confirmOpen).toBe(false);
    expect(calls).toEqual([]);

    await startBake();
    await continueBakeConfirm();
    expect(bakeState().confirmOpen).toBe(false);
    expect(calls).toContainEqual(["edit_bake_start", { startSamples: 0, endSamples: 480_000 }]);
  });

  it("doesn't ask when the noise-only slot is bypassed", async () => {
    await openDoc();
    await setRack([rackSlotDto(), noiseOnlySlot({ bypass: true })]);
    const calls = mockBake();
    await startBake();
    expect(bakeState().confirmOpen).toBe(false);
    expect(calls.map(([cmd]) => cmd)).toContain("edit_bake_start");
  });
});
