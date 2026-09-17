import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearActionHandlers } from "../shortcuts";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import { transportStateDto } from "../test/fixtures";
import Toolbar from "./Toolbar.svelte";

/** H-37 (SPEC-003 §2.1): the toolbar's Loop toggle button. */

let loopEnabled = false;
// H-80: mirrors the engine — with a document open, loop_range is the selection when it's long
// enough, else the whole document; `null` only reflects loop being off or no document at all.
let docLenSamples = 96_000;
let loopRange: [number, number] | null = null;
const calls: unknown[] = [];

function stateDto() {
  return transportStateDto({
    doc_len_samples: docLenSamples,
    can_play: docLenSamples > 0,
    loop_enabled: loopEnabled,
    loop_range: loopEnabled && docLenSamples > 0 ? (loopRange ?? [0, docLenSamples]) : null,
  });
}

beforeEach(() => {
  loopEnabled = false;
  docLenSamples = 96_000;
  loopRange = null;
  calls.length = 0;
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "transport_set_loop":
          calls.push((args as { enabled: boolean }).enabled);
          loopEnabled = (args as { enabled: boolean }).enabled;
          return stateDto();
        case "transport_get":
          return stateDto();
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
  resetWaveformViewForTest();
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("the Loop button", () => {
  it("toggles loop playback, shows its pressed state and the Ctrl+L shortcut", async () => {
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();

    const button = () => target.querySelector<HTMLButtonElement>('[data-testid="transport-loop"]')!;
    expect(button().getAttribute("aria-pressed")).toBe("false");
    expect(button().getAttribute("aria-label")).toBe("Loop playback");
    expect(button().getAttribute("aria-keyshortcuts")).toMatch(/Control\+L|Meta\+L/);

    button().click();
    await settle();
    expect(calls).toEqual([true]);
    expect(button().getAttribute("aria-pressed")).toBe("true");
    // H-80: loop on without a selection now loops the whole document — never inert with a
    // document open, so the plain label applies.
    expect(button().getAttribute("aria-label")).toBe("Loop playback");

    button().click();
    await settle();
    expect(calls).toEqual([true, false]);
    expect(button().getAttribute("aria-pressed")).toBe("false");

    unmount(app);
    target.remove();
    teardown();
  });

  it("the label is the plain one while a loop region is active", async () => {
    loopEnabled = true;
    loopRange = [1_000, 9_000];
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();
    const button = target.querySelector<HTMLButtonElement>('[data-testid="transport-loop"]')!;
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(button.getAttribute("aria-label")).toBe("Loop playback");
    unmount(app);
    target.remove();
    teardown();
  });

  // H-80: the only remaining inert case is no document at all — a whole-document loop is always
  // possible once one is open, so the toggle's on-state always means "this is actually looping".
  it("is inert only with no document open", async () => {
    loopEnabled = true;
    docLenSamples = 0;
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();
    const button = target.querySelector<HTMLButtonElement>('[data-testid="transport-loop"]')!;
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(button.getAttribute("aria-label")).toBe("Loop playback — open a file to loop");
    unmount(app);
    target.remove();
    teardown();
  });
});
