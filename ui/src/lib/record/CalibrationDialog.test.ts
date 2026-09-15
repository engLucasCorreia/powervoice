import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { CalibrationResultDto, RecordOffsetDto } from "../ipc/bindings";
import { clearActionHandlers } from "../shortcuts";
import { initRecord, openCalibration, recordState, resetRecordForTest } from "../state/record.svelte";
import { recordStateDto } from "../test/fixtures";
import CalibrationDialog from "./CalibrationDialog.svelte";

let calls: Array<{ cmd: string; args: unknown }> = [];

const REC = recordStateDto();

function offsetDto(source: RecordOffsetDto["source"], ms: number): RecordOffsetDto {
  return {
    available: true,
    host: "pipewire",
    input_device: "Mic",
    output_device: "DAC",
    device_rate_hz: 48_000,
    offset_ms: ms,
    source,
    updated_unix_ms: source ? 1_789_000_000_000 : null,
    confidence: source === "calibrated" ? 1 : null,
    buffer_frames: null,
    current_buffer_frames: null,
  };
}

function result(patch: Partial<CalibrationResultDto>): CalibrationResultDto {
  return {
    job_id: 5,
    verify: false,
    offset_ms: 3,
    offset_samples: 144,
    device_rate_hz: 48_000,
    confidence: 1,
    reps_agreeing: 5,
    psr_median: 60,
    peak_dbfs: -32.1,
    clipped: false,
    weak: false,
    accepted: true,
    reason: null,
    ...patch,
  };
}

beforeEach(() => {
  calls = [];
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "record_get":
          return REC;
        case "record_offset_get":
          return offsetDto(null, 0);
        case "calibration_run":
          return 5;
        case "record_offset_set": {
          const a = args as { offsetMs: number };
          return offsetDto("calibrated", a.offsetMs);
        }
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetRecordForTest();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

const el = (id: string): HTMLElement => {
  const found = document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  if (!found) {
    throw new Error(`missing ${id}`);
  }
  return found;
};

const cmds = () => calls.filter((c) => c.cmd.startsWith("calibration_") || c.cmd === "record_offset_set");

describe("calibration dialog (T-304, SPEC-022 §2.14)", () => {
  it("AC-17/AC-18: a rejected run can't be applied; an accepted one is stored, then Verify is offered", async () => {
    const stop = initRecord();
    await settle();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(CalibrationDialog, { target });
    try {
      openCalibration();
      await settle();
      expect(el("calibration-dialog").getAttribute("aria-modal")).toBe("true");
      el("calibration-start").click();
      await settle();
      expect(cmds()).toEqual([{ cmd: "calibration_run", args: { verify: false } }]);
      await emit("job_progress", { job_id: 5, kind: "calibration", state: "running", fraction: 0.5 });
      await settle();
      expect((el("calibration-progress") as HTMLProgressElement).value).toBe(0.5);

      // Rejected: 2 of 5 agree — Apply is disabled and nothing is stored.
      await emit(
        "calibration_result",
        result({ accepted: false, confidence: 0.4, reps_agreeing: 2, reason: "low_confidence" }),
      );
      await settle();
      expect(el("calibration-rejected").textContent).toContain("2 of 5");
      expect((el("calibration-apply") as HTMLButtonElement).disabled).toBe(true);

      // Retry → accepted (+3.00 ms, 144 samples at 48 kHz) → Apply stores it.
      el("calibration-retry").click();
      await settle();
      await emit("calibration_result", result({}));
      await settle();
      expect(el("calibration-accepted").textContent).toContain("+3.00 ms (144 samples at 48 kHz)");
      el("calibration-apply").click();
      await settle();
      expect(cmds().at(-1)).toEqual({
        cmd: "record_offset_set",
        args: { offsetMs: 3, source: "calibrated", confidence: 1 },
      });
      expect(recordState().offset?.source).toBe("calibrated");
      el("calibration-verify").click();
      await settle();
      expect(cmds().at(-1)).toEqual({ cmd: "calibration_run", args: { verify: true } });
      await emit("calibration_result", result({ verify: true, offset_ms: 0.02, offset_samples: 1 }));
      await settle();
      expect(el("calibration-residual").textContent).toContain("0.02 ms");
    } finally {
      unmount(app);
      stop();
    }
  });
});
