import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto, RecordStateDto, SpectroRequestDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearActionHandlers } from "../keymap";
import { initRecord, resetRecordForTest } from "../state/record.svelte";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import SpectralView from "./SpectralView.svelte";

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

function unstubSize(): void {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
}

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetDocumentStateForTest();
  resetRecordForTest();
  resetSpectralForTest();
  unstubSize();
});

const FIXTURE: DocumentDto = {
  name: "take.wav",
  path: "/home/user/take.wav",
  sample_rate_hz: 48_000,
  len_samples: 480_000,
  dirty: false,
  audio_rev: 1,
};

interface SpectroCall {
  viewId: number;
  request: SpectroRequestDto;
}

function setupIpc(spectroRequests: SpectroCall[]): void {
  mockIPC(
    (cmd, args) => {
      if (cmd === "document_open") {
        return FIXTURE;
      }
      if (cmd === "spectro_attach" || cmd === "spectro_detach") {
        return undefined;
      }
      if (cmd === "spectro_request") {
        spectroRequests.push(args as unknown as SpectroCall);
        return undefined;
      }
      if (cmd === "record_get") {
        return {
          input_device: null,
          input_channel: 1,
          input_status: "not_selected",
          armed: false,
          input_open: false,
          input_rate_hz: null,
          recording: false,
          finishing: false,
          monitor: "off",
          monitoring: false,
          monitor_latency_us: null,
          monitor_dropouts: 0,
          dropout_count: 0,
          disk_remaining_s: null,
        } satisfies RecordStateDto;
      }
      return null;
    },
    { shouldMockEvents: true },
  );
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("SpectralView (T-207, SPEC-007 essential subset)", () => {
  it("shows the empty state with no document open", () => {
    setupIpc([]);
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="spectral-empty"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="spectral-canvas"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("renders the canvas, ruler and toolbar, and requests tiles once a document opens", async () => {
    stubSize(800, 200);
    const spectroRequests: SpectroCall[] = [];
    setupIpc(spectroRequests);

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    await settle();
    await settle();

    expect(target.querySelector('[data-testid="spectral-canvas"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="spectral-ruler"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="spectral-toolbar"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="spectral-empty"]')).toBeNull();

    expect(spectroRequests.length).toBeGreaterThan(0);
    const last = spectroRequests.at(-1)!;
    // 48 kHz -> Auto FFT 2048; spp_dev 1 -> hop = max(pow2_floor(1), 2048/16) = 128.
    expect(last.request.fft_size).toBe(2048);
    expect(last.request.hop).toBe(128);
    expect(last.request.window).toBe(0);

    unmount(app);
    target.remove();
  });

  it("shows the recording-frozen overlay and issues no further spectro_request while recording", async () => {
    stubSize(800, 200);
    const spectroRequests: SpectroCall[] = [];
    setupIpc(spectroRequests);

    const stopRecord = initRecord();
    await settle();

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    await settle();
    await settle();
    const countBeforeRecording = spectroRequests.length;
    expect(countBeforeRecording).toBeGreaterThan(0);

    await emit("record_state", {
      input_device: "Mic",
      input_channel: 1,
      input_status: "healthy",
      armed: true,
      input_open: true,
      input_rate_hz: 48_000,
      recording: true,
      finishing: false,
      monitor: "off",
      monitoring: true,
      monitor_latency_us: null,
      monitor_dropouts: 0,
      dropout_count: 0,
      disk_remaining_s: null,
    } satisfies RecordStateDto);
    await settle();

    expect(target.querySelector('[data-testid="spectral-recording-overlay"]')).not.toBeNull();
    expect(spectroRequests.length).toBe(countBeforeRecording);

    stopRecord();
    unmount(app);
    target.remove();
  });

  it("changing colormap, floor, ceiling or frequency scale issues no new spectro_request (AC-8)", async () => {
    stubSize(800, 200);
    const spectroRequests: SpectroCall[] = [];
    setupIpc(spectroRequests);

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    await settle();
    await settle();
    const countBefore = spectroRequests.length;
    expect(countBefore).toBeGreaterThan(0);

    const spectral = spectralState();
    spectral.setColormap("viridis");
    spectral.setFloorDb(-100);
    spectral.setCeilDb(-10);
    spectral.setFreqScale("linear");
    await settle();

    expect(spectroRequests.length).toBe(countBefore);

    unmount(app);
    target.remove();
  });

  it("changing the FFT size requests new tiles at the new size", async () => {
    stubSize(800, 200);
    const spectroRequests: SpectroCall[] = [];
    setupIpc(spectroRequests);

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    await settle();
    await settle();
    const countBefore = spectroRequests.length;

    spectralState().setFftSize(4096);
    await settle();

    expect(spectroRequests.length).toBeGreaterThan(countBefore);
    expect(spectroRequests.at(-1)!.request.fft_size).toBe(4096);

    unmount(app);
    target.remove();
  });
});
