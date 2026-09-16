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
import { documentTimeFieldChars, formatDocumentTime, type TimeRulerFormat } from "../waveform/timeFormat";
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

async function openDocOfLength(sampleRateHz: number, lenSamples: number): Promise<void> {
  mockIPC((cmd) => {
    if (cmd === "document_open") {
      return docDto({ sample_rate_hz: sampleRateHz, len_samples: lenSamples });
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

/**
 * H-48 item 1 (owner report): the fields were a fixed 8ch, which clipped long values —
 * "00:00:19.(" instead of "00:00:19.123" — worse still at narrow window widths. Fields must now
 * size themselves (in `ch`, tabular figures) to the widest value the active format/document pair
 * can produce, for every time format, so nothing clips at 1280×720 or 2126×850.
 */
describe("SelectionReadout layout (H-48 item 1: fields size to content, never clip)", () => {
  const FORMATS: TimeRulerFormat[] = ["timecode", "samples", "seconds"];
  const testids = ["selection-start", "selection-end", "selection-length"] as const;

  function fieldWidthCh(el: HTMLInputElement): number {
    const match = /^([\d.]+)ch$/.exec(el.style.width);
    expect(match, `expected an inline "<n>ch" width, got "${el.style.width}"`).not.toBeNull();
    return Number(match![1]);
  }

  it("sizes every field to the document length's formatted width, for every time format", async () => {
    const rateHz = 48_000;
    const lenSamples = 172_800_000; // a real ~1-hour-class document
    await openDocOfLength(rateHz, lenSamples);
    setSelectionFromResult([0, lenSamples]); // Start=0, End=Length=the document's own length

    for (const format of FORMATS) {
      setTimeRulerFormat(format);
      const { target, app } = mountReadout();
      const expectedChars = documentTimeFieldChars(rateHz, lenSamples, format);

      for (const testid of testids) {
        const input = target.querySelector<HTMLInputElement>(`[data-testid="${testid}"]`)!;
        const chars = fieldWidthCh(input);
        expect(chars).toBe(expectedChars);
        // The load-bearing property: the field is never narrower than its own displayed text.
        expect(chars).toBeGreaterThanOrEqual(input.value.length);
      }

      unmount(app);
      target.remove();
    }
  });

  // The owner's exact repro, reproduced end to end through the component (not just the pure
  // function): a 10 s take in timecode format used to render "00:00:19.(" out of a fixed 8ch box.
  it("fixes the reported clip: a 10 s take in timecode format is never clipped to 8ch", async () => {
    const rateHz = 48_000;
    const lenSamples = 10 * rateHz;
    await openDocOfLength(rateHz, lenSamples);
    setTimeRulerFormat("timecode");
    setSelectionFromResult([0, lenSamples]);
    const { target, app } = mountReadout();

    const end = target.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!;
    expect(end.value).toBe(formatDocumentTime(lenSamples, rateHz, "timecode"));
    expect(end.value).toBe("00:00:10.000");
    expect(fieldWidthCh(end)).toBeGreaterThanOrEqual(12); // was a fixed 8ch before this ticket

    unmount(app);
    target.remove();
  });

  it("grows the field width for a long document in samples format, instead of a fixed guess", async () => {
    // Opening a document loads its own persisted time format (T-206), so the format must be set
    // *after* each open, not once up front.
    await openDocOfLength(48_000, 48_000); // 1 s
    setTimeRulerFormat("samples");
    setSelectionFromResult([0, 48_000]);
    const { target: shortTarget, app: shortApp } = mountReadout();
    const shortChars = fieldWidthCh(
      shortTarget.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!,
    );
    unmount(shortApp);
    shortTarget.remove();
    resetSelectionForTest();
    resetDocumentStateForTest();
    resetWaveformViewForTest();

    await openDocOfLength(48_000, 4_000_000_000); // a very long document
    setTimeRulerFormat("samples");
    setSelectionFromResult([0, 4_000_000_000]);
    const { target: longTarget, app: longApp } = mountReadout();
    const longChars = fieldWidthCh(
      longTarget.querySelector<HTMLInputElement>('[data-testid="selection-end"]')!,
    );
    unmount(longApp);
    longTarget.remove();

    expect(longChars).toBeGreaterThan(shortChars);
  });
});
