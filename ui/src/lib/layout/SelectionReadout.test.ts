import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import {
  resetSelectionForTest,
  selectionState,
  setSelectionFromResult,
} from "../state/selection.svelte";
import { resetWaveformViewForTest, setTimeRulerFormat } from "../state/waveformView.svelte";
import { docDto } from "../test/fixtures";
import SelectionReadout from "./SelectionReadout.svelte";

/**
 * T-206 (SPEC-006 §2.2/§2.9): selection start/end/length readouts, editable, with units — a pure
 * view-state edit (no IPC on commit), so these tests drive `state/selection.svelte.ts` directly
 * rather than mounting the whole waveform/pointer stack (that's `WaveformView.test.ts`'s job). A
 * document is opened (mocked `document_open`) so `len_samples` is real, since the fields clamp to
 * it; every test uses the `samples` time format for exact, easy-to-read assertions.
 */

async function openTestDoc(): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return docDto({ sample_rate_hz: 48_000, len_samples: 480_000 });
    }
    return null;
  });
  await openDocument("/home/user/take.wav");
}

afterEach(() => {
  clearMocks();
  resetSelectionForTest();
  resetWaveformViewForTest();
  resetDocumentStateForTest();
});

function mountReadout(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(SelectionReadout, { target });
  flushSync();
  return { target, app };
}

describe("SelectionReadout (T-206, SPEC-006 §2.2/§2.9)", () => {
  it("renders nothing with no selection", async () => {
    await openTestDoc();
    const { target, app } = mountReadout();
    expect(target.querySelector('[data-testid="selection-readout"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows Start/End/Length once a selection exists, following the current time format", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();

    expect(target.querySelector('[data-testid="selection-readout"]')).not.toBeNull();
    const start = target.querySelector<HTMLInputElement>('[data-testid="selection-start"]')!;
    const end = target.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!;
    const length = target.querySelector<HTMLInputElement>('[data-testid="selection-length"]')!;
    expect(start.value).toBe("48000");
    expect(end.value).toBe("96000");
    expect(length.value).toBe("48000");

    unmount(app);
    target.remove();
  });

  it("editing Start moves that boundary, keeping End fixed", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const start = target.querySelector<HTMLInputElement>('[data-testid="selection-start"]')!;

    start.value = "10000";
    start.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    start.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 10_000, endSample: 96_000 });

    unmount(app);
    target.remove();
  });

  it("editing End moves that boundary, keeping Start fixed", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const end = target.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!;

    end.value = "200000";
    end.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    end.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 200_000 });

    unmount(app);
    target.remove();
  });

  it("clamps End to len_samples", async () => {
    await openTestDoc(); // len_samples: 480_000
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const end = target.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!;

    end.value = "999999999";
    end.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    end.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 480_000 });

    unmount(app);
    target.remove();
  });

  it("editing Length keeps Start fixed and moves End", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const length = target.querySelector<HTMLInputElement>('[data-testid="selection-length"]')!;

    length.value = "1000";
    length.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    length.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 49_000 });

    unmount(app);
    target.remove();
  });

  it("an invalid edit (garbage text, or Start past End) is rejected", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const start = target.querySelector<HTMLInputElement>('[data-testid="selection-start"]')!;

    start.value = "not a number";
    start.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    start.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 96_000 });
    expect(start.value).toBe("48000"); // Enter clears the draft, so it re-shows the committed value

    start.value = "200000"; // past End
    start.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    start.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    flushSync();
    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 96_000 });

    unmount(app);
    target.remove();
  });

  it("Escape reverts an in-progress edit without committing", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    setTimeRulerFormat("samples");
    const { target, app } = mountReadout();
    const start = target.querySelector<HTMLInputElement>('[data-testid="selection-start"]')!;

    start.value = "10000";
    start.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    start.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();

    expect(selectionState().current).toEqual({ startSample: 48_000, endSample: 96_000 });
    expect(start.value).toBe("48000");

    unmount(app);
    target.remove();
  });

  it("disappears again once the selection is cleared", async () => {
    await openTestDoc();
    setSelectionFromResult([48_000, 96_000]);
    const { target, app } = mountReadout();
    expect(target.querySelector('[data-testid="selection-readout"]')).not.toBeNull();

    setSelectionFromResult(null);
    flushSync();
    expect(target.querySelector('[data-testid="selection-readout"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
