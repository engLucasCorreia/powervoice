import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { SpectroRequestDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { clearActionHandlers } from "../shortcuts";
import { initRecord, resetRecordForTest } from "../state/record.svelte";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import { docDto, recordStateDto } from "../test/fixtures";
import SpectralView from "./SpectralView.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";

describe("SpectralView hover readout during a selection drag (H-121)", () => {
  async function mountOpen(): Promise<{ target: HTMLElement; app: ReturnType<typeof mount>; fire: (type: string, clientX: number) => void }> {
    stubSize(800, 200);
    setupIpc([]);
    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    await settle();
    await settle();
    const container = target.querySelector<HTMLElement>(".canvas-container")!;
    const fire = (type: string, clientX: number) => {
      container.dispatchEvent(new PointerEvent(type, { clientX, clientY: 50, bubbles: true }));
      flushSync();
    };
    return { target, app, fire };
  }

  it("hides the time/Hz/dB readout while a selection is being dragged and shows it again on hover after release", async () => {
    const { target, app, fire } = await mountOpen();
    const readout = () => target.querySelector('[data-testid="spectral-hover"]');

    fire("pointermove", 100);
    expect(readout(), "plain hover shows the readout").not.toBeNull();

    fire("pointerdown", 100);
    fire("pointermove", 110);
    expect(readout(), "the readout must not follow the pointer during a drag").toBeNull();
    fire("pointermove", 200);
    expect(readout()).toBeNull();

    fire("pointerup", 200);
    fire("pointermove", 210);
    expect(readout(), "hover after release shows it again").not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("a press without a drag (under the 3 px threshold) keeps the readout", async () => {
    const { target, app, fire } = await mountOpen();
    fire("pointermove", 100);
    fire("pointerdown", 100);
    fire("pointermove", 101);
    expect(target.querySelector('[data-testid="spectral-hover"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });
});

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
  resetWaveformViewForTest();
  resetRecordForTest();
  resetSpectralForTest();
  unstubSize();
});

const FIXTURE = docDto();

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
        return recordStateDto({
          input_device: null,
          input_status: "not_selected",
          input_rate_hz: null,
        });
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

  it("sizes the canvas backing store in device pixels (H-12 HiDPI)", async () => {
    stubSize(800, 200);
    const dprDescriptor = Object.getOwnPropertyDescriptor(window, "devicePixelRatio");
    Object.defineProperty(window, "devicePixelRatio", { configurable: true, value: 2 });
    setupIpc([]);

    await openDocument("/home/user/take.wav");
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectralView, { target });
    // The draw loop runs off a real `requestAnimationFrame` (jsdom has no canvas, but its rAF is
    // a real ~16 ms timer, MEMORY.md) — wait for at least one tick.
    await new Promise((resolve) => setTimeout(resolve, 50));
    flushSync();

    const canvas = target.querySelector<HTMLCanvasElement>('[data-testid="spectral-canvas"]')!;
    expect(canvas.width).toBe(1_600); // 800 CSS px * dpr 2
    expect(canvas.height).toBe(400); // 200 CSS px * dpr 2

    unmount(app);
    target.remove();
    if (dprDescriptor) {
      Object.defineProperty(window, "devicePixelRatio", dprDescriptor);
    } else {
      Reflect.deleteProperty(window, "devicePixelRatio");
    }
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

    await emit(
      "record_state",
      recordStateDto({ armed: true, input_open: true, recording: true, monitoring: true }),
    );
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
