import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { documentState, initDocument, resetDocumentStateForTest } from "../document/document.svelte";
import type { DocumentDto } from "../ipc/bindings";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import {
  docDto as docFixture,
  recordStateDto,
  transportStateDto,
} from "../test/fixtures";
import { attachKeymap, clearActionHandlers } from "../shortcuts";
import {
  initRecord,
  onInputTelemetry,
  recordState,
  resetRecordForTest,
  resolveLowDiskPrompt,
} from "../state/record.svelte";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { PeakBallistics } from "../meters/ballistics";
import {
  DISK_WARN_MINUTES,
  formatElapsed,
  formatLatencyMs,
  formatRemaining,
  monitorLatencyLevel,
} from "./format";
import RecordControls from "./RecordControls.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import {
  beginDrag,
  dragTo,
  resetSelectionForTest,
  selectionState,
  setSelectionFromResult,
} from "../state/selection.svelte";

/** T-304: what the mocked `record_start_at` resolves Record to. */
let startedOp: "insert" | "punch" = "insert";
let calls: Array<{ cmd: string; args: unknown }> = [];
let docLen = 0;
let recording = false;
let inputDevice: string | null = "Mic";
let failRecordStart = false;
let dropoutCount = 0;
let diskRemainingS: number | null = null;
let monitorLatencyUs: number | null = null;

function recDto(): ReturnType<typeof recordStateDto> {
  return recordStateDto({
    input_device: inputDevice,
    input_status: inputDevice ? "healthy" : "not_selected",
    armed: recording,
    input_open: recording,
    recording,
    monitor_latency_us: monitorLatencyUs,
    dropout_count: dropoutCount,
    disk_remaining_s: diskRemainingS,
  });
}

function transportDto(): ReturnType<typeof transportStateDto> {
  return transportStateDto({ doc_len_samples: docLen, can_play: docLen > 0 });
}

function docDto(len_samples: number, dirty: boolean): DocumentDto {
  return docFixture({ path: "/tmp/take.wav", len_samples, dirty });
}

function frame(flags: number, playheadSample = 0): TelemetryFrame {
  return {
    seq: 0,
    flags,
    playheadSample,
    playheadTimeNs: 0,
    rate: 0,
    outPeakDbfs: Number.NEGATIVE_INFINITY,
    outRmsDbfs: Number.NEGATIVE_INFINITY,
    inPeakDbfs: -12,
    inRmsDbfs: -20,
    audioRev: 0,
    droppedRtEvents: 0,
  };
}

beforeEach(() => {
  calls = [];
  startedOp = "insert";
  docLen = 0;
  recording = false;
  inputDevice = "Mic";
  failRecordStart = false;
  dropoutCount = 0;
  diskRemainingS = null;
  monitorLatencyUs = null;
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "record_get":
        case "record_arm":
        case "record_set_monitor":
          return recDto();
        case "record_start":
          if (failRecordStart) {
            throw { code: "device_lost", key: "error.record.input_unavailable", params: {} };
          }
          recording = true;
          return recDto();
        case "record_start_at":
          recording = true;
          return {
            take_id: 7,
            op: startedOp,
            at_samples: 240_000,
            end_samples: startedOp === "punch" ? 384_000 : null,
            preroll_samples: startedOp === "punch" ? 96_000 : 0,
            postroll_samples: startedOp === "punch" ? 48_000 : 0,
            aligned: startedOp === "punch",
            state: recDto(),
          };
        case "record_stop":
          recording = false;
          return recDto();
        case "transport_get":
          return transportDto();
        case "clock_now_ns":
          return 0;
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
  resetTransportForTest();
  resetRecordForTest();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
  resetSelectionForTest();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function setup(): Promise<{ el: (id: string) => HTMLElement; teardown: () => void }> {
  const stopTransport = await initTransport();
  const stopRecord = initRecord();
  await settle();
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(RecordControls, { target });
  flushSync();
  const el = (id: string): HTMLElement => {
    const found = document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    if (!found) {
      throw new Error(`missing ${id}`);
    }
    return found;
  };
  return {
    el,
    teardown: () => {
      unmount(app);
      stopRecord();
      stopTransport();
    },
  };
}

const recordCalls = () => calls.filter((c) => c.cmd.startsWith("record_") && c.cmd !== "record_get");

describe("record panel (S1-04)", () => {
  it("formats the elapsed time as h:mm:ss.t", () => {
    expect(formatElapsed(0, 48_000)).toBe("0:00:00.0");
    expect(formatElapsed(Math.round(3723.45 * 48_000), 48_000)).toBe("1:02:03.4");
    expect(formatElapsed(100, 0)).toBe("0:00:00.0");
  });

  it("peak bar: instant attack, 20 dB/s release, 1.5 s hold (ballistics maths: meters/ballistics.test.ts)", () => {
    const b = new PeakBallistics();
    b.update(-6, 1000);
    expect(b.bar).toBe(-6);
    expect(b.hold).toBe(-6);
    b.update(-60, 1500);
    expect(b.bar).toBeCloseTo(-16, 6);
    expect(b.hold).toBe(-6);
    b.update(-60, 2500);
    expect(b.hold).toBe(-6);
    b.update(-60, 3000);
    expect(b.hold).toBeLessThan(-6);
    expect(b.hold).toBeGreaterThanOrEqual(b.bar);
    b.update(-3, 3100);
    expect(b.bar).toBe(-3);
    expect(b.hold).toBe(-3);
  });

  it("Record starts a new recording into an empty document and toggles to Stop", async () => {
    const { el, teardown } = await setup();
    el("record-button").click();
    await settle();
    expect(recordCalls()).toEqual([{ cmd: "record_start", args: { replace: false } }]);
    expect(el("record-button").textContent?.trim()).toBe("Stop");
    expect((el("record-arm") as HTMLButtonElement).disabled).toBe(true);
    el("record-button").click();
    await settle();
    expect(recordCalls().at(-1)?.cmd).toBe("record_stop");
    teardown();
  });

  it("T-304 (SPEC-022 §2.2, AC-1): Record on a document with audio records at the cursor or punches the selection", async () => {
    const stopDocument = await initDocument();
    const { el, teardown } = await setup();
    // A document with audio no longer replaces the document: no dialog, no unsaved-changes ask.
    await emit("document_changed", docDto(96_000, true));
    await settle();
    el("record-button").click();
    await settle();
    expect(documentState().unsavedPrompt).toBeNull();
    expect(recordState().newRecordingPrompt).toBeNull();
    expect(recordCalls()).toEqual([{ cmd: "record_start_at", args: { selection: null } }]);
    expect(recordState().op?.op).toBe("insert");
    el("record-button").click(); // Stop
    await settle();
    await emit("record_finished", {
      take_id: 7,
      op: "insert",
      committed: true,
      cancel_reason: null,
      result: { changed: true, audio_rev: 2, len_samples: 144_000, selection: null, playhead_samples: 72_000 },
    });
    await settle();
    expect(recordState().op).toBeNull();

    // A non-empty selection is sent along; the backend resolves it to a punch-in.
    setSelectionFromResult([24_000, 48_000]);
    startedOp = "punch";
    calls = [];
    el("record-button").click();
    await settle();
    expect(recordCalls()).toEqual([{ cmd: "record_start_at", args: { selection: [24_000, 48_000] } }]);
    // SPEC-022 §2.11: selection gestures are ignored while the operation runs.
    beginDrag(10);
    dragTo(500);
    expect(selectionState().current).toEqual({ startSample: 24_000, endSample: 48_000 });
    teardown();
    stopDocument();
  });

  it("T-304 (SPEC-022 §2.11, AC-8): the panel counts the pre-roll down, then shows the punch progress", async () => {
    const stopDocument = await initDocument();
    const { el, teardown } = await setup();
    await emit("document_changed", docDto(960_000, false));
    await settle();
    startedOp = "punch";
    el("record-button").click();
    await settle();
    await emit("record_phase", { take_id: 7, op: "punch", phase: "preroll", doc_pos_samples: 144_000, app_ns: 0 });
    onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 144_000));
    await settle();
    expect(el("record-phase").textContent?.trim()).toBe("Pre-roll 2.0 s");
    onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 235_200));
    await settle();
    expect(el("record-phase").textContent?.trim()).toBe("Pre-roll 0.1 s");
    await emit("record_phase", { take_id: 7, op: "punch", phase: "recording", doc_pos_samples: 240_000, app_ns: 0 });
    onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 302_400));
    await settle();
    expect(el("record-phase").textContent?.trim()).toBe("Punch-in 0:01.3 / 0:03.0");
    await emit("record_phase", { take_id: 7, op: "punch", phase: "postroll", doc_pos_samples: 384_000, app_ns: 0 });
    await settle();
    expect(el("record-phase").textContent?.trim()).toBe("Post-roll");
    await emit("record_finished", {
      take_id: 7,
      op: "punch",
      committed: true,
      cancel_reason: null,
      result: { changed: true, audio_rev: 2, len_samples: 960_000, selection: [240_000, 384_000], playhead_samples: 240_000 },
    });
    await settle();
    expect(document.querySelector('[data-testid="record-phase"]')).toBeNull();
    expect(selectionState().current).toEqual({ startSample: 240_000, endSample: 384_000 });
    teardown();
    stopDocument();
  });

  it("Arm and Record are disabled without an input device", async () => {
    inputDevice = null;
    const { el, teardown } = await setup();
    expect((el("record-button") as HTMLButtonElement).disabled).toBe(true);
    expect((el("record-arm") as HTMLButtonElement).disabled).toBe(true);
    expect(el("record-button").title).toContain("Audio Devices");
    teardown();
  });

  it("Shift+R toggles the recording", async () => {
    const { teardown } = await setup();
    const detach = attachKeymap(window, { isMac: false });
    const press = async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyR", shiftKey: true, bubbles: true }));
      await settle();
    };
    await press();
    await press();
    expect(recordCalls().map((c) => c.cmd)).toEqual(["record_start", "record_stop"]);
    detach();
    teardown();
  });

  it("the clip lamp latches until clicked or a new recording starts", async () => {
    const { el, teardown } = await setup();
    const lit = () => el("record-clip").classList.contains("lit");
    onInputTelemetry(frame(VXTM_FLAGS.IN_CLIP), 0);
    flushSync();
    expect(lit()).toBe(true);
    onInputTelemetry(frame(0), 16);
    flushSync();
    expect(lit()).toBe(true);
    el("record-clip").click();
    flushSync();
    expect(lit()).toBe(false);
    onInputTelemetry(frame(VXTM_FLAGS.IN_CLIP), 32);
    flushSync();
    expect(lit()).toBe(true);
    el("record-button").click();
    await settle();
    expect(lit()).toBe(false);
    teardown();
  });

  it("H-10 item 2: a failed Record attempt keeps a real clip lamp lit (SPEC-002 AC-2)", async () => {
    const { el, teardown } = await setup();
    const lit = () => el("record-clip").classList.contains("lit");
    onInputTelemetry(frame(VXTM_FLAGS.IN_CLIP), 0);
    flushSync();
    expect(lit()).toBe(true);

    failRecordStart = true;
    el("record-button").click();
    await settle();
    expect(recordCalls().map((c) => c.cmd)).toEqual(["record_start"]);
    // The take never started, so a lamp that was genuinely lit must stay lit.
    expect(lit()).toBe(true);

    failRecordStart = false;
    el("record-button").click();
    await settle();
    // Once the take actually starts, the lamp clears (SPEC-002 §2.1/AC-2).
    expect(lit()).toBe(false);
    teardown();
  });

  it("H-10 item 4: shows the live dropout counter only once a dropout occurred", async () => {
    const { el, teardown } = await setup();
    recording = true;
    dropoutCount = 0;
    await emit("record_state", recDto());
    await settle();
    expect(document.querySelector('[data-testid="record-dropouts"]')).toBeNull();

    dropoutCount = 2;
    await emit("record_state", recDto());
    await settle();
    expect(el("record-dropouts").textContent?.trim()).toBe("2 dropouts");

    dropoutCount = 0;
    recording = false;
    await emit("record_state", recDto());
    await settle();
    expect(document.querySelector('[data-testid="record-dropouts"]')).toBeNull();
    teardown();
  });

  it("the elapsed time follows the take position while recording", async () => {
    const { el, teardown } = await setup();
    onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 48_000 * 2.5), 0);
    flushSync();
    expect(el("record-elapsed").textContent).toBe("0:00:02.5");
    teardown();
  });

  it("formats remaining disk time as h:mm:ss / m:ss (H-11)", () => {
    expect(formatRemaining(59)).toBe("0:59");
    expect(formatRemaining(600)).toBe("10:00");
    expect(formatRemaining(3_661)).toBe("1:01:01");
    expect(DISK_WARN_MINUTES).toBe(10);
  });

  it("H-11: shows the remaining disk time, amber below the warn threshold", async () => {
    const { el, teardown } = await setup();
    expect(document.querySelector('[data-testid="record-disk-remaining"]')).toBeNull();

    diskRemainingS = 20 * 60;
    await emit("record_state", recDto());
    await settle();
    expect(el("record-disk-remaining").textContent?.trim()).toBe("20:00 left");
    expect(el("record-disk-remaining").classList.contains("low")).toBe(false);

    diskRemainingS = 8 * 60;
    await emit("record_state", recDto());
    await settle();
    expect(el("record-disk-remaining").classList.contains("low")).toBe(true);
    teardown();
  });

  it("H-11 (SPEC-002 §2.5): Record below the low-disk threshold confirms first", async () => {
    diskRemainingS = 8 * 60;
    const { el, teardown } = await setup();
    el("record-button").click();
    await settle();
    // Blocked on the confirm prompt: `record_start` hasn't run yet.
    expect(recordCalls()).toEqual([]);
    expect(recordState().lowDiskPrompt?.minutes).toBe(8);

    resolveLowDiskPrompt(false);
    await settle();
    expect(recordCalls()).toEqual([]);
    expect(recordState().lowDiskPrompt).toBeNull();

    el("record-button").click();
    await settle();
    resolveLowDiskPrompt(true);
    await settle();
    expect(recordCalls()).toEqual([{ cmd: "record_start", args: { replace: false } }]);
    teardown();
  });

  it("T-107 (SPEC-002 AC-12): latency warnings — none below 20 ms, amber from 20 ms, red from 40 ms", () => {
    expect(monitorLatencyLevel(19.9)).toBe("ok");
    expect(monitorLatencyLevel(20.0)).toBe("amber");
    expect(monitorLatencyLevel(39.9)).toBe("amber");
    expect(monitorLatencyLevel(40.0)).toBe("red");
    expect(formatLatencyMs(23_700)).toBe("23.7");
  });

  it("T-107: shows the monitoring latency readout with its warning, and offers Through rack", async () => {
    const { el, teardown } = await setup();
    expect(document.querySelector('[data-testid="record-monitor-latency"]')).toBeNull();
    const cases: Array<[number, "ok" | "amber" | "red"]> = [
      [19_900, "ok"],
      [20_000, "amber"],
      [39_900, "amber"],
      [40_000, "red"],
    ];
    for (const [us, level] of cases) {
      monitorLatencyUs = us;
      await emit("record_state", recDto());
      await settle();
      const readout = el("record-monitor-latency");
      expect(readout.textContent?.trim()).toBe(`Monitoring latency ${(us / 1000).toFixed(1)} ms`);
      expect(readout.classList.contains("amber")).toBe(level === "amber");
      expect(readout.classList.contains("red")).toBe(level === "red");
      if (level === "red") {
        expect(readout.title).toContain("smaller buffer size");
      } else if (level === "amber") {
        expect(readout.title).toBe("You may hear your voice delayed.");
      }
    }
    const select = el("record-monitor") as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => o.value)).toEqual(["off", "dry", "through_rack"]);
    select.value = "through_rack";
    // Svelte 5 delegates `change` to the root: the event must bubble.
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(recordCalls().at(-1)).toEqual({ cmd: "record_set_monitor", args: { mode: "through_rack" } });
    teardown();
  });
});
