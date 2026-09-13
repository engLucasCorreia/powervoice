import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  documentState,
  initDocument,
  resetDocumentStateForTest,
  resolveUnsavedPrompt,
} from "../document/document.svelte";
import type { DocumentDto, RecordStateDto, TransportStateDto } from "../ipc/bindings";
import { VXTM_FLAGS, type TelemetryFrame } from "../ipc/telemetry";
import { attachKeymap, clearActionHandlers } from "../keymap";
import { initRecord, onInputTelemetry, resetRecordForTest } from "../state/record.svelte";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { PeakBallistics } from "./ballistics";
import { formatElapsed } from "./format";
import RecordControls from "./RecordControls.svelte";

let calls: Array<{ cmd: string; args: unknown }> = [];
let docLen = 0;
let recording = false;
let inputDevice: string | null = "Mic";

function recDto(): RecordStateDto {
  return {
    input_device: inputDevice,
    input_channel: 1,
    input_status: inputDevice ? "healthy" : "not_selected",
    armed: recording,
    input_open: recording,
    input_rate_hz: 48_000,
    recording,
    finishing: false,
    monitor: "off",
    monitoring: false,
  };
}

function transportDto(): TransportStateDto {
  return {
    playing: false,
    playhead_samples: 0,
    play_start_samples: 0,
    doc_len_samples: docLen,
    doc_rate_hz: 48_000,
    can_play: docLen > 0,
  };
}

function docDto(len_samples: number, dirty: boolean): DocumentDto {
  return { name: "take.wav", path: "/tmp/take.wav", sample_rate_hz: 48_000, len_samples, dirty, audio_rev: 1 };
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
  docLen = 0;
  recording = false;
  inputDevice = "Mic";
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "record_get":
        case "record_arm":
        case "record_set_monitor":
          return recDto();
        case "record_start":
          recording = true;
          return recDto();
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

  it("peak bar: instant attack, 20 dB/s release, 1.5 s hold", () => {
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

  it("New Recording replaces a document with audio after the unsaved-changes prompt", async () => {
    const stopDocument = await initDocument();
    const { el, teardown } = await setup();
    // A saved document with audio is replaced without asking.
    await emit("document_changed", docDto(96_000, false));
    await settle();
    el("record-button").click();
    await settle();
    expect(recordCalls()).toEqual([{ cmd: "record_start", args: { replace: true } }]);
    el("record-button").click();
    await settle();
    // A modified one asks first: Cancel keeps it, Don't Save replaces it.
    await emit("document_changed", docDto(96_000, true));
    await settle();
    calls = [];
    el("record-button").click();
    await settle();
    expect(documentState().unsavedPrompt).not.toBeNull();
    expect(recordCalls()).toEqual([]);
    resolveUnsavedPrompt("cancel");
    await settle();
    expect(recordCalls()).toEqual([]);
    el("record-button").click();
    await settle();
    resolveUnsavedPrompt("discard");
    await settle();
    expect(recordCalls()).toEqual([{ cmd: "record_start", args: { replace: true } }]);
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

  it("the elapsed time follows the take position while recording", async () => {
    const { el, teardown } = await setup();
    onInputTelemetry(frame(VXTM_FLAGS.RECORDING, 48_000 * 2.5), 0);
    flushSync();
    expect(el("record-elapsed").textContent).toBe("0:00:02.5");
    teardown();
  });
});
